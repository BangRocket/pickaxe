mod config;
mod network;
mod player;
mod state;

use config::ServerConfig;
use pickaxe_scripting::ScriptRuntime;
use state::ServerState;
use std::path::Path;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::{error, info};

/// Events sent from connection tasks to the main thread for Lua processing.
#[derive(Debug)]
pub enum ScriptEvent {
    PlayerJoin { name: String },
    PlayerMove { name: String, x: String, y: String, z: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("Starting Pickaxe server...");

    let config = Arc::new(ServerConfig::load(Path::new("config/server.toml"))?);
    info!(
        "Config loaded: bind={}:{}, max_players={}, online_mode={}",
        config.bind, config.port, config.max_players, config.online_mode
    );

    // Initialize Lua scripting (must stay on this thread)
    let scripting = ScriptRuntime::new(&[
        Path::new("lua"),
    ])?;

    // Fire server_start event synchronously
    scripting.fire_event("server_start", &[]);

    let server_state = Arc::new(ServerState::new());
    info!("World generated (flat)");

    // Channel for script events from connection tasks
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<ScriptEvent>();

    let addr = format!("{}:{}", config.bind, config.port);
    let listener = TcpListener::bind(&addr).await?;
    info!("Listening on {}", addr);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((socket, peer)) => {
                        info!("New connection from {}", peer);
                        let config = config.clone();
                        let state = server_state.clone();
                        let tx = event_tx.clone();
                        tokio::spawn(async move {
                            network::handle_connection(socket, config, state, tx).await;
                        });
                    }
                    Err(e) => {
                        error!("Failed to accept connection: {}", e);
                    }
                }
            }
            Some(event) = event_rx.recv() => {
                match event {
                    ScriptEvent::PlayerJoin { name } => {
                        scripting.fire_event("player_join", &[("name", &name)]);
                    }
                    ScriptEvent::PlayerMove { name, x, y, z } => {
                        scripting.fire_event("player_move", &[
                            ("name", &name),
                            ("x", &x),
                            ("y", &y),
                            ("z", &z),
                        ]);
                    }
                }
            }
        }
    }
}
