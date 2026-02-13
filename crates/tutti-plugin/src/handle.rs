//! PluginHandle — main-thread control handle for loaded plugins.

use crate::bridge::PluginBridge;
use crate::protocol::{ParameterInfo, PluginMetadata};
use std::sync::Arc;

/// Handle for controlling a loaded plugin from the main thread.
///
/// Provides editor, state, and parameter query operations.
/// Clone is cheap (bridge is Arc-based internally).
/// Does not touch audio processing.
///
/// Action methods return `&Self` for chaining. Query methods return data.
///
/// # Example
///
/// ```ignore
/// let (unit, handle) = engine.clap("Reverb.clap").build()?;
/// engine.graph_mut(|net| net.add(unit).master());
///
/// // Chainable actions
/// handle
///     .set_parameter(1, 0.8)
///     .set_parameter(2, 0.5)
///     .set_parameter(3, 120.0);
///
/// // Save/load presets
/// let state = handle.save_state().unwrap();
/// handle.load_state(&state);
///
/// // Open editor in a window
/// if let Some((w, h)) = handle.open_editor(window_handle) {
///     // Size the window to w×h
/// }
/// ```
#[derive(Clone)]
pub struct PluginHandle {
    bridge: Arc<dyn PluginBridge>,
    metadata: PluginMetadata,
}

impl PluginHandle {
    /// Create a PluginHandle from a PluginClient (before it's moved into Box<dyn AudioUnit>).
    pub fn from_client(client: &crate::client::PluginClient) -> Option<Self> {
        let bridge = client.bridge_arc()?;
        Some(Self {
            bridge,
            metadata: client.metadata().clone(),
        })
    }

    /// Create a PluginHandle directly from a bridge and metadata.
    ///
    /// Useful for testing or when constructing a handle without a full PluginClient.
    pub fn from_bridge_and_metadata(
        bridge: Arc<dyn PluginBridge>,
        metadata: PluginMetadata,
    ) -> Self {
        Self { bridge, metadata }
    }

    // =========================================================================
    // Editor
    // =========================================================================

    /// Whether the plugin has a GUI editor.
    pub fn has_editor(&self) -> bool {
        self.metadata.has_editor
    }

    /// Open the plugin editor GUI. Returns (width, height) on success.
    ///
    /// `parent_handle` is the native window handle (NSView*, HWND, etc.) cast to u64.
    pub fn open_editor(&self, parent_handle: u64) -> Option<(u32, u32)> {
        self.bridge.open_editor(parent_handle)
    }

    /// Close the plugin editor GUI.
    pub fn close_editor(&self) -> &Self {
        self.bridge.close_editor();
        self
    }

    /// Tick the plugin editor idle loop. Call periodically (e.g., 30Hz) while editor is open.
    pub fn editor_idle(&self) -> &Self {
        self.bridge.editor_idle();
        self
    }

    // =========================================================================
    // State
    // =========================================================================

    /// Save the plugin state. Returns the state bytes on success.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        self.bridge.save_state()
    }

    /// Load plugin state from bytes.
    pub fn load_state(&self, data: &[u8]) -> &Self {
        self.bridge.load_state(data);
        self
    }

    // =========================================================================
    // Parameters
    // =========================================================================

    /// Get the full parameter list from the plugin.
    pub fn parameters(&self) -> Option<Vec<ParameterInfo>> {
        self.bridge.get_parameter_list()
    }

    /// Get a single parameter value.
    pub fn get_parameter(&self, param_id: u32) -> Option<f32> {
        self.bridge.get_parameter(param_id)
    }

    /// Set a parameter value. RT-safe (fire-and-forget).
    pub fn set_parameter(&self, param_id: u32, value: f32) -> &Self {
        self.bridge.set_parameter_rt(param_id, value);
        self
    }

    // =========================================================================
    // Metadata
    // =========================================================================

    /// Get the plugin metadata.
    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    /// Get the plugin name.
    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    /// Returns true if the plugin server process has crashed.
    pub fn is_crashed(&self) -> bool {
        self.bridge.is_crashed()
    }
}
