//! Plugin metadata for IPC protocol
//!
//! This is used for serializing plugin information between the host and bridge processes.

use serde::{Deserialize, Serialize};

/// Audio I/O configuration
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AudioIO {
    /// Number of audio input channels
    pub inputs: usize,
    /// Number of audio output channels
    pub outputs: usize,
}

impl AudioIO {
    /// Stereo in, stereo out
    pub fn stereo() -> Self {
        Self {
            inputs: 2,
            outputs: 2,
        }
    }
}

/// Plugin metadata for bridge protocol
///
/// This is a simplified version used for IPC. It contains the essential
/// information needed to identify and configure a plugin.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PluginMetadata {
    /// Unique plugin ID
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Vendor/author name
    pub vendor: String,

    /// Version string
    pub version: String,

    /// Audio I/O configuration
    pub audio_io: AudioIO,

    /// Does this plugin receive MIDI?
    pub receives_midi: bool,

    /// Does this plugin have a custom GUI?
    pub has_editor: bool,

    /// Editor size (width, height) if available
    pub editor_size: Option<(u32, u32)>,

    /// Plugin latency in samples
    pub latency_samples: usize,

    /// Whether the plugin supports 64-bit (f64) audio processing
    #[serde(default)]
    pub supports_f64: bool,
}

impl PluginMetadata {
    /// Create new metadata with required fields
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            vendor: String::new(),
            version: "1.0.0".to_string(),
            audio_io: AudioIO::stereo(),
            receives_midi: false,
            has_editor: false,
            editor_size: None,
            latency_samples: 0,
            supports_f64: false,
        }
    }

    pub fn vendor(mut self, vendor: impl Into<String>) -> Self {
        self.vendor = vendor.into();
        self
    }

    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.vendor = author.into();
        self
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    pub fn audio_io(mut self, inputs: usize, outputs: usize) -> Self {
        self.audio_io = AudioIO { inputs, outputs };
        self
    }

    pub fn midi(mut self, receives_midi: bool) -> Self {
        self.receives_midi = receives_midi;
        self
    }

    pub fn editor(mut self, has_editor: bool, size: Option<(u32, u32)>) -> Self {
        self.has_editor = has_editor;
        self.editor_size = size;
        self
    }

    pub fn latency(mut self, samples: usize) -> Self {
        self.latency_samples = samples;
        self
    }

    pub fn f64_support(mut self, supports_f64: bool) -> Self {
        self.supports_f64 = supports_f64;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_f64_support() {
        let meta = PluginMetadata::new("test.plugin", "Test Plugin").f64_support(true);
        assert!(meta.supports_f64);

        let meta_no_f64 = PluginMetadata::new("test.plugin2", "Test Plugin 2");
        assert!(!meta_no_f64.supports_f64);
    }

    #[test]
    fn test_metadata_f64_serde_roundtrip() {
        let meta = PluginMetadata::new("test.reverb", "Super Reverb")
            .vendor("TestCo")
            .audio_io(2, 2)
            .f64_support(true);

        let json = serde_json::to_string(&meta).unwrap();
        let decoded: PluginMetadata = serde_json::from_str(&json).unwrap();

        assert!(decoded.supports_f64);
        assert_eq!(decoded.name, "Super Reverb");
    }

    #[test]
    fn test_metadata_f64_serde_default_false() {
        // Simulate deserializing old metadata without supports_f64 field
        let json = r#"{"id":"old","name":"Old Plugin","vendor":"","version":"1.0.0","audio_io":{"inputs":2,"outputs":2},"receives_midi":false,"has_editor":false,"editor_size":null,"latency_samples":0}"#;
        let decoded: PluginMetadata = serde_json::from_str(json).unwrap();

        // Should default to false for backward compat
        assert!(!decoded.supports_f64);
    }
}
