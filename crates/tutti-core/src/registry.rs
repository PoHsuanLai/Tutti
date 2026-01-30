//! Node Registry for Dynamic Node Creation
//!
//! Provides a global registry for creating audio nodes from string identifiers.
//! This enables dynamic node creation from serialized data or scripting languages.

use fundsp::prelude::AudioUnit;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Create a `NodeParams` HashMap with key-value pairs.
///
/// # Example
/// ```ignore
/// let params = params! {
///     "frequency" => 440.0,
///     "q" => 1.0,
/// };
/// ```
#[macro_export]
macro_rules! params {
    ($($key:expr => $value:expr),* $(,)?) => {{
        let mut map = $crate::NodeParams::new();
        $(
            map.insert($key.to_string(), $value.into());
        )*
        map
    }};
}

/// Create a node from registry and add it to the graph with a tag.
///
/// # Example
/// ```ignore
/// let sine_id = node!(net, registry, "sine", "my_sine", {
///     "frequency" => 440.0
/// });
/// ```
#[macro_export]
macro_rules! node {
    ($net:expr, $registry:expr, $node_type:expr, $tag:expr, { $($key:expr => $value:expr),* $(,)? }) => {{
        let params = $crate::params! { $($key => $value),* };
        let audio_unit = $registry.create($node_type, &params)
            .expect(&format!("Failed to create node type '{}'", $node_type));
        $net.add_tagged(audio_unit, $tag)
    }};

    ($net:expr, $registry:expr, $node_type:expr, $tag:expr) => {{
        let params = $crate::NodeParams::new();
        let audio_unit = $registry.create($node_type, &params)
            .expect(&format!("Failed to create node type '{}'", $node_type));
        $net.add_tagged(audio_unit, $tag)
    }};
}

/// Function that constructs a node from parameters
pub type NodeConstructor =
    Arc<dyn Fn(&NodeParams) -> Result<Box<dyn AudioUnit>, NodeRegistryError> + Send + Sync>;

/// Node parameters (simple key-value map)
pub type NodeParams = HashMap<String, NodeParamValue>;

/// Parameter value types
#[derive(Debug, Clone)]
pub enum NodeParamValue {
    Float(f64),
    Int(i64),
    Bool(bool),
    String(String),
}

impl NodeParamValue {
    /// Convert to f64 if possible
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Convert to f32 if possible
    pub fn as_f32(&self) -> Option<f32> {
        self.as_f64().map(|f| f as f32)
    }

    /// Convert to i64 if possible
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    /// Convert to bool if possible
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Convert to string slice if possible
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

impl From<f64> for NodeParamValue {
    fn from(f: f64) -> Self {
        Self::Float(f)
    }
}

impl From<f32> for NodeParamValue {
    fn from(f: f32) -> Self {
        Self::Float(f as f64)
    }
}

impl From<i64> for NodeParamValue {
    fn from(i: i64) -> Self {
        Self::Int(i)
    }
}

impl From<i32> for NodeParamValue {
    fn from(i: i32) -> Self {
        Self::Int(i as i64)
    }
}

impl From<bool> for NodeParamValue {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<String> for NodeParamValue {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<&str> for NodeParamValue {
    fn from(s: &str) -> Self {
        Self::String(s.to_string())
    }
}

/// Global registry of node constructors
pub struct NodeRegistry {
    constructors: Arc<RwLock<HashMap<String, NodeConstructor>>>,
}

impl NodeRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            constructors: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a node constructor
    ///
    /// # Example
    /// ```ignore
    /// registry.register("sine", |params| {
    ///     let freq = params.get("frequency")
    ///         .and_then(|v| v.as_f32())
    ///         .unwrap_or(440.0);
    ///     Ok(Box::new(sine_hz(freq)))
    /// });
    /// ```
    pub fn register<F>(&self, name: impl Into<String>, constructor: F)
    where
        F: Fn(&NodeParams) -> Result<Box<dyn AudioUnit>, NodeRegistryError> + Send + Sync + 'static,
    {
        self.constructors
            .write()
            .unwrap()
            .insert(name.into(), Arc::new(constructor));
    }

    /// Create a node from registered name and parameters
    ///
    /// # Example
    /// ```ignore
    /// let mut params = NodeParams::new();
    /// params.insert("frequency".to_string(), 880.0.into());
    /// let node = registry.create("sine", &params)?;
    /// ```
    pub fn create(
        &self,
        name: &str,
        params: &NodeParams,
    ) -> Result<Box<dyn AudioUnit>, NodeRegistryError> {
        let constructors = self.constructors.read().unwrap();
        let constructor = constructors
            .get(name)
            .ok_or_else(|| NodeRegistryError::UnknownNodeType(name.to_string()))?;

        constructor(params)
    }

    /// List all registered node types
    pub fn list_types(&self) -> Vec<String> {
        self.constructors.read().unwrap().keys().cloned().collect()
    }

    /// Check if a type is registered
    pub fn has_type(&self, name: &str) -> bool {
        self.constructors.read().unwrap().contains_key(name)
    }

    /// Unregister a node type
    pub fn unregister(&self, name: &str) -> bool {
        self.constructors.write().unwrap().remove(name).is_some()
    }

    /// Clear all registrations
    pub fn clear(&self) {
        self.constructors.write().unwrap().clear();
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        let registry = Self::new();
        register_builtin_nodes(&registry);
        registry
    }
}

impl Clone for NodeRegistry {
    fn clone(&self) -> Self {
        Self {
            constructors: Arc::clone(&self.constructors),
        }
    }
}

/// Errors that can occur when using the registry
#[derive(Debug, thiserror::Error)]
pub enum NodeRegistryError {
    #[error("Unknown node type: {0}")]
    UnknownNodeType(String),

    #[error("Missing required parameter: {0}")]
    MissingParameter(String),

    #[error("Invalid parameter value for '{0}': {1}")]
    InvalidParameter(String, String),

    #[error("Node construction failed: {0}")]
    ConstructionFailed(String),
}

/// Helper to get a required parameter
pub fn get_param<T>(
    params: &NodeParams,
    name: &str,
    convert: impl FnOnce(&NodeParamValue) -> Option<T>,
) -> Result<T, NodeRegistryError> {
    params
        .get(name)
        .ok_or_else(|| NodeRegistryError::MissingParameter(name.to_string()))
        .and_then(|v| {
            convert(v).ok_or_else(|| {
                NodeRegistryError::InvalidParameter(name.to_string(), format!("{:?}", v))
            })
        })
}

/// Helper to get an optional parameter with default
pub fn get_param_or<T>(
    params: &NodeParams,
    name: &str,
    default: T,
    convert: impl FnOnce(&NodeParamValue) -> Option<T>,
) -> T {
    params.get(name).and_then(convert).unwrap_or(default)
}

/// Register built-in node types
fn register_builtin_nodes(registry: &NodeRegistry) {
    use fundsp::prelude::*;

    // =========================================================================
    // Generators
    // =========================================================================

    registry.register("sine", |params| {
        let freq = get_param_or(params, "frequency", 440.0, |v| v.as_f32());
        Ok(Box::new(sine_hz::<f32>(freq)))
    });

    registry.register("saw", |params| {
        let freq = get_param_or(params, "frequency", 440.0, |v| v.as_f32());
        Ok(Box::new(saw_hz(freq)))
    });

    registry.register("square", |params| {
        let freq = get_param_or(params, "frequency", 440.0, |v| v.as_f32());
        Ok(Box::new(square_hz(freq)))
    });

    registry.register("triangle", |params| {
        let freq = get_param_or(params, "frequency", 440.0, |v| v.as_f32());
        Ok(Box::new(triangle_hz(freq)))
    });

    registry.register("noise", |_params| Ok(Box::new(noise())));

    registry.register("dc", |params| {
        let value = get_param_or(params, "value", 0.0, |v| v.as_f32());
        Ok(Box::new(dc::<f32>(value)))
    });

    // =========================================================================
    // Effects
    // =========================================================================

    registry.register("reverb_stereo", |params| {
        let room_size = get_param_or(params, "room_size", 0.5, |v| v.as_f64());
        let time = get_param_or(params, "time", 5.0, |v| v.as_f64());
        let diffusion = get_param_or(params, "diffusion", 1.0, |v| v.as_f64());
        Ok(Box::new(reverb_stereo(room_size, time, diffusion)))
    });

    registry.register("delay", |params| {
        let time = get_param_or(params, "time", 0.25, |v| v.as_f64());
        Ok(Box::new(delay(time)))
    });

    registry.register("lowpass", |params| {
        let cutoff = get_param_or(params, "cutoff", 1000.0, |v| v.as_f32());
        Ok(Box::new(lowpass_hz::<f32>(cutoff, 1.0)))
    });

    registry.register("highpass", |params| {
        let cutoff = get_param_or(params, "cutoff", 1000.0, |v| v.as_f32());
        Ok(Box::new(highpass_hz::<f32>(cutoff, 1.0)))
    });

    registry.register("bandpass", |params| {
        let center = get_param_or(params, "center", 1000.0, |v| v.as_f32());
        let q = get_param_or(params, "q", 1.0, |v| v.as_f32());
        Ok(Box::new(bandpass_hz::<f32>(center, q)))
    });

    // =========================================================================
    // Utilities
    // =========================================================================

    registry.register("pass", |_params| Ok(Box::new(pass())));

    registry.register("mul", |params| {
        let value = get_param_or(params, "value", 1.0, |v| v.as_f32());
        Ok(Box::new(mul::<f32>(value)))
    });

    registry.register("add", |params| {
        let value = get_param_or(params, "value", 0.0, |v| v.as_f32());
        Ok(Box::new(add::<f32>(value)))
    });

    registry.register("pan", |params| {
        let pan_value = get_param_or(params, "pan", 0.0, |v| v.as_f32());
        Ok(Box::new(fundsp::prelude::pan(pan_value)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_param_conversion() {
        let val = NodeParamValue::Float(440.5);
        assert_eq!(val.as_f64(), Some(440.5));
        assert_eq!(val.as_f32(), Some(440.5_f32));
        assert_eq!(val.as_i64(), Some(440));

        let val = NodeParamValue::Int(42);
        assert_eq!(val.as_i64(), Some(42));
        assert_eq!(val.as_f64(), Some(42.0));

        let val = NodeParamValue::Bool(true);
        assert_eq!(val.as_bool(), Some(true));

        let val = NodeParamValue::String("test".to_string());
        assert_eq!(val.as_str(), Some("test"));
    }

    #[test]
    fn test_registry_basic() {
        let registry = NodeRegistry::new();

        // Register a simple node
        registry.register("test", |_params| {
            Ok(Box::new(fundsp::prelude::pass::<f32>()))
        });

        assert!(registry.has_type("test"));
        assert!(!registry.has_type("nonexistent"));

        let types = registry.list_types();
        assert_eq!(types.len(), 1);
        assert!(types.contains(&"test".to_string()));
    }

    #[test]
    fn test_builtin_nodes() {
        let registry = NodeRegistry::default();

        let types = registry.list_types();
        assert!(types.contains(&"sine".to_string()));
        assert!(types.contains(&"reverb_stereo".to_string()));
        assert!(types.contains(&"lowpass".to_string()));
    }

    #[test]
    fn test_create_node() {
        let registry = NodeRegistry::default();

        let mut params = NodeParams::new();
        params.insert("frequency".to_string(), 880.0.into());

        let node = registry.create("sine", &params);
        assert!(node.is_ok());

        let node = node.unwrap();
        assert_eq!(node.inputs(), 0);
        assert_eq!(node.outputs(), 1);
    }

    #[test]
    fn test_unknown_node_type() {
        let registry = NodeRegistry::default();

        let params = NodeParams::new();
        let result = registry.create("nonexistent", &params);

        assert!(result.is_err());
        match result {
            Err(NodeRegistryError::UnknownNodeType(name)) => {
                assert_eq!(name, "nonexistent");
            }
            _ => panic!("Expected UnknownNodeType error"),
        }
    }
}
