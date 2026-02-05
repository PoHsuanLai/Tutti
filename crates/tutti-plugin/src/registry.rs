//! Plugin Registration for NodeRegistry
//!
//! Provides registration functions for plugin paths to be used with tutti's NodeRegistry system.

use crate::client::PluginClient;
use crate::error::{BridgeError, LoadStage};
use crate::protocol::BridgeConfig;
use std::path::Path;
use std::path::PathBuf;
use tutti_core::{NodeRegistry, NodeRegistryError, Params};

// Allow BridgeError to convert to NodeRegistryError
impl From<BridgeError> for NodeRegistryError {
    fn from(e: BridgeError) -> Self {
        NodeRegistryError::Plugin(e.to_string())
    }
}

/// Register a single plugin by path.
///
/// The plugin server process (`tutti-plugin-server`) must be available in PATH
/// or the same directory as the executable.
///
/// # Supported Parameters
///
/// - `sample_rate` - Sample rate in Hz (default: 44100.0)
/// - `param_<id>` - Set parameter by native ID (e.g., `param_0`, `param_1`)
///
/// # Example
/// ```ignore
/// let runtime = tokio::runtime::Runtime::new()?;
/// let registry = NodeRegistry::default();
///
/// register_plugin(&registry, runtime.handle(), "reverb", "/path/to/reverb.vst3")?;
///
/// // Create instance with custom parameters:
/// let reverb = registry.create("reverb", &params! {
///     "sample_rate" => "48000.0",
///     "param_0" => "0.75",
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
        let p = Params::new(params);
        // Get sample rate from params (required for plugin loading)
        let sample_rate: f64 = p.get_or("sample_rate", 44100.0);

        // Load plugin using runtime.block_on
        let (client, _handle) = runtime.block_on(PluginClient::load(
            BridgeConfig::default(),
            path_buf.clone(),
            sample_rate,
        ))?;

        // Apply initial parameter values from params
        // Format: "param_<id>" => value (e.g., "param_0" => 0.5)
        for (key, value) in params {
            if let Some(id_str) = key.strip_prefix("param_") {
                if let Ok(param_id) = id_str.parse::<u32>() {
                    if let Some(param_value) = value.as_f32() {
                        client.set_parameter(param_id, param_value);
                    }
                }
            }
        }

        Ok(Box::new(client))
    });

    Ok(())
}

/// Register all plugins in a directory.
///
/// Scans the directory for .vst, .vst3, and .clap files and registers them.
///
/// # Example
/// ```ignore
/// register_plugin_directory(&registry, runtime.handle(), "/Library/Audio/Plug-Ins/VST3")?;
/// ```
pub fn register_plugin_directory<P: AsRef<Path>>(
    registry: &NodeRegistry,
    runtime: &tokio::runtime::Handle,
    path: P,
) -> Result<Vec<String>, BridgeError> {
    let mut registered = Vec::new();
    let dir_path = path.as_ref();

    if !dir_path.is_dir() {
        return Err(BridgeError::LoadFailed {
            path: dir_path.to_path_buf(),
            stage: LoadStage::Scanning,
            reason: "Not a directory".to_string(),
        });
    }

    // Scan for plugin files
    for entry in std::fs::read_dir(dir_path).map_err(|e| BridgeError::LoadFailed {
        path: dir_path.to_path_buf(),
        stage: LoadStage::Scanning,
        reason: format!("Failed to read directory: {}", e),
    })? {
        let entry = entry.map_err(|e| BridgeError::LoadFailed {
            path: dir_path.to_path_buf(),
            stage: LoadStage::Scanning,
            reason: format!("Failed to read entry: {}", e),
        })?;
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

/// Register all system plugins (VST2, VST3, CLAP).
///
/// Scans all standard plugin directories for the current platform.
pub fn register_all_system_plugins(
    registry: &NodeRegistry,
    runtime: &tokio::runtime::Handle,
) -> Result<Vec<String>, BridgeError> {
    let mut registered = Vec::new();

    for path in get_all_search_paths() {
        if path.exists() {
            match register_plugin_directory(registry, runtime, &path) {
                Ok(mut plugins) => registered.append(&mut plugins),
                Err(e) => tracing::warn!("Failed to scan {}: {}", path.display(), e),
            }
        }
    }

    tracing::info!("Registered {} plugins total", registered.len());
    Ok(registered)
}

/// Check if a path is a plugin file
fn is_plugin_file(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        matches!(ext, "vst" | "vst3" | "clap" | "component")
    } else {
        false
    }
}

/// Get all plugin search paths for the current platform
fn get_all_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let home = std::env::var("HOME").unwrap_or_default();

    #[cfg(target_os = "macos")]
    {
        // VST2
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins/VST"));
        paths.push(PathBuf::from(format!("{}/Library/Audio/Plug-Ins/VST", home)));
        // VST3
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
        paths.push(PathBuf::from(format!(
            "{}/Library/Audio/Plug-Ins/VST3",
            home
        )));
        // CLAP
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins/CLAP"));
        paths.push(PathBuf::from(format!(
            "{}/Library/Audio/Plug-Ins/CLAP",
            home
        )));
    }

    #[cfg(target_os = "windows")]
    {
        // VST2
        paths.push(PathBuf::from("C:\\Program Files\\VstPlugins"));
        paths.push(PathBuf::from("C:\\Program Files\\Common Files\\VST2"));
        paths.push(PathBuf::from("C:\\Program Files (x86)\\VstPlugins"));
        // VST3
        paths.push(PathBuf::from("C:\\Program Files\\Common Files\\VST3"));
        paths.push(PathBuf::from(
            "C:\\Program Files (x86)\\Common Files\\VST3",
        ));
        // CLAP
        paths.push(PathBuf::from("C:\\Program Files\\Common Files\\CLAP"));
        paths.push(PathBuf::from(
            "C:\\Program Files (x86)\\Common Files\\CLAP",
        ));
    }

    #[cfg(target_os = "linux")]
    {
        // VST2
        paths.push(PathBuf::from("/usr/lib/vst"));
        paths.push(PathBuf::from("/usr/local/lib/vst"));
        paths.push(PathBuf::from(format!("{}/.vst", home)));
        // VST3
        paths.push(PathBuf::from("/usr/lib/vst3"));
        paths.push(PathBuf::from("/usr/local/lib/vst3"));
        paths.push(PathBuf::from(format!("{}/.vst3", home)));
        // CLAP
        paths.push(PathBuf::from("/usr/lib/clap"));
        paths.push(PathBuf::from("/usr/local/lib/clap"));
        paths.push(PathBuf::from(format!("{}/.clap", home)));
    }

    paths
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
