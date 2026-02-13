//! Metadata registry for neural nodes.

use crate::compat::{Arc, HashMap, Vec};
use core::sync::atomic::{AtomicU64, Ordering};
use dashmap::DashMap;
use fundsp::net::NodeId;
use serde::{Deserialize, Serialize};

static MODEL_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeuralModelId(pub u64);

impl NeuralModelId {
    pub fn new() -> Self {
        Self(MODEL_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    pub fn from_raw(id: u64) -> Self {
        Self(id)
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Default for NeuralModelId {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Display for NeuralModelId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "NeuralModel({})", self.0)
    }
}

/// Registry for neural nodes in the audio graph.
///
/// Maps NodeId → NeuralModelId. This is all the graph analyzer needs
/// to group nodes by model for batched inference.
pub struct NeuralNodeManager {
    nodes: DashMap<NodeId, NeuralModelId>,
}

impl NeuralNodeManager {
    pub fn new() -> Self {
        Self {
            nodes: DashMap::new(),
        }
    }

    pub fn register(&self, node_id: NodeId, model_id: NeuralModelId) {
        self.nodes.insert(node_id, model_id);
    }

    pub(crate) fn unregister(&self, node_id: &NodeId) -> Option<NeuralModelId> {
        self.nodes.remove(node_id).map(|(_, v)| v)
    }

    pub fn is_neural(&self, node_id: &NodeId) -> bool {
        self.nodes.contains_key(node_id)
    }

    pub(crate) fn group_by_model(&self) -> HashMap<NeuralModelId, Vec<NodeId>> {
        let mut groups: HashMap<NeuralModelId, Vec<NodeId>> = HashMap::new();
        for entry in self.nodes.iter() {
            groups.entry(*entry.value()).or_default().push(*entry.key());
        }
        groups
    }

    pub(crate) fn all_nodes(&self) -> Vec<NodeId> {
        self.nodes.iter().map(|e| *e.key()).collect()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn clear(&self) {
        self.nodes.clear();
    }
}

impl Default for NeuralNodeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for NeuralNodeManager {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NeuralNodeManager")
            .field("node_count", &self.nodes.len())
            .finish()
    }
}

pub type SharedNeuralNodeManager = Arc<NeuralNodeManager>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::Box;

    #[test]
    fn test_model_id_generation() {
        let id1 = NeuralModelId::new();
        let id2 = NeuralModelId::new();
        assert_ne!(id1.as_u64(), id2.as_u64());
    }

    #[test]
    fn test_model_id_from_raw() {
        let id = NeuralModelId::from_raw(12345);
        assert_eq!(id.as_u64(), 12345);
    }

    #[test]
    fn test_registry_basic() {
        let registry = NeuralNodeManager::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_registry_register_unregister() {
        use fundsp::net::Net;
        use fundsp::prelude::*;

        let registry = NeuralNodeManager::new();

        let mut net = Net::new(0, 2);
        let node_id = net.push(Box::new(dc(0.0f32)));

        let model_id = NeuralModelId::new();
        registry.register(node_id, model_id);

        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
        assert!(registry.is_neural(&node_id));

        let removed = registry.unregister(&node_id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap(), model_id);
        assert!(registry.is_empty());
        assert!(!registry.is_neural(&node_id));
    }

    #[test]
    fn test_group_by_model() {
        use fundsp::net::Net;
        use fundsp::prelude::*;

        let registry = NeuralNodeManager::new();
        let mut net = Net::new(0, 2);

        let model_a = NeuralModelId::from_raw(1);
        let model_b = NeuralModelId::from_raw(2);

        let node1 = net.push(Box::new(dc(0.0f32)));
        let node2 = net.push(Box::new(dc(0.0f32)));
        let node3 = net.push(Box::new(dc(0.0f32)));

        registry.register(node1, model_a);
        registry.register(node2, model_a);
        registry.register(node3, model_b);

        let groups = registry.group_by_model();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[&model_a].len(), 2);
        assert_eq!(groups[&model_b].len(), 1);
    }
}
