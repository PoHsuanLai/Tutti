//! Plugin metadata for IPC serialization.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AudioIO {
    pub inputs: usize,
    pub outputs: usize,
}

impl AudioIO {
    pub fn stereo() -> Self {
        Self {
            inputs: 2,
            outputs: 2,
        }
    }
}

/// Plugin metadata exchanged over IPC.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PluginMetadata {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub audio_io: AudioIO,
    pub receives_midi: bool,
    pub has_editor: bool,
    pub editor_size: Option<(u32, u32)>,
    pub latency_samples: usize,
    #[serde(default)]
    pub supports_f64: bool,
}

impl PluginMetadata {
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
