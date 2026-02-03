//! Node registry for dynamic node creation.

use crate::compat::{Arc, Box, HashMap, RwLock, String, ToString, Vec};
use crate::error::NodeRegistryError;
use fundsp::prelude::AudioUnit;

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
            map.insert($key, $value.into());  // No .to_string() - zero allocation!
        )*
        map
    }};
}

/// Function that constructs a node from parameters
pub type NodeConstructor =
    Arc<dyn Fn(&NodeParams) -> Result<Box<dyn AudioUnit>, NodeRegistryError> + Send + Sync>;

/// Node parameters (simple key-value map)
///
/// Uses `&'static str` for keys to avoid string allocations.
/// Parameter names are known at compile time, so this is zero-cost.
pub type NodeParams = HashMap<&'static str, NodeParamValue>;

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

/// Trait for converting NodeParamValue to a specific type
pub trait ParamConvert: Sized {
    fn from_param(value: &NodeParamValue) -> Option<Self>;
}

impl ParamConvert for f32 {
    fn from_param(value: &NodeParamValue) -> Option<Self> {
        value.as_f32()
    }
}

impl ParamConvert for f64 {
    fn from_param(value: &NodeParamValue) -> Option<Self> {
        value.as_f64()
    }
}

impl ParamConvert for i32 {
    fn from_param(value: &NodeParamValue) -> Option<Self> {
        value.as_i64().map(|i| i as i32)
    }
}

impl ParamConvert for i64 {
    fn from_param(value: &NodeParamValue) -> Option<Self> {
        value.as_i64()
    }
}

impl ParamConvert for bool {
    fn from_param(value: &NodeParamValue) -> Option<Self> {
        value.as_bool()
    }
}

impl ParamConvert for String {
    fn from_param(value: &NodeParamValue) -> Option<Self> {
        value.as_str().map(|s| s.to_string())
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
    ///     let freq: f32 = get_param_or(params, "frequency", 440.0);
    ///     Ok(Box::new(sine_hz(freq)))
    /// });
    /// ```
    pub fn register<F>(&self, name: impl Into<String>, constructor: F)
    where
        F: Fn(&NodeParams) -> Result<Box<dyn AudioUnit>, NodeRegistryError> + Send + Sync + 'static,
    {
        let name = name.into();
        self.constructors
            .write()
            .insert(name, Arc::new(constructor));
    }

    /// Create a node instance from registered type
    pub fn create(
        &self,
        node_type: &str,
        params: &NodeParams,
    ) -> Result<Box<dyn AudioUnit>, NodeRegistryError> {
        let constructors = self.constructors.read();
        let constructor = constructors
            .get(node_type)
            .ok_or_else(|| NodeRegistryError::UnknownNodeType(node_type.to_string()))?
            .clone();
        drop(constructors);

        constructor(params)
    }

    /// List all registered node types
    pub fn list_types(&self) -> Vec<String> {
        self.constructors.read().keys().cloned().collect()
    }

    /// Check if a type is registered
    pub fn has_type(&self, name: &str) -> bool {
        self.constructors.read().contains_key(name)
    }

    /// Unregister a node type
    pub fn unregister(&self, name: &str) -> bool {
        self.constructors.write().remove(name).is_some()
    }

    /// Clear all registrations
    pub fn clear(&self) {
        self.constructors.write().clear();
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for NodeRegistry {
    fn clone(&self) -> Self {
        Self {
            constructors: Arc::clone(&self.constructors),
        }
    }
}

/// Helper to get a required parameter
///
/// # Example
/// ```ignore
/// let freq: f32 = get_param(params, "frequency")?;
/// let volume: f64 = get_param(params, "volume")?;
/// let enabled: bool = get_param(params, "enabled")?;
/// ```
pub fn get_param<T: ParamConvert>(params: &NodeParams, name: &str) -> Result<T, NodeRegistryError> {
    params
        .get(name)
        .ok_or_else(|| NodeRegistryError::MissingParameter(name.to_string()))
        .and_then(|v| {
            T::from_param(v).ok_or_else(|| {
                NodeRegistryError::InvalidParameter(name.to_string(), format!("{:?}", v))
            })
        })
}

/// Helper to get an optional parameter with default
///
/// # Example
/// ```ignore
/// let freq: f32 = get_param_or(params, "frequency", 440.0);
/// let volume: f64 = get_param_or(params, "volume", 0.5);
/// let enabled: bool = get_param_or(params, "enabled", true);
/// ```
pub fn get_param_or<T: ParamConvert>(params: &NodeParams, name: &str, default: T) -> T {
    params
        .get(name)
        .and_then(|v| T::from_param(v))
        .unwrap_or(default)
}
