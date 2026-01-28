//! Plugin server process entry point
//!
//! This binary is spawned by the main process to host plugins in isolation.

use std::env;
use tutti_plugin::{BridgeConfig, PluginServer, Result};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    tracing::info!("Plugin bridge starting...");

    // Parse config from args (TODO: better CLI parsing)
    let socket_path = env::args()
        .nth(1)
        .expect("Socket path required as first argument");

    let config = BridgeConfig {
        socket_path: socket_path.into(),
        ..Default::default()
    };

    // Create and run server
    let mut server = PluginServer::new(config).await?;

    tracing::info!("Bridge ready, waiting for connection...");

    server.run().await?;

    tracing::info!("Bridge shutting down");

    Ok(())
}
