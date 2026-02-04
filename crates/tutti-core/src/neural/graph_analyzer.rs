//! Graph analysis for neural batching optimization.

use super::metadata::{NeuralModelId, NeuralNodeManager};
use crate::compat::{HashMap, Vec};
use alloc::collections::VecDeque;
use fundsp::net::{Net, NodeId, Source};
use hashbrown::HashSet;

/// Result of graph analysis for neural batching.
#[derive(Debug, Clone, Default)]
pub struct BatchingStrategy {
    pub model_batches: HashMap<NeuralModelId, Vec<NodeId>>,
    pub execution_order: HashMap<NodeId, usize>,
    pub total_neural_nodes: usize,
}

impl BatchingStrategy {
    /// Check if strategy is empty (no neural nodes).
    pub fn is_empty(&self) -> bool {
        self.total_neural_nodes == 0
    }

    /// Batch efficiency ratio (total nodes / GPU calls needed).
    pub fn batch_efficiency(&self) -> f32 {
        if self.model_batches.is_empty() {
            return 0.0;
        }
        let total_nodes: usize = self.model_batches.values().map(|v| v.len()).sum();
        let num_batches = self.model_batches.len();
        total_nodes as f32 / num_batches as f32
    }

    /// Get count of unique models.
    pub fn model_count(&self) -> usize {
        self.model_batches.len()
    }
}

/// Analyzes the Net graph to compute optimal neural batching.
pub(crate) struct GraphAnalyzer<'a> {
    net: &'a Net,
    manager: &'a NeuralNodeManager,
}

impl<'a> GraphAnalyzer<'a> {
    /// Creates a new graph analyzer.
    pub fn new(net: &'a Net, manager: &'a NeuralNodeManager) -> Self {
        Self { net, manager }
    }

    /// Analyze the graph and compute batching strategy.
    pub fn analyze(&self) -> BatchingStrategy {
        // 1. Group neural nodes by model_id
        let model_batches = self.manager.group_by_model();

        let total_neural_nodes = model_batches.values().map(|v| v.len()).sum();

        if total_neural_nodes == 0 {
            return BatchingStrategy::default();
        }

        // 2. Compute execution order (topological sort)
        let execution_order = self.compute_execution_order();

        BatchingStrategy {
            model_batches,
            execution_order,
            total_neural_nodes,
        }
    }

    /// Compute topological execution order for neural nodes.
    fn compute_execution_order(&self) -> HashMap<NodeId, usize> {
        let neural_nodes: HashSet<NodeId> = self.manager.all_nodes().into_iter().collect();
        let node_count = neural_nodes.len();

        // Pre-size maps to avoid rehashing during graph construction
        let mut order = HashMap::with_capacity(node_count);
        let mut deps: HashMap<NodeId, Vec<NodeId>> = HashMap::with_capacity(node_count);
        let mut indegree: HashMap<NodeId, usize> = HashMap::with_capacity(node_count);

        for &node_id in &neural_nodes {
            deps.entry(node_id).or_default();
            indegree.entry(node_id).or_insert(0);
        }

        // Find dependencies between neural nodes
        // A depends on B if there's any path from B to A in the graph
        for &node_id in &neural_nodes {
            let upstream = self.find_upstream_neural_nodes(node_id, &neural_nodes);
            for &upstream_id in &upstream {
                deps.entry(upstream_id).or_default().push(node_id);
                *indegree.entry(node_id).or_insert(0) += 1;
            }
        }

        // Kahn's algorithm for topological sort
        let mut queue: VecDeque<NodeId> = indegree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut index = 0;
        while let Some(node_id) = queue.pop_front() {
            order.insert(node_id, index);
            index += 1;

            if let Some(dependents) = deps.get(&node_id) {
                for &dependent_id in dependents {
                    if let Some(deg) = indegree.get_mut(&dependent_id) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(dependent_id);
                        }
                    }
                }
            }
        }

        order
    }

    /// Find all upstream neural nodes that feed into `target`.
    fn find_upstream_neural_nodes(
        &self,
        target: NodeId,
        neural_nodes: &HashSet<NodeId>,
    ) -> Vec<NodeId> {
        // Pre-size for typical upstream depth (most nodes have <8 upstream neural nodes)
        let estimated_size = neural_nodes.len().min(8);
        let mut upstream_neural = Vec::with_capacity(estimated_size);
        let mut visited = HashSet::with_capacity(estimated_size);
        let mut queue = VecDeque::with_capacity(estimated_size);

        // Start with immediate inputs
        if self.net.contains(target) {
            let inputs = self.net.inputs_in(target);
            for i in 0..inputs {
                if let Source::Local(src_id, _) = self.net.source(target, i) {
                    queue.push_back(src_id);
                }
            }
        }

        while let Some(current_id) = queue.pop_front() {
            if !visited.insert(current_id) {
                continue;
            }

            // If this is a neural node, record it
            if neural_nodes.contains(&current_id) && current_id != target {
                upstream_neural.push(current_id);
            }

            // Continue traversing upstream
            if self.net.contains(current_id) {
                let inputs = self.net.inputs_in(current_id);
                for i in 0..inputs {
                    if let Source::Local(src_id, _) = self.net.source(current_id, i) {
                        if !visited.contains(&src_id) {
                            queue.push_back(src_id);
                        }
                    }
                }
            }
        }

        upstream_neural
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::Box;
    use crate::neural::metadata::NeuralModelId;
    use fundsp::prelude::*;

    fn create_test_net_and_manager() -> (Net, NeuralNodeManager) {
        let net = Net::new(0, 2);
        let manager = NeuralNodeManager::new();
        (net, manager)
    }

    #[test]
    fn test_empty_graph() {
        let (net, manager) = create_test_net_and_manager();
        let analyzer = GraphAnalyzer::new(&net, &manager);
        let strategy = analyzer.analyze();

        assert!(strategy.is_empty());
        assert_eq!(strategy.total_neural_nodes, 0);
        assert_eq!(strategy.batch_efficiency(), 0.0);
    }

    #[test]
    fn test_single_neural_node() {
        let (mut net, manager) = create_test_net_and_manager();

        let model_id = NeuralModelId::from_raw(1);
        let node_id = net.push(Box::new(dc(0.0f32)));
        manager.register(node_id, model_id);

        let analyzer = GraphAnalyzer::new(&net, &manager);
        let strategy = analyzer.analyze();

        assert!(!strategy.is_empty());
        assert_eq!(strategy.total_neural_nodes, 1);
        assert_eq!(strategy.model_count(), 1);
        assert_eq!(strategy.model_batches[&model_id].len(), 1);
    }

    #[test]
    fn test_same_model_grouping() {
        let (mut net, manager) = create_test_net_and_manager();

        let model_a = NeuralModelId::from_raw(1);

        // Add 4 nodes with the same model
        let node1 = net.push(Box::new(dc(0.0f32)));
        let node2 = net.push(Box::new(dc(0.0f32)));
        let node3 = net.push(Box::new(dc(0.0f32)));
        let node4 = net.push(Box::new(dc(0.0f32)));

        manager.register(node1, model_a);
        manager.register(node2, model_a);
        manager.register(node3, model_a);
        manager.register(node4, model_a);

        let analyzer = GraphAnalyzer::new(&net, &manager);
        let strategy = analyzer.analyze();

        assert_eq!(strategy.total_neural_nodes, 4);
        assert_eq!(strategy.model_count(), 1);
        assert_eq!(strategy.model_batches[&model_a].len(), 4);
        assert_eq!(strategy.batch_efficiency(), 4.0); // 4 nodes / 1 batch
    }

    #[test]
    fn test_multiple_models() {
        let (mut net, manager) = create_test_net_and_manager();

        let model_a = NeuralModelId::from_raw(1);
        let model_b = NeuralModelId::from_raw(2);

        // 2 nodes with model_a, 2 with model_b
        let node1 = net.push(Box::new(dc(0.0f32)));
        let node2 = net.push(Box::new(dc(0.0f32)));
        let node3 = net.push(Box::new(dc(0.0f32)));
        let node4 = net.push(Box::new(dc(0.0f32)));

        manager.register(node1, model_a);
        manager.register(node2, model_a);
        manager.register(node3, model_b);
        manager.register(node4, model_b);

        let analyzer = GraphAnalyzer::new(&net, &manager);
        let strategy = analyzer.analyze();

        assert_eq!(strategy.total_neural_nodes, 4);
        assert_eq!(strategy.model_count(), 2);
        assert_eq!(strategy.model_batches[&model_a].len(), 2);
        assert_eq!(strategy.model_batches[&model_b].len(), 2);
        assert_eq!(strategy.batch_efficiency(), 2.0); // 4 nodes / 2 batches
    }

    #[test]
    fn test_execution_order_dependencies() {
        let (mut net, manager) = create_test_net_and_manager();

        let model_a = NeuralModelId::from_raw(1);

        // neural1 → neural2 (neural1 must execute before neural2)
        let neural1 = net.push(Box::new(pass()));
        let neural2 = net.push(Box::new(pass()));

        net.connect(neural1, 0, neural2, 0);

        manager.register(neural1, model_a);
        manager.register(neural2, model_a);

        let analyzer = GraphAnalyzer::new(&net, &manager);
        let strategy = analyzer.analyze();

        // neural1 should have lower execution index than neural2
        let order1 = strategy.execution_order[&neural1];
        let order2 = strategy.execution_order[&neural2];
        assert!(
            order1 < order2,
            "neural1 (order {}) should execute before neural2 (order {})",
            order1,
            order2
        );
    }

    #[test]
    fn test_complex_graph() {
        let (mut net, manager) = create_test_net_and_manager();

        let model_ddsp = NeuralModelId::from_raw(1);
        let model_amp = NeuralModelId::from_raw(2);

        // Graph topology: ddsp1 and ddsp2 both feed into mixer, then to amp_sim

        let ddsp1 = net.push(Box::new(dc((0.5f32, 0.5f32)))); // stereo
        let ddsp2 = net.push(Box::new(dc((0.5f32, 0.5f32))));
        let mixer = net.push(Box::new((pass() | pass()) + (pass() | pass())));
        let amp_sim = net.push(Box::new(pass() | pass()));

        // Connect
        net.connect(ddsp1, 0, mixer, 0);
        net.connect(ddsp1, 1, mixer, 1);
        net.connect(ddsp2, 0, mixer, 2);
        net.connect(ddsp2, 1, mixer, 3);
        net.connect(mixer, 0, amp_sim, 0);
        net.connect(mixer, 1, amp_sim, 1);

        manager.register(ddsp1, model_ddsp);
        manager.register(ddsp2, model_ddsp);
        manager.register(amp_sim, model_amp);

        let analyzer = GraphAnalyzer::new(&net, &manager);
        let strategy = analyzer.analyze();

        // Verify grouping
        assert_eq!(strategy.total_neural_nodes, 3);
        assert_eq!(strategy.model_count(), 2);
        assert_eq!(strategy.model_batches[&model_ddsp].len(), 2);
        assert_eq!(strategy.model_batches[&model_amp].len(), 1);

        // Execution order: ddsp1 and ddsp2 before amp_sim
        let order_ddsp1 = strategy.execution_order[&ddsp1];
        let order_ddsp2 = strategy.execution_order[&ddsp2];
        let order_amp = strategy.execution_order[&amp_sim];

        assert!(order_ddsp1 < order_amp);
        assert!(order_ddsp2 < order_amp);
    }
}
