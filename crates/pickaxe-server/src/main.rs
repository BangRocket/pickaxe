mod bridge;
mod config;
mod ecs;
mod network;
mod tick;

use config::ServerConfig;
use pickaxe_scripting::ScriptRuntime;
use std::path::Path;
use std::sync::atomic::{AtomicI32, AtomicUsize};
use std::sync::{Arc, Mutex};
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

    // Shared entity ID counter
    let next_eid = Arc::new(AtomicI32::new(1));

    // Initialize Lua scripting (must stay on this thread — Lua VM is !Send)
    let scripting = ScriptRuntime::new()?;
    // Shared storage for Lua-registered commands and block overrides
    let lua_commands: bridge::LuaCommands = Arc::new(Mutex::new(Vec::new()));
    let block_overrides: bridge::BlockOverrides = Arc::new(Mutex::new(std::collections::HashMap::new()));
    // Register bridge APIs before mods load so they're available in init.lua
    bridge::register_world_api(scripting.lua())?;
    bridge::register_players_api(scripting.lua())?;
    bridge::register_commands_api(scripting.lua(), lua_commands.clone())?;
    bridge::register_blocks_api(scripting.lua(), block_overrides.clone())?;
    bridge::register_entities_api(scripting.lua(), next_eid.clone())?;
    bridge::register_sounds_api(scripting.lua())?;
    bridge::register_particles_api(scripting.lua())?;
    scripting.load_mods(&[Path::new("lua")])?;

    // Fire server_start event synchronously
    scripting.fire_event("server_start", &[]);

    info!("World generated (flat)");

    // Channel for new players entering play state
    let (new_player_tx, new_player_rx) = mpsc::unbounded_channel::<tick::NewPlayer>();

    // Player count for status responses
    let player_count = Arc::new(AtomicUsize::new(0));

    // TCP listener
    let addr = format!("{}:{}", config.bind, config.port);
    let listener = TcpListener::bind(&addr).await?;
    info!("Listening on {}", addr);

    // Create save channel and spawn saver task
    let world_dir = std::path::PathBuf::from(&config.world_dir);
    let (save_tx, save_rx) = mpsc::unbounded_channel::<tick::SaveOp>();
    let saver_world_dir = world_dir.clone();
    tokio::task::spawn_blocking(move || tick::run_saver_task(save_rx, saver_world_dir));

    // Create region storage for WorldState (read path only).
    // The saver task has its own RegionStorage for writes. This is safe because
    // the read path only loads chunks on first access (cache miss), and once cached
    // they stay in memory. The write path only appends/overwrites on disk.
    let region_dir = world_dir.join("region");
    std::fs::create_dir_all(&region_dir)?;
    let region_storage = pickaxe_region::RegionStorage::new(region_dir)?;

    // Graceful shutdown channel
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let ctrlc_tx = shutdown_tx.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        info!("Received shutdown signal");
        let _ = ctrlc_tx.send(true);
    });

    // Run tick loop and TCP accept loop concurrently on the main task.
    // The tick loop owns the Lua VM (which is !Send), so it must stay on this task.
    // Connection handling is spawned onto the Tokio runtime (those tasks are Send).
    let tick_config = config.clone();
    let tick_player_count = player_count.clone();
    let tick_next_eid = next_eid.clone();

    tokio::select! {
        _ = tick::run_tick_loop(tick_config, scripting, new_player_rx, tick_player_count, lua_commands, block_overrides, tick_next_eid, save_tx, region_storage, shutdown_rx) => {
            info!("Server shut down cleanly");
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
