use crate::bridge::PluginBridge;
use crate::protocol::{ParameterInfo, PluginMetadata};
use std::sync::Arc;

/// Main-thread control handle for a loaded plugin (editor, state, parameters).
/// Clone is cheap (Arc-based). Action methods return `&Self` for chaining.
#[derive(Clone)]
pub struct PluginHandle {
    bridge: Arc<dyn PluginBridge>,
    metadata: PluginMetadata,
}

impl PluginHandle {
    /// Must be called before the client is moved into `Box<dyn AudioUnit>`.
    pub fn from_client(client: &crate::client::PluginClient) -> Option<Self> {
        let bridge = client.bridge_arc()?;
        Some(Self {
            bridge,
            metadata: client.metadata().clone(),
        })
    }

    pub fn from_bridge_and_metadata(
        bridge: Arc<dyn PluginBridge>,
        metadata: PluginMetadata,
    ) -> Self {
        Self { bridge, metadata }
    }

    pub fn has_editor(&self) -> bool {
        self.metadata.has_editor
    }

    /// `parent_handle` is the native window handle (NSView*, HWND, etc.) cast to u64.
    pub fn open_editor(&self, parent_handle: u64) -> Option<(u32, u32)> {
        self.bridge.open_editor(parent_handle)
    }

    pub fn close_editor(&self) -> &Self {
        self.bridge.close_editor();
        self
    }

    /// Call periodically (~30Hz) while editor is open.
    pub fn editor_idle(&self) -> &Self {
        self.bridge.editor_idle();
        self
    }

    pub fn save_state(&self) -> Option<Vec<u8>> {
        self.bridge.save_state()
    }

    pub fn load_state(&self, data: &[u8]) -> &Self {
        self.bridge.load_state(data);
        self
    }

    pub fn parameters(&self) -> Option<Vec<ParameterInfo>> {
        self.bridge.get_parameter_list()
    }

    pub fn get_parameter(&self, param_id: u32) -> Option<f32> {
        self.bridge.get_parameter(param_id)
    }

    /// RT-safe, fire-and-forget.
    pub fn set_parameter(&self, param_id: u32, value: f32) -> &Self {
        self.bridge.set_parameter_rt(param_id, value);
        self
    }

    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    pub fn is_crashed(&self) -> bool {
        self.bridge.is_crashed()
    }
}
