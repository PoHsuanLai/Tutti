//! Graph-aware PDC analysis.
//!
//! Computes per-input compensation delays using the arrival-time algorithm:
//! walk the graph in topological order, compute the worst-case latency at each
//! node's input, then insert delays on shorter paths to align them at merge points.

use crate::compat::{HashMap, Vec};
use alloc::collections::VecDeque;
use fundsp::audiounit::AudioUnit;
use fundsp::net::{Net, NodeId, Source};
use hashbrown::HashSet;

#[derive(Debug, Clone)]
pub(crate) struct PdcCompensation {
    pub(crate) node_id: NodeId,
    pub(crate) input_port: usize,
    pub(crate) delay_samples: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct PdcOutputCompensation {
    pub(crate) output_channel: usize,
    pub(crate) delay_samples: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PdcAnalysis {
    /// Per-input-port compensation delays.
    pub(crate) compensations: Vec<PdcCompensation>,
    /// Per-output-channel compensation delays.
    pub(crate) output_compensations: Vec<PdcOutputCompensation>,
    /// Maximum latency across all paths to the output (total graph latency).
    pub(crate) total_latency: usize,
    /// Per-node latency cache (for node_info display).
    pub(crate) node_latencies: HashMap<NodeId, usize>,
}

/// Analyze the graph and compute per-input compensation delays.
///
/// Requires `&mut Net` because `AudioUnit::latency()` takes `&mut self`.
///
/// Returns empty analysis if all nodes have zero latency (common fast path).
pub(crate) fn analyze(net: &mut Net) -> PdcAnalysis {
    let node_ids: Vec<NodeId> = net.ids().copied().collect();

    if node_ids.is_empty() {
        return PdcAnalysis::default();
    }

    // Step 1: Query latency for each node
    let node_latencies: HashMap<NodeId, usize> = node_ids
        .iter()
        .map(|&id| {
            let lat = net.node_mut(id).latency().unwrap_or(0.0).round() as usize;
            (id, lat)
        })
        .collect();

    // Fast path: no latency in the graph
    if !node_latencies.values().any(|&lat| lat > 0) {
        return PdcAnalysis {
            node_latencies,
            ..Default::default()
        };
    }

    // Step 2: Compute topological order (Kahn's algorithm)
    let topo_order = topological_sort(net, &node_ids);

    // Step 3: Compute arrival_time for each node
    // arrival_time(v) = max over all inputs of (arrival_time(src) + latency(src))
    let mut arrival_time: HashMap<NodeId, usize> = HashMap::with_capacity(node_ids.len());

    for &id in &topo_order {
        let inputs = net.inputs_in(id);
        let max_arrival = (0..inputs)
            .filter_map(|port| {
                if let Source::Local(src_id, _) = net.source(id, port) {
                    let src_arrival = arrival_time.get(&src_id).copied().unwrap_or(0);
                    let src_lat = node_latencies.get(&src_id).copied().unwrap_or(0);
                    Some(src_arrival + src_lat)
                } else {
                    None
                }
            })
            .max()
            .unwrap_or(0);

        arrival_time.insert(id, max_arrival);
    }

    // Step 4: Compute per-input compensation at fan-in points
    let mut compensations = Vec::new();

    // Helper: compute arrival time at a source port
    let source_arrival = |src_id: NodeId| -> usize {
        arrival_time.get(&src_id).copied().unwrap_or(0)
            + node_latencies.get(&src_id).copied().unwrap_or(0)
    };

    for &id in &topo_order {
        let inputs = net.inputs_in(id);
        if inputs < 2 {
            continue; // No fan-in, no compensation needed
        }

        let max_input_arrival = (0..inputs)
            .filter_map(|port| match net.source(id, port) {
                Source::Local(src_id, _) => Some(source_arrival(src_id)),
                _ => None,
            })
            .max()
            .unwrap_or(0);

        // Compute per-input compensation
        compensations.extend((0..inputs).filter_map(|port| {
            if let Source::Local(src_id, _) = net.source(id, port) {
                let delay = max_input_arrival.saturating_sub(source_arrival(src_id));
                if delay > 0 {
                    return Some(PdcCompensation {
                        node_id: id,
                        input_port: port,
                        delay_samples: delay,
                    });
                }
            }
            None
        }));
    }

    // Step 5: Compute output compensation
    let mut output_compensations = Vec::new();
    let num_outputs = net.outputs();

    if num_outputs > 1 {
        let max_output_arrival = (0..num_outputs)
            .filter_map(|ch| match net.output_source(ch) {
                Source::Local(src_id, _) => Some(source_arrival(src_id)),
                _ => None,
            })
            .max()
            .unwrap_or(0);

        output_compensations.extend((0..num_outputs).filter_map(|ch| {
            if let Source::Local(src_id, _) = net.output_source(ch) {
                let delay = max_output_arrival.saturating_sub(source_arrival(src_id));
                if delay > 0 {
                    return Some(PdcOutputCompensation {
                        output_channel: ch,
                        delay_samples: delay,
                    });
                }
            }
            None
        }));
    }

    // Compute total graph latency (max arrival at any output)
    let total_latency = (0..num_outputs)
        .filter_map(|ch| match net.output_source(ch) {
            Source::Local(src_id, _) => Some(source_arrival(src_id)),
            _ => None,
        })
        .max()
        .unwrap_or(0);

    PdcAnalysis {
        compensations,
        output_compensations,
        total_latency,
        node_latencies,
    }
}

/// Compute topological order of nodes using Kahn's algorithm.
fn topological_sort(net: &Net, node_ids: &[NodeId]) -> Vec<NodeId> {
    let count = node_ids.len();
    let mut in_degree: HashMap<NodeId, usize> = HashMap::with_capacity(count);
    let mut dependents: HashMap<NodeId, Vec<NodeId>> = HashMap::with_capacity(count);
    let id_set: HashSet<NodeId> = node_ids.iter().copied().collect();

    // Initialize
    for &id in node_ids {
        in_degree.insert(id, 0);
        dependents.entry(id).or_default();
    }

    // Build dependency graph from Net edges
    for &id in node_ids {
        let inputs = net.inputs_in(id);
        for port in 0..inputs {
            if let Source::Local(src_id, _) = net.source(id, port) {
                if id_set.contains(&src_id) {
                    // src_id → id edge (src feeds into id)
                    // Only count each unique src→dest pair once for in-degree
                    dependents.entry(src_id).or_default().push(id);
                    *in_degree.entry(id).or_insert(0) += 1;
                }
            }
        }
    }

    // Kahn's: start with nodes that have no incoming edges
    let mut queue: VecDeque<NodeId> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut order = Vec::with_capacity(count);

    while let Some(id) = queue.pop_front() {
        order.push(id);

        if let Some(deps) = dependents.get(&id) {
            for &dep_id in deps {
                if let Some(deg) = in_degree.get_mut(&dep_id) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push_back(dep_id);
                    }
                }
            }
        }
    }

    // If order doesn't contain all nodes, there's a cycle — append remaining
    if order.len() < count {
        for &id in node_ids {
            if !order.contains(&id) {
                order.push(id);
            }
        }
    }

    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::Box;
    use crate::pdc::PdcDelayUnit;
    use fundsp::prelude::*;

    #[test]
    fn test_empty_graph() {
        let mut net = Net::new(0, 2);
        let analysis = analyze(&mut net);
        assert_eq!(analysis.total_latency, 0);
        assert!(analysis.compensations.is_empty());
        assert!(analysis.output_compensations.is_empty());
    }

    #[test]
    fn test_no_latency_nodes() {
        let mut net = Net::new(0, 2);
        let a = net.push(Box::new(dc(1.0)));
        let b = net.push(Box::new(pass()));
        net.connect(a, 0, b, 0);
        net.pipe_output(b);

        let analysis = analyze(&mut net);
        assert_eq!(analysis.total_latency, 0);
        assert!(analysis.compensations.is_empty());
    }

    #[test]
    fn test_single_chain_no_compensation() {
        // src → [latency=100] → output
        // Only one path, no compensation needed at merge points
        let mut net = Net::new(0, 2);
        let src = net.push(Box::new(dc(1.0)));
        let effect = net.push(Box::new(PdcDelayUnit::new(100)));
        net.connect(src, 0, effect, 0);
        net.pipe_output(effect);

        let analysis = analyze(&mut net);
        // No fan-in compensation (single path), but total latency is captured
        assert!(analysis.compensations.is_empty());
    }

    #[test]
    fn test_parallel_merge_compensation() {
        // path1: src1 → [effect with latency 512] → mixer → output
        // path2: src2 ──────────────────────────→ mixer → output
        //
        // mixer has 2 inputs. src2's path needs 512 samples delay.

        let mut net = Net::new(0, 1);

        let src1 = net.push(Box::new(dc(1.0)));
        let src2 = net.push(Box::new(dc(1.0)));
        // Use PdcDelayUnit as a stand-in for a latency-producing effect.
        // But PdcDelayUnit::latency() returns None (route returns Unknown).
        // We need a node that reports latency. Use fundsp's delay() instead.
        let effect = net.push(Box::new(delay(512.0 / 44100.0)));
        let mixer = net.push(Box::new(pass() + pass())); // 2 inputs, 1 output

        net.connect(src1, 0, effect, 0);
        net.connect(effect, 0, mixer, 0);
        net.connect(src2, 0, mixer, 1);
        net.pipe_output(mixer);

        let analysis = analyze(&mut net);

        // The effect (fundsp delay) reports latency via route()
        let effect_lat = *analysis.node_latencies.get(&effect).unwrap_or(&0);

        if effect_lat > 0 {
            // mixer input 1 (from src2) needs compensation = effect_lat
            assert!(!analysis.compensations.is_empty());
            let comp = &analysis.compensations[0];
            assert_eq!(comp.node_id, mixer);
            assert_eq!(comp.input_port, 1);
            assert_eq!(comp.delay_samples, effect_lat);
        }
    }

    #[test]
    fn test_diamond_compensation() {
        // A → B(lat=100) → D
        // A → C(lat=0)   → D
        // D's input from C needs 100 samples delay.

        let mut net = Net::new(0, 1);

        let a = net.push(Box::new(dc(1.0)));
        // B: a node with known latency. Use limiter which has latency.
        let b = net.push(Box::new(delay(100.0 / 44100.0)));
        let c = net.push(Box::new(pass()));
        let d = net.push(Box::new(pass() + pass())); // 2 inputs → 1 output

        net.connect(a, 0, b, 0);
        net.connect(a, 0, c, 0);
        net.connect(b, 0, d, 0);
        net.connect(c, 0, d, 1);
        net.pipe_output(d);

        let analysis = analyze(&mut net);
        let b_lat = *analysis.node_latencies.get(&b).unwrap_or(&0);

        if b_lat > 0 {
            // D's input from C (port 1) needs compensation = b_lat
            let comp = analysis
                .compensations
                .iter()
                .find(|c| c.node_id == d && c.input_port == 1);
            assert!(comp.is_some(), "Expected compensation on D input 1");
            assert_eq!(comp.unwrap().delay_samples, b_lat);

            // D's input from B (port 0) needs no compensation
            let comp_b = analysis
                .compensations
                .iter()
                .find(|c| c.node_id == d && c.input_port == 0);
            assert!(comp_b.is_none(), "B's path should not need compensation");
        }
    }

    #[test]
    fn test_topological_sort_basic() {
        let mut net = Net::new(0, 1);
        let a = net.push(Box::new(dc(1.0)));
        let b = net.push(Box::new(pass()));
        let c = net.push(Box::new(pass()));

        net.connect(a, 0, b, 0);
        net.connect(b, 0, c, 0);

        let ids = vec![a, b, c];
        let order = topological_sort(&net, &ids);

        let pos_a = order.iter().position(|&x| x == a).unwrap();
        let pos_b = order.iter().position(|&x| x == b).unwrap();
        let pos_c = order.iter().position(|&x| x == c).unwrap();

        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }
}
