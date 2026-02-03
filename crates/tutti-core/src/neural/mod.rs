//! Neural audio processing traits, metadata, and graph analysis.
//!
//! No GPU or ML framework dependencies — only traits and infrastructure for
//! graph-aware batching. Concrete implementations live in external crates.

pub mod graph_analyzer;
pub mod metadata;
pub mod traits;

// Re-export core types
pub use graph_analyzer::BatchingStrategy;
pub(crate) use graph_analyzer::GraphAnalyzer;
pub use metadata::{NeuralModelId, NeuralNodeManager, SharedNeuralNodeManager};
pub use traits::{
    ArcNeuralEffectBuilder, ArcNeuralSynthBuilder, NeuralEffectBuilder, NeuralSynthBuilder,
};
