//! Load and process audio through a VST2 plugin

use std::path::PathBuf;
use tutti_core::AudioUnit;
use tutti_plugin::protocol::BridgeConfig;
use tutti_plugin::PluginClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let plugin_path = std::env::args()
        .nth(1)
        .expect("Usage: vst2_host <plugin.vst>");

    let (mut client, _handle) = PluginClient::load(
        BridgeConfig::default(),
        PathBuf::from(plugin_path),
        44100.0,
    )
    .await?;

    let ins = <PluginClient as AudioUnit>::inputs(&client);
    let outs = <PluginClient as AudioUnit>::outputs(&client);
    println!("VST2 plugin loaded ({} in, {} out)", ins, outs);

    for _ in 0..1000 {
        let input = vec![0.0f32; ins];
        let mut output = vec![0.0f32; outs];
        <PluginClient as AudioUnit>::tick(&mut client, &input, &mut output);
    }

    println!("Processed 1000 samples");
    Ok(())
}
