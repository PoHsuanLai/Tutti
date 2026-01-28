//! Neural audio processing traits, metadata, and graph analysis.
//!
//! No GPU or ML framework dependencies — only traits and infrastructure for
//! graph-aware batching. Concrete implementations live in external crates.

pub mod metadata;
pub mod traits;
pub mod graph_analyzer;

// Re-export core types
pub use metadata::{NeuralModelId, NeuralNodeInfo, NeuralNodeManager, SharedNeuralNodeManager};
pub use traits::{
    ArcNeuralEffectBuilder, ArcNeuralSynthBuilder, NeuralEffectBuilder, NeuralSynthBuilder,
};
pub use graph_analyzer::{BatchingStrategy, GraphAnalyzer};

