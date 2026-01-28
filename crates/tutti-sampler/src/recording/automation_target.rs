//! Automation target identifiers.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Automation target identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutomationTarget {
    /// A parameter on a specific audio node in the FunDSP graph
    NodeParam {
        /// The FunDSP node ID (from `net.add()`)
        node_id: u64,
        /// Parameter index within the node (0-based)
        param_index: usize,
        /// Optional human-readable parameter name
        param_name: Option<String>,
    },

    /// Global master output volume (0.0 to 1.0+)
    MasterVolume,

    /// Global master stereo pan (-1.0 left to +1.0 right)
    MasterPan,

    /// Transport tempo in BPM (e.g., 60.0 to 300.0)
    Tempo,

    /// User-defined custom target with string identifier
    ///
    /// Use this for application-specific automation targets that
    /// don't fit the other categories.
    Custom(String),
}

impl AutomationTarget {
    /// Create a node parameter target with only the essential fields
    pub fn node_param(node_id: u64, param_index: usize) -> Self {
        Self::NodeParam {
            node_id,
            param_index,
            param_name: None,
        }
    }

    /// Create a node parameter target with a human-readable name
    pub fn node_param_named(node_id: u64, param_index: usize, name: impl Into<String>) -> Self {
        Self::NodeParam {
            node_id,
            param_index,
            param_name: Some(name.into()),
        }
    }

    /// Create a custom target
    pub fn custom(id: impl Into<String>) -> Self {
        Self::Custom(id.into())
    }

    /// Get a unique string key for this target (useful for HashMap keys)
    pub fn key(&self) -> String {
        match self {
            Self::NodeParam {
                node_id,
                param_index,
                ..
            } => format!("node:{node_id}:{param_index}"),
            Self::MasterVolume => "master:volume".to_string(),
            Self::MasterPan => "master:pan".to_string(),
            Self::Tempo => "transport:tempo".to_string(),
            Self::Custom(id) => format!("custom:{id}"),
        }
    }

    /// Get a human-readable display name for this target
    pub fn display_name(&self) -> String {
        match self {
            Self::NodeParam {
                node_id,
                param_index,
                param_name,
            } => {
                if let Some(name) = param_name {
                    format!("Node {node_id}: {name}")
                } else {
                    format!("Node {node_id}: Param {param_index}")
                }
            }
            Self::MasterVolume => "Master Volume".to_string(),
            Self::MasterPan => "Master Pan".to_string(),
            Self::Tempo => "Tempo".to_string(),
            Self::Custom(id) => id.clone(),
        }
    }

    /// Returns true if this target is a node parameter
    pub fn is_node_param(&self) -> bool {
        matches!(self, Self::NodeParam { .. })
    }

    /// Returns true if this target is a master control
    pub fn is_master(&self) -> bool {
        matches!(self, Self::MasterVolume | Self::MasterPan)
    }

    /// Returns true if this target is the transport tempo
    pub fn is_tempo(&self) -> bool {
        matches!(self, Self::Tempo)
    }

    /// Get the node ID if this is a NodeParam target
    pub fn node_id(&self) -> Option<u64> {
        match self {
            Self::NodeParam { node_id, .. } => Some(*node_id),
            _ => None,
        }
    }

    /// Get the default value range for this target type
    ///
    /// Returns (min, max, default) tuple
    pub fn default_range(&self) -> (f32, f32, f32) {
        match self {
            Self::MasterVolume => (0.0, 2.0, 1.0), // Allow slight boost
            Self::MasterPan => (-1.0, 1.0, 0.0),   // Full left to right
            Self::Tempo => (20.0, 300.0, 120.0),   // BPM range
            Self::NodeParam { .. } => (0.0, 1.0, 0.5), // Normalized default
            Self::Custom(_) => (0.0, 1.0, 0.5),    // Normalized default
        }
    }
}

impl fmt::Display for AutomationTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

impl Default for AutomationTarget {
    fn default() -> Self {
        Self::Custom("default".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_param_creation() {
        let target = AutomationTarget::node_param(42, 0);
        assert!(target.is_node_param());
        assert_eq!(target.node_id(), Some(42));
        assert_eq!(target.key(), "node:42:0");
    }

    #[test]
    fn test_node_param_named() {
        let target = AutomationTarget::node_param_named(42, 0, "cutoff");
        assert_eq!(target.display_name(), "Node 42: cutoff");
    }

    #[test]
    fn test_master_targets() {
        assert!(AutomationTarget::MasterVolume.is_master());
        assert!(AutomationTarget::MasterPan.is_master());
        assert!(!AutomationTarget::Tempo.is_master());
    }

    #[test]
    fn test_unique_keys() {
        let targets = [
            AutomationTarget::node_param(0, 0),
            AutomationTarget::node_param(0, 1),
            AutomationTarget::node_param(1, 0),
            AutomationTarget::MasterVolume,
            AutomationTarget::MasterPan,
            AutomationTarget::Tempo,
            AutomationTarget::custom("my_target"),
        ];

        let keys: std::collections::HashSet<_> = targets.iter().map(|t| t.key()).collect();
        assert_eq!(keys.len(), targets.len(), "All keys should be unique");
    }

    #[test]
    fn test_default_ranges() {
        let (min, max, default) = AutomationTarget::MasterVolume.default_range();
        assert_eq!(min, 0.0);
        assert_eq!(max, 2.0);
        assert_eq!(default, 1.0);

        let (min, max, default) = AutomationTarget::Tempo.default_range();
        assert_eq!(min, 20.0);
        assert_eq!(max, 300.0);
        assert_eq!(default, 120.0);
    }

    #[test]
    fn test_serialization() {
        let target = AutomationTarget::node_param_named(42, 0, "cutoff");
        let json = serde_json::to_string(&target).unwrap();
        let deserialized: AutomationTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(target, deserialized);
    }
}
