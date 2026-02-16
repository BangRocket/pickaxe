mod config;
mod ecs;
mod network;
mod tick;

use config::ServerConfig;
use pickaxe_scripting::ScriptRuntime;
use std::path::Path;
use std::sync::atomic::{AtomicI32, AtomicUsize};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::{error, info};

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

    // Initialize Lua scripting (must stay on this thread — Lua VM is !Send)
    let scripting = ScriptRuntime::new()?;
    // Bridge registration goes here (before mod loading)
    scripting.load_mods(&[Path::new("lua")])?;

    // Fire server_start event synchronously
    scripting.fire_event("server_start", &[]);

    info!("World generated (flat)");

    // Channel for new players entering play state
    let (new_player_tx, new_player_rx) = mpsc::unbounded_channel::<tick::NewPlayer>();

    // Shared entity ID counter
    let next_eid = Arc::new(AtomicI32::new(1));

    // Player count for status responses
    let player_count = Arc::new(AtomicUsize::new(0));

    // TCP listener
    let addr = format!("{}:{}", config.bind, config.port);
    let listener = TcpListener::bind(&addr).await?;
    info!("Listening on {}", addr);

    // Run tick loop and TCP accept loop concurrently on the main task.
    // The tick loop owns the Lua VM (which is !Send), so it must stay on this task.
    // Connection handling is spawned onto the Tokio runtime (those tasks are Send).
    let tick_config = config.clone();
    let tick_player_count = player_count.clone();

    tokio::select! {
        _ = tick::run_tick_loop(tick_config, scripting, new_player_rx, tick_player_count) => {
            error!("Tick loop exited unexpectedly");
        }
        _ = accept_loop(listener, config, new_player_tx, next_eid, player_count) => {
            error!("Accept loop exited unexpectedly");
        }
    }

    Ok(())
}

async fn accept_loop(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    new_player_tx: mpsc::UnboundedSender<tick::NewPlayer>,
    next_eid: Arc<AtomicI32>,
    player_count: Arc<AtomicUsize>,
) {
    loop {
        match listener.accept().await {
            Ok((socket, peer)) => {
                info!("New connection from {}", peer);
                let config = config.clone();
                let tx = new_player_tx.clone();
                let eid = next_eid.clone();
                let pc = player_count.clone();
                tokio::spawn(async move {
                    network::handle_connection(
                        socket,
                        config,
                        tx,
                        eid,
                        move || pc.load(std::sync::atomic::Ordering::Relaxed),
                    )
                    .await;
                });
            }
            Err(e) => {
                error!("Failed to accept connection: {}", e);
            }
        }
    }
}
