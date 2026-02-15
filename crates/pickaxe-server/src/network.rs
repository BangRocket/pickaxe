use crate::config::ServerConfig;
use crate::ScriptEvent;
use anyhow::Result;
use bytes::BytesMut;
use pickaxe_protocol_core::{Connection, ConnectionState, InternalPacket, KnownPack, write_varint};
use pickaxe_protocol_v1_21::V1_21Adapter;
use pickaxe_protocol_core::ProtocolAdapter;
use pickaxe_types::{GameMode, GameProfile, TextComponent, Vec3d, BlockPos};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::player::PlayerHandle;

/// Handle a single client connection through the entire protocol lifecycle.
pub async fn handle_connection(
    stream: TcpStream,
    config: Arc<ServerConfig>,
    server_state: Arc<crate::ServerState>,
    event_tx: mpsc::UnboundedSender<ScriptEvent>,
) {
    let peer = stream.peer_addr().unwrap_or_else(|_| "unknown".parse().unwrap());
    let mut conn = Connection::new(stream);
    let adapter = V1_21Adapter::new();

    if let Err(e) = handle_connection_inner(&mut conn, &adapter, &config, &server_state, peer, &event_tx).await {
        debug!("Connection {} ended: {}", peer, e);
    }
}

async fn handle_connection_inner(
    conn: &mut Connection,
    adapter: &V1_21Adapter,
    config: &ServerConfig,
    server_state: &Arc<crate::ServerState>,
    peer: std::net::SocketAddr,
    event_tx: &mpsc::UnboundedSender<ScriptEvent>,
) -> Result<()> {
    // === Handshake ===
    let (id, mut data) = conn.read_packet().await?;
    let packet = adapter.decode_packet(ConnectionState::Handshaking, id, &mut data)?;

    let next_state = match packet {
        InternalPacket::Handshake {
            protocol_version,
            next_state,
            ..
        } => {
            debug!("Handshake from {}: protocol={}, next_state={}", peer, protocol_version, next_state);
            if protocol_version != adapter.protocol_version() {
                warn!(
                    "Client {} has protocol version {}, expected {}",
                    peer,
                    protocol_version,
                    adapter.protocol_version()
                );
            }
            next_state
        }
        _ => return Err(anyhow::anyhow!("Expected handshake packet")),
    };

    match ConnectionState::from_handshake_next(next_state) {
        Some(ConnectionState::Status) => handle_status(conn, adapter, config, server_state).await,
        Some(ConnectionState::Login) => {
            let profile = handle_login(conn, adapter, config, server_state).await?;
            handle_configuration(conn, adapter, config).await?;
            handle_play(conn, adapter, config, server_state, profile, event_tx).await
        }
        _ => Err(anyhow::anyhow!("Invalid next state: {}", next_state)),
    }
}

async fn handle_status(
    conn: &mut Connection,
    adapter: &V1_21Adapter,
    config: &ServerConfig,
    server_state: &Arc<crate::ServerState>,
) -> Result<()> {
    loop {
        let (id, mut data) = conn.read_packet().await?;
        let packet = adapter.decode_packet(ConnectionState::Status, id, &mut data)?;

        match packet {
            InternalPacket::StatusRequest => {
                let player_count = server_state.player_count();
                let response_json = format!(
                    r#"{{"version":{{"name":"1.21.1","protocol":767}},"players":{{"max":{},"online":{}}},"description":{{"text":"{}"}}}}"#,
                    config.max_players, player_count, config.motd
                );
                send_packet(conn, adapter, ConnectionState::Status,
                    &InternalPacket::StatusResponse { json: response_json }).await?;
            }
            InternalPacket::PingRequest { payload } => {
                send_packet(conn, adapter, ConnectionState::Status,
                    &InternalPacket::PongResponse { payload }).await?;
                return Ok(());
            }
            _ => {}
        }
    }
}

async fn handle_login(
    conn: &mut Connection,
    adapter: &V1_21Adapter,
    config: &ServerConfig,
    _server_state: &Arc<crate::ServerState>,
) -> Result<GameProfile> {
    // Wait for Login Start
    let (id, mut data) = conn.read_packet().await?;
    let packet = adapter.decode_packet(ConnectionState::Login, id, &mut data)?;

    let (name, client_uuid) = match packet {
        InternalPacket::LoginStart { name, uuid } => {
            info!("Login Start from: {} ({})", name, uuid);
            (name, uuid)
        }
        _ => return Err(anyhow::anyhow!("Expected Login Start")),
    };

    // For now: offline mode only — skip encryption
    // TODO: online mode with RSA + Mojang session server

    // Enable compression
    let compression_threshold = 256;
    send_packet(conn, adapter, ConnectionState::Login,
        &InternalPacket::SetCompression { threshold: compression_threshold }).await?;
    conn.enable_compression(compression_threshold);

    // Build profile (offline mode: generate UUID from name)
    let uuid = if config.online_mode {
        client_uuid
    } else {
        offline_uuid(&name)
    };

    let profile = GameProfile {
        uuid,
        name: name.clone(),
        properties: Vec::new(),
    };

    // Send Login Success
    send_packet(conn, adapter, ConnectionState::Login,
        &InternalPacket::LoginSuccess { profile: profile.clone() }).await?;

    // Wait for Login Acknowledged
    let (id, mut data) = conn.read_packet().await?;
    let ack = adapter.decode_packet(ConnectionState::Login, id, &mut data)?;
    match ack {
        InternalPacket::LoginAcknowledged => {
            debug!("Login acknowledged by {}", name);
        }
        _ => return Err(anyhow::anyhow!("Expected Login Acknowledged")),
    }

    Ok(profile)
}

async fn handle_configuration(
    conn: &mut Connection,
    adapter: &V1_21Adapter,
    _config: &ServerConfig,
) -> Result<()> {
    // Send Known Packs request (empty — we don't have any)
    send_packet(conn, adapter, ConnectionState::Configuration,
        &InternalPacket::KnownPacksRequest { packs: vec![
            KnownPack {
                namespace: "minecraft".into(),
                id: "core".into(),
                version: "1.21".into(),
            }
        ] }).await?;

    // Wait for Known Packs response
    let (id, mut data) = conn.read_packet().await?;
    let packet = adapter.decode_packet(ConnectionState::Configuration, id, &mut data)?;
    match packet {
        InternalPacket::KnownPacksResponse { packs } => {
            debug!("Client knows {} packs", packs.len());
        }
        _ => {
            debug!("Expected Known Packs response, got something else (id=0x{:02X}), continuing", id);
        }
    }

    // Send all registry data
    let registries = adapter.registry_data();
    for registry_packet in &registries {
        send_packet(conn, adapter, ConnectionState::Configuration, registry_packet).await?;
    }

    // Send Finish Configuration
    send_packet(conn, adapter, ConnectionState::Configuration,
        &InternalPacket::FinishConfiguration).await?;

    // Read until we get Finish Configuration Ack (client may send Client Information, Plugin Messages first)
    loop {
        let (id, mut data) = conn.read_packet().await?;
        let packet = adapter.decode_packet(ConnectionState::Configuration, id, &mut data)?;
        match packet {
            InternalPacket::FinishConfigurationAck => {
                debug!("Configuration finished");
                return Ok(());
            }
            InternalPacket::ClientInformation { locale, view_distance, .. } => {
                debug!("Client info: locale={}, view_distance={}", locale, view_distance);
            }
            InternalPacket::PluginMessage { channel, .. } => {
                debug!("Plugin message: {}", channel);
            }
            _ => {
                debug!("Ignoring config packet id=0x{:02X}", id);
            }
        }
    }
}

async fn handle_play(
    conn: &mut Connection,
    adapter: &V1_21Adapter,
    config: &ServerConfig,
    server_state: &Arc<crate::ServerState>,
    profile: GameProfile,
    event_tx: &mpsc::UnboundedSender<ScriptEvent>,
) -> Result<()> {
    let player_name = profile.name.clone();
    info!("{} entering Play state", player_name);

    let entity_id = server_state.next_entity_id();
    server_state.add_player(entity_id, &profile);

    // Fire player_join event
    let _ = event_tx.send(ScriptEvent::PlayerJoin {
        name: player_name.clone(),
    });

    let result = handle_play_inner(conn, adapter, config, server_state, &profile, entity_id, event_tx).await;

    server_state.remove_player(entity_id);
    info!("{} disconnected", player_name);

    result
}

async fn handle_play_inner(
    conn: &mut Connection,
    adapter: &V1_21Adapter,
    config: &ServerConfig,
    server_state: &Arc<crate::ServerState>,
    profile: &GameProfile,
    entity_id: i32,
    event_tx: &mpsc::UnboundedSender<ScriptEvent>,
) -> Result<()> {
    let view_distance = config.view_distance as i32;

    // Send Join Game
    send_packet(conn, adapter, ConnectionState::Play, &InternalPacket::JoinGame {
        entity_id,
        is_hardcore: false,
        dimension_names: vec!["minecraft:overworld".into()],
        max_players: config.max_players as i32,
        view_distance,
        simulation_distance: view_distance,
        reduced_debug_info: false,
        enable_respawn_screen: true,
        do_limited_crafting: false,
        dimension_type: 0,
        dimension_name: "minecraft:overworld".into(),
        hashed_seed: 0,
        game_mode: GameMode::Creative,
        previous_game_mode: -1,
        is_debug: false,
        is_flat: true,
        portal_cooldown: 0,
        enforces_secure_chat: false,
    }).await?;

    // Compute spawn position: flat world top block is grass_block at y=-60
    // So spawn on top of grass at y=-59 (feet position)
    let spawn_y = -59.0;
    let spawn_pos = Vec3d::new(0.5, spawn_y, 0.5);

    // Set center chunk
    let center_cx = (spawn_pos.x as i32) >> 4;
    let center_cz = (spawn_pos.z as i32) >> 4;
    send_packet(conn, adapter, ConnectionState::Play, &InternalPacket::SetCenterChunk {
        chunk_x: center_cx,
        chunk_z: center_cz,
    }).await?;

    // Send chunks
    send_chunks_around(conn, adapter, center_cx, center_cz, view_distance, server_state).await?;

    // Synchronize player position
    let teleport_id = 1;
    send_packet(conn, adapter, ConnectionState::Play, &InternalPacket::SynchronizePlayerPosition {
        position: spawn_pos,
        yaw: 0.0,
        pitch: 0.0,
        flags: 0,
        teleport_id,
    }).await?;

    // Send Game Event: Start waiting for level chunks (event 13, value 0)
    send_packet(conn, adapter, ConnectionState::Play, &InternalPacket::GameEvent {
        event: 13,
        value: 0.0,
    }).await?;

    // Send default spawn position (must come after GameEvent to avoid NPE)
    send_packet(conn, adapter, ConnectionState::Play, &InternalPacket::SetDefaultSpawnPosition {
        position: BlockPos::new(0, -60, 0),
        angle: 0.0,
    }).await?;

    // Enter the play loop
    let mut player = PlayerHandle::new(
        entity_id,
        profile.clone(),
        spawn_pos,
        0.0,
        0.0,
        center_cx,
        center_cz,
        view_distance,
    );

    // Keep-alive tracking
    let mut last_keep_alive = std::time::Instant::now();
    let mut pending_keep_alive: Option<i64> = None;
    let mut last_keep_alive_sent = std::time::Instant::now();

    loop {
        tokio::select! {
            result = conn.read_packet() => {
                let (id, mut data) = result?;
                let packet = adapter.decode_packet(ConnectionState::Play, id, &mut data)?;

                match packet {
                    InternalPacket::ConfirmTeleportation { teleport_id: tid } => {
                        debug!("Teleport confirmed: {}", tid);
                    }
                    InternalPacket::PlayerPosition { x, y, z, on_ground } => {
                        player.update_position(x, y, z, on_ground);
                        let _ = event_tx.send(ScriptEvent::PlayerMove {
                            name: profile.name.clone(),
                            x: format!("{:.1}", x),
                            y: format!("{:.1}", y),
                            z: format!("{:.1}", z),
                        });
                        handle_chunk_updates(conn, adapter, &mut player, server_state).await?;
                    }
                    InternalPacket::PlayerPositionAndRotation { x, y, z, yaw, pitch, on_ground } => {
                        player.update_position_and_rotation(x, y, z, yaw, pitch, on_ground);
                        let _ = event_tx.send(ScriptEvent::PlayerMove {
                            name: profile.name.clone(),
                            x: format!("{:.1}", x),
                            y: format!("{:.1}", y),
                            z: format!("{:.1}", z),
                        });
                        handle_chunk_updates(conn, adapter, &mut player, server_state).await?;
                    }
                    InternalPacket::PlayerRotation { yaw, pitch, on_ground } => {
                        player.update_rotation(yaw, pitch, on_ground);
                    }
                    InternalPacket::PlayerOnGround { on_ground } => {
                        player.on_ground = on_ground;
                    }
                    InternalPacket::KeepAliveServerbound { id: ka_id } => {
                        if pending_keep_alive == Some(ka_id) {
                            pending_keep_alive = None;
                            last_keep_alive = std::time::Instant::now();
                        }
                    }
                    InternalPacket::BlockDig { status, position, sequence, .. } => {
                        // status 0 = started digging (instant break in creative mode)
                        if status == 0 {
                            let old_block = server_state.set_block(&position, 0); // air
                            // BlockUpdate MUST come before AcknowledgeBlockChange —
                            // the ack tells the client "apply server-sent state", so
                            // the server state must arrive first.
                            send_packet(conn, adapter, ConnectionState::Play,
                                &InternalPacket::BlockUpdate { position, block_id: 0 }).await?;
                            send_packet(conn, adapter, ConnectionState::Play,
                                &InternalPacket::AcknowledgeBlockChange { sequence }).await?;
                            debug!("{} broke block at {:?} (was {})", profile.name, position, old_block);
                            let _ = event_tx.send(ScriptEvent::BlockBreak {
                                name: profile.name.clone(),
                                x: position.x,
                                y: position.y,
                                z: position.z,
                                block_id: old_block,
                            });
                        }
                    }
                    InternalPacket::BlockPlace { position, face, sequence, .. } => {
                        let target = offset_by_face(&position, face);
                        // Place stone for now (no inventory tracking)
                        let block_id = 1; // stone
                        server_state.set_block(&target, block_id);
                        send_packet(conn, adapter, ConnectionState::Play,
                            &InternalPacket::BlockUpdate { position: target, block_id }).await?;
                        send_packet(conn, adapter, ConnectionState::Play,
                            &InternalPacket::AcknowledgeBlockChange { sequence }).await?;
                        debug!("{} placed block at {:?}", profile.name, target);
                        let _ = event_tx.send(ScriptEvent::BlockPlace {
                            name: profile.name.clone(),
                            x: target.x,
                            y: target.y,
                            z: target.z,
                            block_id,
                        });
                    }
                    InternalPacket::Unknown { packet_id, .. } => {
                        // Silently ignore unknown packets
                        let _ = packet_id;
                    }
                    _ => {}
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                // Periodic tasks
                let now = std::time::Instant::now();

                // Send keep-alive every 15 seconds
                if now.duration_since(last_keep_alive_sent).as_secs() >= 15 {
                    let ka_id = now.elapsed().as_millis() as i64;
                    send_packet(conn, adapter, ConnectionState::Play,
                        &InternalPacket::KeepAliveClientbound { id: ka_id }).await?;
                    pending_keep_alive = Some(ka_id);
                    last_keep_alive_sent = now;
                }

                // Kick if no keep-alive response for 30 seconds
                if pending_keep_alive.is_some()
                    && now.duration_since(last_keep_alive).as_secs() >= 30
                {
                    send_packet(conn, adapter, ConnectionState::Play,
                        &InternalPacket::Disconnect {
                            reason: TextComponent::plain("Timed out"),
                        }).await?;
                    return Err(anyhow::anyhow!("Keep-alive timeout"));
                }
            }
        }
    }
}

async fn send_chunks_around(
    conn: &mut Connection,
    adapter: &V1_21Adapter,
    center_cx: i32,
    center_cz: i32,
    view_distance: i32,
    server_state: &Arc<crate::ServerState>,
) -> Result<()> {
    // Chunk Batch Start: 0x0D, empty
    send_packet_raw(conn, adapter, ConnectionState::Play, 0x0D, &[]).await?;

    let mut batch_size = 0i32;
    for cx in (center_cx - view_distance)..=(center_cx + view_distance) {
        for cz in (center_cz - view_distance)..=(center_cz + view_distance) {
            let chunk_packet = server_state.get_chunk_packet(cx, cz);
            send_packet(conn, adapter, ConnectionState::Play, &chunk_packet).await?;
            batch_size += 1;
        }
    }

    // Chunk Batch Finished: 0x0C, VarInt batch_size
    let mut finish_buf = BytesMut::new();
    write_varint(&mut finish_buf, batch_size);
    send_packet_raw(conn, adapter, ConnectionState::Play, 0x0C, &finish_buf).await?;

    Ok(())
}

async fn handle_chunk_updates(
    conn: &mut Connection,
    adapter: &V1_21Adapter,
    player: &mut PlayerHandle,
    server_state: &Arc<crate::ServerState>,
) -> Result<()> {
    let new_cx = (player.position.x as i32) >> 4;
    let new_cz = (player.position.z as i32) >> 4;

    if new_cx != player.chunk_x || new_cz != player.chunk_z {
        let old_cx = player.chunk_x;
        let old_cz = player.chunk_z;
        player.chunk_x = new_cx;
        player.chunk_z = new_cz;

        // Send Set Center Chunk
        send_packet(conn, adapter, ConnectionState::Play, &InternalPacket::SetCenterChunk {
            chunk_x: new_cx,
            chunk_z: new_cz,
        }).await?;

        let vd = player.view_distance;

        // Unload old chunks that are now out of range
        for cx in (old_cx - vd)..=(old_cx + vd) {
            for cz in (old_cz - vd)..=(old_cz + vd) {
                if (cx - new_cx).abs() > vd || (cz - new_cz).abs() > vd {
                    send_packet(conn, adapter, ConnectionState::Play, &InternalPacket::UnloadChunk {
                        chunk_x: cx,
                        chunk_z: cz,
                    }).await?;
                }
            }
        }

        // Chunk Batch Start: 0x0D, empty
        send_packet_raw(conn, adapter, ConnectionState::Play, 0x0D, &[]).await?;

        // Load new chunks that are now in range
        let mut batch_size = 0i32;
        for cx in (new_cx - vd)..=(new_cx + vd) {
            for cz in (new_cz - vd)..=(new_cz + vd) {
                if (cx - old_cx).abs() > vd || (cz - old_cz).abs() > vd {
                    let chunk_packet = server_state.get_chunk_packet(cx, cz);
                    send_packet(conn, adapter, ConnectionState::Play, &chunk_packet).await?;
                    batch_size += 1;
                }
            }
        }

        // Chunk Batch Finished: 0x0C, VarInt batch_size
        let mut finish_buf = BytesMut::new();
        write_varint(&mut finish_buf, batch_size);
        send_packet_raw(conn, adapter, ConnectionState::Play, 0x0C, &finish_buf).await?;
    }

    Ok(())
}

/// Send an InternalPacket using the adapter's encode.
async fn send_packet(
    conn: &mut Connection,
    adapter: &V1_21Adapter,
    state: ConnectionState,
    packet: &InternalPacket,
) -> Result<()> {
    let encoded = adapter.encode_packet(state, packet)?;
    // The encoded data includes the packet ID as a varint prefix, then payload.
    // Connection::write_packet expects (packet_id, payload) separately.
    // So we need to split them.
    let mut data = encoded;
    let packet_id = pickaxe_protocol_core::read_varint(&mut data)?;
    conn.write_packet(packet_id, &data).await
}

/// Send a raw packet with known ID and payload.
async fn send_packet_raw(
    conn: &mut Connection,
    _adapter: &V1_21Adapter,
    _state: ConnectionState,
    packet_id: i32,
    payload: &[u8],
) -> Result<()> {
    conn.write_packet(packet_id, payload).await
}

/// Offset a block position by the given face direction.
/// Face: 0=bottom(y-1), 1=top(y+1), 2=north(z-1), 3=south(z+1), 4=west(x-1), 5=east(x+1)
fn offset_by_face(pos: &BlockPos, face: u8) -> BlockPos {
    match face {
        0 => BlockPos::new(pos.x, pos.y - 1, pos.z),
        1 => BlockPos::new(pos.x, pos.y + 1, pos.z),
        2 => BlockPos::new(pos.x, pos.y, pos.z - 1),
        3 => BlockPos::new(pos.x, pos.y, pos.z + 1),
        4 => BlockPos::new(pos.x - 1, pos.y, pos.z),
        5 => BlockPos::new(pos.x + 1, pos.y, pos.z),
        _ => *pos,
    }
}

/// Generate an offline-mode UUID from a player name (MD5 hash, version 3 style).
fn offline_uuid(name: &str) -> Uuid {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let input = format!("OfflinePlayer:{}", name);
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    let h1 = hasher.finish();
    input.len().hash(&mut hasher);
    let h2 = hasher.finish();
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&h1.to_be_bytes());
    bytes[8..].copy_from_slice(&h2.to_be_bytes());
    // Set version 3 and variant bits
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}
