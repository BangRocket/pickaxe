use anyhow::{bail, Result};
use bytes::{Buf, BufMut, BytesMut};
use pickaxe_nbt::NbtValue;
use pickaxe_protocol_core::*;

use crate::registries;

pub struct V1_21Adapter;

impl V1_21Adapter {
    pub fn new() -> Self {
        Self
    }
}

impl ProtocolAdapter for V1_21Adapter {
    fn protocol_version(&self) -> i32 {
        767
    }

    fn decode_packet(
        &self,
        state: ConnectionState,
        id: i32,
        data: &mut BytesMut,
    ) -> Result<InternalPacket> {
        match state {
            ConnectionState::Handshaking => decode_handshaking(id, data),
            ConnectionState::Status => decode_status(id, data),
            ConnectionState::Login => decode_login(id, data),
            ConnectionState::Configuration => decode_configuration(id, data),
            ConnectionState::Play => decode_play(id, data),
        }
    }

    fn encode_packet(
        &self,
        state: ConnectionState,
        packet: &InternalPacket,
    ) -> Result<BytesMut> {
        match state {
            ConnectionState::Status => encode_status(packet),
            ConnectionState::Login => encode_login(packet),
            ConnectionState::Configuration => encode_configuration(packet),
            ConnectionState::Play => encode_play(packet),
            _ => bail!("Cannot encode packets in {:?} state", state),
        }
    }

    fn registry_data(&self) -> Vec<InternalPacket> {
        registries::build_registry_packets()
    }
}

// === Packet ID constants ===

// Status
const STATUS_RESPONSE: i32 = 0x00;
const PONG_RESPONSE: i32 = 0x01;

// Login clientbound
const LOGIN_DISCONNECT: i32 = 0x00;
const ENCRYPTION_REQUEST: i32 = 0x01;
const LOGIN_SUCCESS: i32 = 0x02;
const SET_COMPRESSION: i32 = 0x03;

// Configuration clientbound
const CONFIG_FINISH: i32 = 0x03;
const CONFIG_REGISTRY_DATA: i32 = 0x07;
const CONFIG_KNOWN_PACKS: i32 = 0x0E;

// Play clientbound
const PLAY_DISCONNECT: i32 = 0x1D;
const PLAY_UNLOAD_CHUNK: i32 = 0x21;
const PLAY_GAME_EVENT: i32 = 0x22;
const PLAY_KEEP_ALIVE: i32 = 0x26;
const PLAY_CHUNK_DATA: i32 = 0x27;
const PLAY_LOGIN: i32 = 0x2B;
const PLAY_SYNC_PLAYER_POS: i32 = 0x40;
const PLAY_SET_CENTER_CHUNK: i32 = 0x54;
const PLAY_SET_DEFAULT_SPAWN: i32 = 0x56;

// === Decode functions ===

fn decode_handshaking(id: i32, data: &mut BytesMut) -> Result<InternalPacket> {
    match id {
        0x00 => {
            let protocol_version = read_varint(data)?;
            let server_address = read_string(data, 255)?;
            let server_port = data.get_u16();
            let next_state = read_varint(data)?;
            Ok(InternalPacket::Handshake {
                protocol_version,
                server_address,
                server_port,
                next_state,
            })
        }
        _ => Ok(InternalPacket::Unknown {
            packet_id: id,
            data: data.to_vec(),
        }),
    }
}

fn decode_status(id: i32, data: &mut BytesMut) -> Result<InternalPacket> {
    match id {
        0x00 => Ok(InternalPacket::StatusRequest),
        0x01 => {
            let payload = data.get_i64();
            Ok(InternalPacket::PingRequest { payload })
        }
        _ => Ok(InternalPacket::Unknown {
            packet_id: id,
            data: data.to_vec(),
        }),
    }
}

fn decode_login(id: i32, data: &mut BytesMut) -> Result<InternalPacket> {
    match id {
        0x00 => {
            let name = read_string(data, 16)?;
            let uuid = read_uuid(data)?;
            Ok(InternalPacket::LoginStart { name, uuid })
        }
        0x01 => {
            let shared_secret = read_byte_array(data)?;
            let verify_token = read_byte_array(data)?;
            Ok(InternalPacket::EncryptionResponse {
                shared_secret,
                verify_token,
            })
        }
        0x03 => Ok(InternalPacket::LoginAcknowledged),
        _ => Ok(InternalPacket::Unknown {
            packet_id: id,
            data: data.to_vec(),
        }),
    }
}

fn decode_configuration(id: i32, data: &mut BytesMut) -> Result<InternalPacket> {
    match id {
        0x00 => {
            let locale = read_string(data, 16)?;
            let view_distance = data.get_i8();
            let chat_mode = read_varint(data)?;
            let chat_colors = data.get_u8() != 0;
            let skin_parts = data.get_u8();
            let main_hand = read_varint(data)?;
            let text_filtering = data.get_u8() != 0;
            let allow_listing = data.get_u8() != 0;
            Ok(InternalPacket::ClientInformation {
                locale,
                view_distance,
                chat_mode,
                chat_colors,
                skin_parts,
                main_hand,
                text_filtering,
                allow_listing,
            })
        }
        0x02 => {
            let channel = read_string(data, 32767)?;
            let remaining = data.to_vec();
            data.advance(remaining.len());
            Ok(InternalPacket::PluginMessage {
                channel,
                data: remaining,
            })
        }
        0x03 => Ok(InternalPacket::FinishConfigurationAck),
        0x07 => {
            let count = read_varint(data)? as usize;
            let mut packs = Vec::with_capacity(count);
            for _ in 0..count {
                let namespace = read_string(data, 32767)?;
                let id = read_string(data, 32767)?;
                let version = read_string(data, 32767)?;
                packs.push(KnownPack {
                    namespace,
                    id,
                    version,
                });
            }
            Ok(InternalPacket::KnownPacksResponse { packs })
        }
        _ => Ok(InternalPacket::Unknown {
            packet_id: id,
            data: data.to_vec(),
        }),
    }
}

fn decode_play(id: i32, data: &mut BytesMut) -> Result<InternalPacket> {
    match id {
        0x00 => {
            let teleport_id = read_varint(data)?;
            Ok(InternalPacket::ConfirmTeleportation { teleport_id })
        }
        0x08 => {
            // Chunk Batch Received — just acknowledge, read the chunks_per_tick float
            let _chunks_per_tick = data.get_f32();
            Ok(InternalPacket::Unknown {
                packet_id: id,
                data: vec![],
            })
        }
        0x18 => {
            let id = data.get_i64();
            Ok(InternalPacket::KeepAliveServerbound { id })
        }
        0x1A => {
            let x = data.get_f64();
            let y = data.get_f64();
            let z = data.get_f64();
            let on_ground = data.get_u8() != 0;
            Ok(InternalPacket::PlayerPosition { x, y, z, on_ground })
        }
        0x1B => {
            let x = data.get_f64();
            let y = data.get_f64();
            let z = data.get_f64();
            let yaw = data.get_f32();
            let pitch = data.get_f32();
            let on_ground = data.get_u8() != 0;
            Ok(InternalPacket::PlayerPositionAndRotation {
                x,
                y,
                z,
                yaw,
                pitch,
                on_ground,
            })
        }
        0x1C => {
            let yaw = data.get_f32();
            let pitch = data.get_f32();
            let on_ground = data.get_u8() != 0;
            Ok(InternalPacket::PlayerRotation {
                yaw,
                pitch,
                on_ground,
            })
        }
        0x1D => {
            let on_ground = data.get_u8() != 0;
            Ok(InternalPacket::PlayerOnGround { on_ground })
        }
        _ => Ok(InternalPacket::Unknown {
            packet_id: id,
            data: data.to_vec(),
        }),
    }
}

// === Encode functions ===

fn encode_status(packet: &InternalPacket) -> Result<BytesMut> {
    let mut buf = BytesMut::new();
    match packet {
        InternalPacket::StatusResponse { json } => {
            write_varint(&mut buf, STATUS_RESPONSE);
            write_string(&mut buf, json);
        }
        InternalPacket::PongResponse { payload } => {
            write_varint(&mut buf, PONG_RESPONSE);
            buf.put_i64(*payload);
        }
        _ => bail!("Cannot encode {:?} in Status state", std::mem::discriminant(packet)),
    }
    Ok(buf)
}

fn encode_login(packet: &InternalPacket) -> Result<BytesMut> {
    let mut buf = BytesMut::new();
    match packet {
        InternalPacket::Disconnect { reason } => {
            write_varint(&mut buf, LOGIN_DISCONNECT);
            write_string(&mut buf, &reason.to_json());
        }
        InternalPacket::EncryptionRequest {
            server_id,
            public_key,
            verify_token,
        } => {
            write_varint(&mut buf, ENCRYPTION_REQUEST);
            write_string(&mut buf, server_id);
            write_byte_array(&mut buf, public_key);
            write_byte_array(&mut buf, verify_token);
            buf.put_u8(1); // should authenticate = true
        }
        InternalPacket::LoginSuccess { profile } => {
            write_varint(&mut buf, LOGIN_SUCCESS);
            write_uuid(&mut buf, &profile.uuid);
            write_string(&mut buf, &profile.name);
            write_varint(&mut buf, profile.properties.len() as i32);
            for prop in &profile.properties {
                write_string(&mut buf, &prop.name);
                write_string(&mut buf, &prop.value);
                if let Some(ref sig) = prop.signature {
                    buf.put_u8(1);
                    write_string(&mut buf, sig);
                } else {
                    buf.put_u8(0);
                }
            }
            buf.put_u8(0); // strict error handling = false
        }
        InternalPacket::SetCompression { threshold } => {
            write_varint(&mut buf, SET_COMPRESSION);
            write_varint(&mut buf, *threshold);
        }
        _ => bail!("Cannot encode {:?} in Login state", std::mem::discriminant(packet)),
    }
    Ok(buf)
}

fn encode_configuration(packet: &InternalPacket) -> Result<BytesMut> {
    let mut buf = BytesMut::new();
    match packet {
        InternalPacket::RegistryData { registry_id, entries } => {
            write_varint(&mut buf, CONFIG_REGISTRY_DATA);
            write_string(&mut buf, registry_id);
            write_varint(&mut buf, entries.len() as i32);
            for entry in entries {
                write_string(&mut buf, &entry.id);
                if let Some(ref nbt_data) = entry.data {
                    buf.put_u8(1); // has data
                    let mut nbt_buf = BytesMut::new();
                    nbt_data.write_root_network(&mut nbt_buf);
                    buf.extend_from_slice(&nbt_buf);
                } else {
                    buf.put_u8(0);
                }
            }
        }
        InternalPacket::FinishConfiguration => {
            write_varint(&mut buf, CONFIG_FINISH);
        }
        InternalPacket::KnownPacksRequest { packs } => {
            write_varint(&mut buf, CONFIG_KNOWN_PACKS);
            write_varint(&mut buf, packs.len() as i32);
            for pack in packs {
                write_string(&mut buf, &pack.namespace);
                write_string(&mut buf, &pack.id);
                write_string(&mut buf, &pack.version);
            }
        }
        InternalPacket::Disconnect { reason } => {
            write_varint(&mut buf, 0x02); // Disconnect (Configuration)
            // In configuration state, disconnect reason is NBT text component
            let nbt = NbtValue::Compound(vec![
                ("text".into(), NbtValue::String(reason.text.clone())),
            ]);
            let mut nbt_buf = BytesMut::new();
            nbt.write_root_network(&mut nbt_buf);
            buf.extend_from_slice(&nbt_buf);
        }
        _ => bail!(
            "Cannot encode {:?} in Configuration state",
            std::mem::discriminant(packet)
        ),
    }
    Ok(buf)
}

fn encode_play(packet: &InternalPacket) -> Result<BytesMut> {
    let mut buf = BytesMut::new();
    match packet {
        InternalPacket::JoinGame {
            entity_id,
            is_hardcore,
            dimension_names,
            max_players,
            view_distance,
            simulation_distance,
            reduced_debug_info,
            enable_respawn_screen,
            do_limited_crafting,
            dimension_type,
            dimension_name,
            hashed_seed,
            game_mode,
            previous_game_mode,
            is_debug,
            is_flat,
            portal_cooldown,
            enforces_secure_chat,
        } => {
            write_varint(&mut buf, PLAY_LOGIN);
            buf.put_i32(*entity_id);
            buf.put_u8(*is_hardcore as u8);
            write_varint(&mut buf, dimension_names.len() as i32);
            for dim in dimension_names {
                write_string(&mut buf, dim);
            }
            write_varint(&mut buf, *max_players);
            write_varint(&mut buf, *view_distance);
            write_varint(&mut buf, *simulation_distance);
            buf.put_u8(*reduced_debug_info as u8);
            buf.put_u8(*enable_respawn_screen as u8);
            buf.put_u8(*do_limited_crafting as u8);
            write_varint(&mut buf, *dimension_type);
            write_string(&mut buf, dimension_name);
            buf.put_i64(*hashed_seed);
            buf.put_u8(game_mode.id());
            buf.put_i8(*previous_game_mode);
            buf.put_u8(*is_debug as u8);
            buf.put_u8(*is_flat as u8);
            // Death location: not present
            buf.put_u8(0);
            write_varint(&mut buf, *portal_cooldown);
            buf.put_u8(*enforces_secure_chat as u8);
        }
        InternalPacket::SynchronizePlayerPosition {
            position,
            yaw,
            pitch,
            flags,
            teleport_id,
        } => {
            write_varint(&mut buf, PLAY_SYNC_PLAYER_POS);
            buf.put_f64(position.x);
            buf.put_f64(position.y);
            buf.put_f64(position.z);
            buf.put_f32(*yaw);
            buf.put_f32(*pitch);
            buf.put_u8(*flags);
            write_varint(&mut buf, *teleport_id);
        }
        InternalPacket::SetCenterChunk { chunk_x, chunk_z } => {
            write_varint(&mut buf, PLAY_SET_CENTER_CHUNK);
            write_varint(&mut buf, *chunk_x);
            write_varint(&mut buf, *chunk_z);
        }
        InternalPacket::ChunkDataAndUpdateLight {
            chunk_x,
            chunk_z,
            heightmaps,
            data,
            block_entities,
            light_data,
        } => {
            write_varint(&mut buf, PLAY_CHUNK_DATA);
            buf.put_i32(*chunk_x);
            buf.put_i32(*chunk_z);
            // Heightmaps NBT
            let mut nbt_buf = BytesMut::new();
            heightmaps.write_root_network(&mut nbt_buf);
            buf.extend_from_slice(&nbt_buf);
            // Chunk data
            write_varint(&mut buf, data.len() as i32);
            buf.extend_from_slice(data);
            // Block entities
            write_varint(&mut buf, 0); // number of block entities
            let _ = block_entities;
            // Light data
            encode_light_data(&mut buf, light_data);
        }
        InternalPacket::UnloadChunk { chunk_x, chunk_z } => {
            write_varint(&mut buf, PLAY_UNLOAD_CHUNK);
            buf.put_i32(*chunk_z);
            buf.put_i32(*chunk_x);
        }
        InternalPacket::KeepAliveClientbound { id } => {
            write_varint(&mut buf, PLAY_KEEP_ALIVE);
            buf.put_i64(*id);
        }
        InternalPacket::GameEvent { event, value } => {
            write_varint(&mut buf, PLAY_GAME_EVENT);
            buf.put_u8(*event);
            buf.put_f32(*value);
        }
        InternalPacket::SetDefaultSpawnPosition { position, angle } => {
            write_varint(&mut buf, PLAY_SET_DEFAULT_SPAWN);
            buf.put_u64(position.encode());
            buf.put_f32(*angle);
        }
        InternalPacket::Disconnect { reason } => {
            write_varint(&mut buf, PLAY_DISCONNECT);
            // Play disconnect uses NBT text component in 1.20.3+
            let nbt = NbtValue::Compound(vec![
                ("text".into(), NbtValue::String(reason.text.clone())),
            ]);
            let mut nbt_buf = BytesMut::new();
            nbt.write_root_network(&mut nbt_buf);
            buf.extend_from_slice(&nbt_buf);
        }
        _ => bail!("Cannot encode {:?} in Play state", std::mem::discriminant(packet)),
    }
    Ok(buf)
}

fn encode_light_data(buf: &mut BytesMut, light: &ChunkLightData) {
    // Sky light mask
    write_varint(buf, light.sky_light_mask.len() as i32);
    for v in &light.sky_light_mask {
        buf.put_i64(*v);
    }
    // Block light mask
    write_varint(buf, light.block_light_mask.len() as i32);
    for v in &light.block_light_mask {
        buf.put_i64(*v);
    }
    // Empty sky light mask
    write_varint(buf, light.empty_sky_light_mask.len() as i32);
    for v in &light.empty_sky_light_mask {
        buf.put_i64(*v);
    }
    // Empty block light mask
    write_varint(buf, light.empty_block_light_mask.len() as i32);
    for v in &light.empty_block_light_mask {
        buf.put_i64(*v);
    }
    // Sky light arrays
    write_varint(buf, light.sky_light_arrays.len() as i32);
    for arr in &light.sky_light_arrays {
        write_varint(buf, arr.len() as i32);
        buf.extend_from_slice(arr);
    }
    // Block light arrays
    write_varint(buf, light.block_light_arrays.len() as i32);
    for arr in &light.block_light_arrays {
        write_varint(buf, arr.len() as i32);
        buf.extend_from_slice(arr);
    }
}

