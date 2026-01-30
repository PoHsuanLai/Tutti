//! Plugin Registration for NodeRegistry
//!
//! Provides registration functions for VST2, VST3, and CLAP plugins
//! to be used with tutti's NodeRegistry system.

use crate::client::PluginClient;
use crate::error::BridgeError;
use crate::protocol::BridgeConfig;
use std::path::{Path, PathBuf};
use tutti_core::{get_param_or, NodeRegistry, NodeRegistryError};

// Allow BridgeError to convert to NodeRegistryError
impl From<BridgeError> for NodeRegistryError {
    fn from(e: BridgeError) -> Self {
        NodeRegistryError::Plugin(e.to_string())
    }
}

/// Register a single plugin by path
///
/// # Example
/// ```ignore
/// let registry = NodeRegistry::default();
/// register_plugin(&registry, "my_reverb", "/path/to/reverb.vst3")?;
///
/// // Later, create instances:
/// let reverb = registry.create("my_reverb", &params! {
///     "preset" => "Large Hall"
/// })?;
/// ```
/// Register a plugin with a tokio runtime handle
///
/// # Example
/// ```ignore
/// let runtime = tokio::runtime::Runtime::new()?;
/// let registry = NodeRegistry::default();
///
/// register_plugin(&registry, runtime.handle(), "reverb", "/path/to/reverb.vst3")?;
///
/// // Later, create instances:
/// let reverb = registry.create("reverb", &params! {
///     "sample_rate" => 44100.0
/// })?;
/// ```
pub fn register_plugin<P: AsRef<Path>>(
    registry: &NodeRegistry,
    runtime: &tokio::runtime::Handle,
    name: impl Into<String>,
    path: P,
) -> Result<(), BridgeError> {
    let path_buf = path.as_ref().to_path_buf();
    let plugin_name = name.into();
    let runtime = runtime.clone();

    registry.register(plugin_name, move |params| {
        // Get sample rate from params (required for plugin loading)
        let sample_rate = get_param_or(params, "sample_rate", 44100.0, |v| v.as_f64());

        // Load plugin using runtime.block_on
        let (client, _handle) = runtime.block_on(PluginClient::load(
            BridgeConfig::default(),
            path_buf.clone(),
            sample_rate,
        ))?;

        // TODO: Apply other parameters (preset, etc.)

        Ok(Box::new(client))
    });

    Ok(())
}

/// Register all plugins in a directory
///
/// Scans the directory for .vst, .vst3, and .clap files and registers them.
///
/// # Example
/// ```ignore
/// register_plugin_directory(&registry, "/Library/Audio/Plug-Ins/VST3")?;
/// ```
pub fn register_plugin_directory<P: AsRef<Path>>(
    registry: &NodeRegistry,
    runtime: &tokio::runtime::Handle,
    path: P,
) -> Result<Vec<String>, BridgeError> {
    let mut registered = Vec::new();
    let dir_path = path.as_ref();

    if !dir_path.is_dir() {
        return Err(BridgeError::LoadFailed(format!(
            "Not a directory: {}",
            dir_path.display()
        )));
    }

    // Scan for plugin files
    for entry in std::fs::read_dir(dir_path)
        .map_err(|e| BridgeError::LoadFailed(format!("Failed to read directory: {}", e)))?
    {
        let entry =
            entry.map_err(|e| BridgeError::LoadFailed(format!("Failed to read entry: {}", e)))?;
        let path = entry.path();

        if is_plugin_file(&path) {
            // Use filename (without extension) as registry key
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                match register_plugin(registry, runtime, name, &path) {
                    Ok(_) => {
                        tracing::info!("Registered plugin: {} from {}", name, path.display());
                        registered.push(name.to_string());
                    }
                    Err(e) => {
                        tracing::warn!("Failed to register {}: {}", path.display(), e);
                    }
                }
            }
        }
    }

    Ok(registered)
}

/// Register VST2 plugins from standard system paths
///
/// Scans platform-specific VST2 plugin directories.
#[cfg(feature = "vst2")]
pub fn register_system_vst2_plugins(
    registry: &NodeRegistry,
    runtime: &tokio::runtime::Handle,
) -> Result<Vec<String>, BridgeError> {
    let mut registered = Vec::new();

    for path in get_vst2_search_paths() {
        if path.exists() {
            match register_plugin_directory(registry, runtime, &path) {
                Ok(mut plugins) => registered.append(&mut plugins),
                Err(e) => tracing::warn!("Failed to scan {}: {}", path.display(), e),
            }
        }
    }

    Ok(registered)
}

/// Register VST3 plugins from standard system paths
///
/// Scans platform-specific VST3 plugin directories.
#[cfg(feature = "vst3")]
pub fn register_system_vst3_plugins(
    registry: &NodeRegistry,
    runtime: &tokio::runtime::Handle,
) -> Result<Vec<String>, BridgeError> {
    let mut registered = Vec::new();

    for path in get_vst3_search_paths() {
        if path.exists() {
            match register_plugin_directory(registry, runtime, &path) {
                Ok(mut plugins) => registered.append(&mut plugins),
                Err(e) => tracing::warn!("Failed to scan {}: {}", path.display(), e),
            }
        }
    }

    Ok(registered)
}

/// Register CLAP plugins from standard system paths
///
/// Scans platform-specific CLAP plugin directories.
#[cfg(feature = "clap")]
pub fn register_system_clap_plugins(
    registry: &NodeRegistry,
    runtime: &tokio::runtime::Handle,
) -> Result<Vec<String>, BridgeError> {
    let mut registered = Vec::new();

    for path in get_clap_search_paths() {
        if path.exists() {
            match register_plugin_directory(registry, runtime, &path) {
                Ok(mut plugins) => registered.append(&mut plugins),
                Err(e) => tracing::warn!("Failed to scan {}: {}", path.display(), e),
            }
        }
    }

    Ok(registered)
}

/// Register all system plugins (VST2, VST3, CLAP)
///
/// Convenience function that scans all standard plugin directories.
pub fn register_all_system_plugins(
    registry: &NodeRegistry,
    runtime: &tokio::runtime::Handle,
) -> Result<Vec<String>, BridgeError> {
    let mut registered = Vec::new();

    #[cfg(feature = "vst2")]
    {
        registered.extend(register_system_vst2_plugins(registry, runtime)?);
    }

    #[cfg(feature = "vst3")]
    {
        registered.extend(register_system_vst3_plugins(registry, runtime)?);
    }

    #[cfg(feature = "clap")]
    {
        registered.extend(register_system_clap_plugins(registry, runtime)?);
    }

    tracing::info!("Registered {} plugins total", registered.len());
    Ok(registered)
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Check if a path is a plugin file
fn is_plugin_file(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        matches!(ext, "vst" | "vst3" | "clap" | "component")
    } else {
        false
    }
}

// NOTE: Parameter setting removed - will be implemented when async registration is added

/// Get VST2 search paths for the current platform
#[cfg(feature = "vst2")]
fn get_vst2_search_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from("/Library/Audio/Plug-Ins/VST"),
            PathBuf::from(format!(
                "{}/Library/Audio/Plug-Ins/VST",
                std::env::var("HOME").unwrap_or_default()
            )),
        ]
    }

    #[cfg(target_os = "windows")]
    {
        vec![
            PathBuf::from("C:\\Program Files\\VstPlugins"),
            PathBuf::from("C:\\Program Files\\Common Files\\VST2"),
            PathBuf::from("C:\\Program Files (x86)\\VstPlugins"),
        ]
    }

    #[cfg(target_os = "linux")]
    {
        vec![
            PathBuf::from("/usr/lib/vst"),
            PathBuf::from("/usr/local/lib/vst"),
            PathBuf::from(format!(
                "{}/.vst",
                std::env::var("HOME").unwrap_or_default()
            )),
        ]
    }
}

/// Get VST3 search paths for the current platform
#[cfg(feature = "vst3")]
fn get_vst3_search_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from("/Library/Audio/Plug-Ins/VST3"),
            PathBuf::from(format!(
                "{}/Library/Audio/Plug-Ins/VST3",
                std::env::var("HOME").unwrap_or_default()
            )),
        ]
    }

    #[cfg(target_os = "windows")]
    {
        vec![
            PathBuf::from("C:\\Program Files\\Common Files\\VST3"),
            PathBuf::from("C:\\Program Files (x86)\\Common Files\\VST3"),
        ]
    }

    #[cfg(target_os = "linux")]
    {
        vec![
            PathBuf::from("/usr/lib/vst3"),
            PathBuf::from("/usr/local/lib/vst3"),
            PathBuf::from(format!(
                "{}/.vst3",
                std::env::var("HOME").unwrap_or_default()
            )),
        ]
    }
}

/// Get CLAP search paths for the current platform
#[cfg(feature = "clap")]
fn get_clap_search_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from("/Library/Audio/Plug-Ins/CLAP"),
            PathBuf::from(format!(
                "{}/Library/Audio/Plug-Ins/CLAP",
                std::env::var("HOME").unwrap_or_default()
            )),
        ]
    }

    #[cfg(target_os = "windows")]
    {
        vec![
            PathBuf::from("C:\\Program Files\\Common Files\\CLAP"),
            PathBuf::from("C:\\Program Files (x86)\\Common Files\\CLAP"),
        ]
    }

    #[cfg(target_os = "linux")]
    {
        vec![
            PathBuf::from("/usr/lib/clap"),
            PathBuf::from("/usr/local/lib/clap"),
            PathBuf::from(format!(
                "{}/.clap",
                std::env::var("HOME").unwrap_or_default()
            )),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_plugin_file() {
        assert!(is_plugin_file(Path::new("/path/to/plugin.vst")));
        assert!(is_plugin_file(Path::new("/path/to/plugin.vst3")));
        assert!(is_plugin_file(Path::new("/path/to/plugin.clap")));
        assert!(!is_plugin_file(Path::new("/path/to/plugin.txt")));
        assert!(!is_plugin_file(Path::new("/path/to/plugin")));
    }
}
