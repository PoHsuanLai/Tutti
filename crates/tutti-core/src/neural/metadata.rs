//! Metadata registry for neural nodes.

use crate::compat::{Arc, HashMap, Vec};
use core::sync::atomic::{AtomicU64, Ordering};
use dashmap::DashMap;
use fundsp::net::NodeId;
use serde::{Deserialize, Serialize};

/// Counter for generating unique model IDs
static MODEL_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Unique identifier for a neural model.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeuralModelId(pub u64);

impl NeuralModelId {
    /// Creates a new unique model ID.
    pub fn new() -> Self {
        Self(MODEL_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Creates a model ID from a raw u64 value.
    pub fn from_raw(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw u64 value.
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

/// Metadata for a neural node in the graph (used in tests for GraphAnalyzer).
#[derive(Debug, Clone)]
#[allow(dead_code)] // Used in tests only
pub(crate) struct NeuralNodeInfo {
    pub model_id: NeuralModelId,
    pub buffer_size: usize,
    pub sample_rate: f32,
    pub is_synth: bool,
    pub latency_samples: usize,
}

#[allow(dead_code)] // Used in tests only
impl NeuralNodeInfo {
    /// Creates metadata for a neural synthesizer.
    pub fn synth(model_id: NeuralModelId, buffer_size: usize, sample_rate: f32) -> Self {
        Self {
            model_id,
            buffer_size,
            sample_rate,
            is_synth: true,
            latency_samples: 0,
        }
    }

    /// Creates metadata for a neural audio effect.
    pub fn effect(model_id: NeuralModelId, buffer_size: usize, sample_rate: f32) -> Self {
        Self {
            model_id,
            buffer_size,
            sample_rate,
            is_synth: false,
            latency_samples: 0,
        }
    }

    /// Sets the processing latency in samples (for PDC).
    pub fn with_latency(mut self, samples: usize) -> Self {
        self.latency_samples = samples;
        self
    }
}

/// Registry for neural node metadata.
pub struct NeuralNodeManager {
    nodes: DashMap<NodeId, NeuralNodeInfo>,
}

impl NeuralNodeManager {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            nodes: DashMap::new(),
        }
    }

    /// Registers a neural node with its metadata (used in tests only).
    #[allow(dead_code)]
    pub(crate) fn register(&self, node_id: NodeId, info: NeuralNodeInfo) {
        tracing::debug!(
            "Registering neural node {:?} with model {}",
            node_id,
            info.model_id
        );
        self.nodes.insert(node_id, info);
    }

    /// Unregisters a neural node.
    pub(crate) fn unregister(&self, node_id: &NodeId) -> Option<NeuralNodeInfo> {
        let result = self.nodes.remove(node_id).map(|(_, v)| v);
        if result.is_some() {
            tracing::debug!("Unregistered neural node {:?}", node_id);
        }
        result
    }

    /// Checks if a node is registered as neural.
    pub fn is_neural(&self, node_id: &NodeId) -> bool {
        self.nodes.contains_key(node_id)
    }

    /// Retrieves metadata for a neural node (used in tests only).
    #[allow(dead_code)]
    pub(crate) fn get(&self, node_id: &NodeId) -> Option<NeuralNodeInfo> {
        self.nodes.get(node_id).map(|r| r.clone())
    }

    /// Get all neural nodes grouped by model_id.
    pub(crate) fn group_by_model(&self) -> HashMap<NeuralModelId, Vec<NodeId>> {
        let mut groups: HashMap<NeuralModelId, Vec<NodeId>> = HashMap::new();
        for entry in self.nodes.iter() {
            groups.entry(entry.model_id).or_default().push(*entry.key());
        }
        groups
    }

    /// Get all registered node IDs.
    pub(crate) fn all_nodes(&self) -> Vec<NodeId> {
        self.nodes.iter().map(|e| *e.key()).collect()
    }

    /// Get count of registered neural nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if registry is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Clear all registrations.
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

/// Arc-wrapped manager for shared ownership
pub type SharedNeuralNodeManager = Arc<NeuralNodeManager>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::Box;
    #[test]
    fn test_model_id_generation() {
        let id1 = NeuralModelId::new();
        let id2 = NeuralModelId::new();
        // Each ID should be unique
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

        // Create a real NodeId
        let mut net = Net::new(0, 2);
        let node_id = net.push(Box::new(dc(0.0f32)));

        // Register
        let model_id = NeuralModelId::new();
        let info = NeuralNodeInfo::synth(model_id, 512, 44100.0);
        registry.register(node_id, info.clone());

        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
        assert!(registry.is_neural(&node_id));

        let retrieved = registry.get(&node_id).unwrap();
        assert_eq!(retrieved.model_id, model_id);
        assert!(retrieved.is_synth);

        // Unregister
        let removed = registry.unregister(&node_id);
        assert!(removed.is_some());
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

        // Add 3 nodes: 2 with model_a, 1 with model_b
        let node1 = net.push(Box::new(dc(0.0f32)));
        let node2 = net.push(Box::new(dc(0.0f32)));
        let node3 = net.push(Box::new(dc(0.0f32)));

        registry.register(node1, NeuralNodeInfo::synth(model_a, 512, 44100.0));
        registry.register(node2, NeuralNodeInfo::synth(model_a, 512, 44100.0));
        registry.register(node3, NeuralNodeInfo::effect(model_b, 512, 44100.0));

        let groups = registry.group_by_model();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[&model_a].len(), 2);
        assert_eq!(groups[&model_b].len(), 1);
    }

    #[test]
    fn test_neural_node_info_builders() {
        let model_id = NeuralModelId::new();

        let synth_info = NeuralNodeInfo::synth(model_id, 512, 44100.0);
        assert!(synth_info.is_synth);
        assert_eq!(synth_info.buffer_size, 512);
        assert_eq!(synth_info.sample_rate, 44100.0);
        assert_eq!(synth_info.latency_samples, 0);

        let effect_info = NeuralNodeInfo::effect(model_id, 1024, 48000.0).with_latency(256);
        assert!(!effect_info.is_synth);
        assert_eq!(effect_info.buffer_size, 1024);
        assert_eq!(effect_info.sample_rate, 48000.0);
        assert_eq!(effect_info.latency_samples, 256);
    }
}
