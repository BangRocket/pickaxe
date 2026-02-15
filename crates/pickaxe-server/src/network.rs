use crate::config::ServerConfig;
use crate::tick::{InboundPacket, NewPlayer};
use anyhow::Result;
use pickaxe_protocol_core::{
    Connection, ConnectionState, ConnectionWriter, InternalPacket, KnownPack,
};
use pickaxe_protocol_v1_21::V1_21Adapter;
use pickaxe_protocol_core::ProtocolAdapter;
use pickaxe_types::GameProfile;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Handle a single client connection through handshake → login → configuration.
/// Once in play state, splits into reader/writer tasks and registers with the tick loop.
pub async fn handle_connection(
    stream: TcpStream,
    config: Arc<ServerConfig>,
    new_player_tx: mpsc::UnboundedSender<NewPlayer>,
    next_eid: Arc<AtomicI32>,
    player_count_fn: impl Fn() -> usize,
) {
    let peer = stream
        .peer_addr()
        .unwrap_or_else(|_| "unknown".parse().unwrap());
    let mut conn = Connection::new(stream);
    let adapter = V1_21Adapter::new();

    if let Err(e) = handle_pre_play(
        &mut conn,
        &adapter,
        &config,
        peer,
        new_player_tx,
        next_eid,
        &player_count_fn,
    )
    .await
    {
        debug!("Connection {} ended: {}", peer, e);
    }
}

async fn handle_pre_play(
    conn: &mut Connection,
    adapter: &V1_21Adapter,
    config: &ServerConfig,
    peer: std::net::SocketAddr,
    new_player_tx: mpsc::UnboundedSender<NewPlayer>,
    next_eid: Arc<AtomicI32>,
    player_count_fn: &impl Fn() -> usize,
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
            debug!(
                "Handshake from {}: protocol={}, next_state={}",
                peer, protocol_version, next_state
            );
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
        Some(ConnectionState::Status) => {
            handle_status(conn, adapter, config, player_count_fn).await
        }
        Some(ConnectionState::Login) => {
            let profile = handle_login(conn, adapter, config).await?;
            handle_configuration(conn, adapter, config).await?;
            enter_play(conn, adapter, profile, new_player_tx, next_eid).await
        }
        _ => Err(anyhow::anyhow!("Invalid next state: {}", next_state)),
    }
}

async fn handle_status(
    conn: &mut Connection,
    adapter: &V1_21Adapter,
    config: &ServerConfig,
    player_count_fn: &impl Fn() -> usize,
) -> Result<()> {
    loop {
        let (id, mut data) = conn.read_packet().await?;
        let packet = adapter.decode_packet(ConnectionState::Status, id, &mut data)?;

        match packet {
            InternalPacket::StatusRequest => {
                let player_count = player_count_fn();
                let response_json = format!(
                    r#"{{"version":{{"name":"1.21.1","protocol":767}},"players":{{"max":{},"online":{}}},"description":{{"text":"{}"}}}}"#,
                    config.max_players, player_count, config.motd
                );
                send_packet(
                    conn,
                    adapter,
                    ConnectionState::Status,
                    &InternalPacket::StatusResponse {
                        json: response_json,
                    },
                )
                .await?;
            }
            InternalPacket::PingRequest { payload } => {
                send_packet(
                    conn,
                    adapter,
                    ConnectionState::Status,
                    &InternalPacket::PongResponse { payload },
                )
                .await?;
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
) -> Result<GameProfile> {
    let (id, mut data) = conn.read_packet().await?;
    let packet = adapter.decode_packet(ConnectionState::Login, id, &mut data)?;

    let (name, client_uuid) = match packet {
        InternalPacket::LoginStart { name, uuid } => {
            info!("Login Start from: {} ({})", name, uuid);
            (name, uuid)
        }
        _ => return Err(anyhow::anyhow!("Expected Login Start")),
    };

    // Enable compression
    let compression_threshold = 256;
    send_packet(
        conn,
        adapter,
        ConnectionState::Login,
        &InternalPacket::SetCompression {
            threshold: compression_threshold,
        },
    )
    .await?;
    conn.enable_compression(compression_threshold);

    // Build profile
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

    send_packet(
        conn,
        adapter,
        ConnectionState::Login,
        &InternalPacket::LoginSuccess {
            profile: profile.clone(),
        },
    )
    .await?;

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
    send_packet(
        conn,
        adapter,
        ConnectionState::Configuration,
        &InternalPacket::KnownPacksRequest {
            packs: vec![KnownPack {
                namespace: "minecraft".into(),
                id: "core".into(),
                version: "1.21".into(),
            }],
        },
    )
    .await?;

    let (id, mut data) = conn.read_packet().await?;
    let packet = adapter.decode_packet(ConnectionState::Configuration, id, &mut data)?;
    match packet {
        InternalPacket::KnownPacksResponse { packs } => {
            debug!("Client knows {} packs", packs.len());
        }
        _ => {
            debug!(
                "Expected Known Packs response, got something else (id=0x{:02X}), continuing",
                id
            );
        }
    }

    let registries = adapter.registry_data();
    for registry_packet in &registries {
        send_packet(
            conn,
            adapter,
            ConnectionState::Configuration,
            registry_packet,
        )
        .await?;
    }

    send_packet(
        conn,
        adapter,
        ConnectionState::Configuration,
        &InternalPacket::FinishConfiguration,
    )
    .await?;

    loop {
        let (id, mut data) = conn.read_packet().await?;
        let packet = adapter.decode_packet(ConnectionState::Configuration, id, &mut data)?;
        match packet {
            InternalPacket::FinishConfigurationAck => {
                debug!("Configuration finished");
                return Ok(());
            }
            InternalPacket::ClientInformation {
                locale,
                view_distance,
                ..
            } => {
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

/// Transition the connection into play state by splitting into reader/writer tasks
/// and registering with the tick loop.
async fn enter_play(
    conn: &mut Connection,
    _adapter: &V1_21Adapter,
    profile: GameProfile,
    new_player_tx: mpsc::UnboundedSender<NewPlayer>,
    next_eid: Arc<AtomicI32>,
) -> Result<()> {
    let entity_id = next_eid.fetch_add(1, Ordering::Relaxed);

    // Channel: tick loop -> writer task (outbound packets)
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<InternalPacket>();

    // Channel: reader task -> tick loop (inbound packets)
    let (in_tx, in_rx) = mpsc::unbounded_channel::<InboundPacket>();

    // Take ownership of the connection internals for split I/O
    let connection = std::mem::replace(conn, Connection::new_dummy());

    // Register with the tick loop
    let _ = new_player_tx.send(NewPlayer {
        entity_id,
        profile: profile.clone(),
        packet_tx: out_tx,
        packet_rx: in_rx,
    });

    // Split the connection into reader and writer halves
    let (reader, writer) = connection.into_split();

    let player_name = profile.name.clone();

    // Writer task: reads packets from channel, encodes and sends them
    let write_adapter = V1_21Adapter::new();
    let writer_handle = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(packet) = out_rx.recv().await {
            if let Err(e) = encode_and_send(&mut writer, &write_adapter, &packet).await {
                debug!("Writer error for {}: {}", player_name, e);
                break;
            }
        }
    });

    // Reader task: reads packets from TCP, decodes and forwards to tick loop
    let read_adapter = V1_21Adapter::new();
    let reader_name = profile.name.clone();
    let _reader_result = async {
        let mut reader = reader;
        loop {
            match reader.read_packet().await {
                Ok((id, mut data)) => {
                    match read_adapter.decode_packet(ConnectionState::Play, id, &mut data) {
                        Ok(packet) => {
                            if in_tx
                                .send(InboundPacket {
                                    entity_id,
                                    packet,
                                })
                                .is_err()
                            {
                                break; // Tick loop shut down
                            }
                        }
                        Err(e) => {
                            debug!("Decode error for {}: {}", reader_name, e);
                        }
                    }
                }
                Err(e) => {
                    debug!("Reader error for {}: {}", reader_name, e);
                    break;
                }
            }
        }
    }
    .await;

    // Reader finished = client disconnected. Drop the inbound sender, which
    // will cause the tick loop to detect disconnection on the next tick.
    drop(in_tx);

    // Wait for writer to finish flushing
    writer_handle.abort();

    Ok(())
}

async fn encode_and_send(
    writer: &mut ConnectionWriter,
    adapter: &V1_21Adapter,
    packet: &InternalPacket,
) -> Result<()> {
    let encoded = adapter.encode_packet(ConnectionState::Play, packet)?;
    let mut data = encoded;
    let packet_id = pickaxe_protocol_core::read_varint(&mut data)?;
    writer.write_packet(packet_id, &data).await
}

/// Send an InternalPacket using the adapter's encode.
async fn send_packet(
    conn: &mut Connection,
    adapter: &V1_21Adapter,
    state: ConnectionState,
    packet: &InternalPacket,
) -> Result<()> {
    let encoded = adapter.encode_packet(state, packet)?;
    let mut data = encoded;
    let packet_id = pickaxe_protocol_core::read_varint(&mut data)?;
    conn.write_packet(packet_id, &data).await
}

/// Generate an offline-mode UUID from a player name.
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
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

