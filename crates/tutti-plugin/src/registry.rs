use crate::client::PluginClient;
use crate::error::{BridgeError, LoadStage};
use crate::protocol::BridgeConfig;
use std::path::{Path, PathBuf};
use tutti_core::{NodeRegistry, NodeRegistryError, Params};

impl From<BridgeError> for NodeRegistryError {
    fn from(e: BridgeError) -> Self {
        NodeRegistryError::Plugin(e.to_string())
    }
}

/// Params: `sample_rate` (default 44100.0), `param_<id>` for plugin params.
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
        let sample_rate: f64 = p.get_or("sample_rate", 44100.0);

        let (client, _handle) = runtime.block_on(PluginClient::load(
            BridgeConfig::default(),
            path_buf.clone(),
            sample_rate,
        ))?;

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

/// Scans for .vst, .vst3, .clap, .component files.
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
            reason: "not a directory".into(),
        });
    }

    for entry in std::fs::read_dir(dir_path).map_err(|e| BridgeError::LoadFailed {
        path: dir_path.to_path_buf(),
        stage: LoadStage::Scanning,
        reason: e.to_string(),
    })? {
        let entry = entry.map_err(|e| BridgeError::LoadFailed {
            path: dir_path.to_path_buf(),
            stage: LoadStage::Scanning,
            reason: e.to_string(),
        })?;
        let path = entry.path();

        if is_plugin_file(&path) {
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                if register_plugin(registry, runtime, name, &path).is_ok() {
                    registered.push(name.to_string());
                }
            }
        }
    }

    Ok(registered)
}

/// Scans platform-standard plugin directories.
pub fn register_all_system_plugins(
    registry: &NodeRegistry,
    runtime: &tokio::runtime::Handle,
) -> Result<Vec<String>, BridgeError> {
    let mut registered = Vec::new();

    for path in system_plugin_paths() {
        if path.exists() {
            if let Ok(mut plugins) = register_plugin_directory(registry, runtime, &path) {
                registered.append(&mut plugins);
            }
        }
    }

    Ok(registered)
}

fn is_plugin_file(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| matches!(ext, "vst" | "vst3" | "clap" | "component"))
}

fn system_plugin_paths() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut paths = Vec::new();

    #[cfg(target_os = "macos")]
    {
        paths.extend([
            PathBuf::from("/Library/Audio/Plug-Ins/VST"),
            PathBuf::from(format!("{home}/Library/Audio/Plug-Ins/VST")),
            PathBuf::from("/Library/Audio/Plug-Ins/VST3"),
            PathBuf::from(format!("{home}/Library/Audio/Plug-Ins/VST3")),
            PathBuf::from("/Library/Audio/Plug-Ins/CLAP"),
            PathBuf::from(format!("{home}/Library/Audio/Plug-Ins/CLAP")),
        ]);
    }

    #[cfg(target_os = "windows")]
    {
        paths.extend([
            PathBuf::from("C:\\Program Files\\VstPlugins"),
            PathBuf::from("C:\\Program Files\\Common Files\\VST2"),
            PathBuf::from("C:\\Program Files (x86)\\VstPlugins"),
            PathBuf::from("C:\\Program Files\\Common Files\\VST3"),
            PathBuf::from("C:\\Program Files (x86)\\Common Files\\VST3"),
            PathBuf::from("C:\\Program Files\\Common Files\\CLAP"),
            PathBuf::from("C:\\Program Files (x86)\\Common Files\\CLAP"),
        ]);
    }

    #[cfg(target_os = "linux")]
    {
        paths.extend([
            PathBuf::from("/usr/lib/vst"),
            PathBuf::from("/usr/local/lib/vst"),
            PathBuf::from(format!("{home}/.vst")),
            PathBuf::from("/usr/lib/vst3"),
            PathBuf::from("/usr/local/lib/vst3"),
            PathBuf::from(format!("{home}/.vst3")),
            PathBuf::from("/usr/lib/clap"),
            PathBuf::from("/usr/local/lib/clap"),
            PathBuf::from(format!("{home}/.clap")),
        ]);
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
