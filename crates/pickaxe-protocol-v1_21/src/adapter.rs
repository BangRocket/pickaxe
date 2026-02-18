use anyhow::{bail, Result};
use bytes::{Buf, BufMut, BytesMut};
use pickaxe_nbt::NbtValue;
use pickaxe_protocol_core::*;
use pickaxe_types::BlockPos;

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
const PLAY_ACK_BLOCK_CHANGE: i32 = 0x05;
const PLAY_BLOCK_DESTROY_STAGE: i32 = 0x06;
const PLAY_BLOCK_UPDATE: i32 = 0x09;
const PLAY_DISCONNECT: i32 = 0x1D;
const PLAY_UNLOAD_CHUNK: i32 = 0x21;
const PLAY_GAME_EVENT: i32 = 0x22;
const PLAY_KEEP_ALIVE: i32 = 0x26;
const PLAY_CHUNK_DATA: i32 = 0x27;
const PLAY_LOGIN: i32 = 0x2B;
const PLAY_PLAYER_REMOVE: i32 = 0x3D;
const PLAY_PLAYER_INFO: i32 = 0x3E;
const PLAY_SYNC_PLAYER_POS: i32 = 0x40;
const PLAY_SET_CENTER_CHUNK: i32 = 0x54;
const PLAY_SET_DEFAULT_SPAWN: i32 = 0x56;
const PLAY_SYSTEM_CHAT: i32 = 0x6C;
const PLAY_SPAWN_ENTITY: i32 = 0x01;
const PLAY_REMOVE_ENTITIES: i32 = 0x42;
const PLAY_UPDATE_ENTITY_POS: i32 = 0x2E;
const PLAY_UPDATE_ENTITY_POS_ROT: i32 = 0x2F;
const PLAY_UPDATE_ENTITY_ROT: i32 = 0x30;
const PLAY_SET_HEAD_ROTATION: i32 = 0x48;
const PLAY_TELEPORT_ENTITY: i32 = 0x70;
const PLAY_DECLARE_COMMANDS: i32 = 0x11;
const PLAY_SET_CONTAINER_CONTENT: i32 = 0x13;
const PLAY_SET_CONTAINER_SLOT: i32 = 0x15;
const PLAY_SET_HELD_ITEM: i32 = 0x53;
const PLAY_DAMAGE_EVENT: i32 = 0x1A;
const PLAY_ENTITY_EVENT: i32 = 0x1F;
const PLAY_HURT_ANIMATION: i32 = 0x24;
const PLAY_PLAYER_COMBAT_KILL: i32 = 0x3C;
const PLAY_RESPAWN: i32 = 0x47;
const PLAY_SET_ENTITY_METADATA: i32 = 0x58;
const PLAY_SET_EQUIPMENT: i32 = 0x5B;
const PLAY_SET_ENTITY_VELOCITY: i32 = 0x5A;
const PLAY_SET_HEALTH: i32 = 0x5D;
const PLAY_CONTAINER_CLOSE: i32 = 0x12;
const PLAY_SET_CONTAINER_DATA: i32 = 0x14;
const PLAY_OPEN_SCREEN: i32 = 0x33;
const PLAY_UPDATE_TIME: i32 = 0x64;
const PLAY_ENTITY_ANIMATION: i32 = 0x03;
const PLAY_TAKE_ITEM_ENTITY: i32 = 0x6F;
const PLAY_SOUND_EFFECT: i32 = 0x68;
const PLAY_WORLD_EVENT: i32 = 0x28;
const PLAY_SET_EXPERIENCE: i32 = 0x5C;
const PLAY_ADD_EXPERIENCE_ORB: i32 = 0x02;
const PLAY_UPDATE_MOB_EFFECT: i32 = 0x75;
const PLAY_REMOVE_MOB_EFFECT: i32 = 0x42;
const PLAY_BLOCK_ENTITY_DATA: i32 = 0x07;
const PLAY_OPEN_SIGN_EDITOR: i32 = 0x34;

// === Decode functions ===

fn decode_handshaking(id: i32, data: &mut BytesMut) -> Result<InternalPacket> {
    match id {
        0x00 => {
            let protocol_version = read_varint(data)?;
            let server_address = read_string(data, 255)?;
            let server_port = read_u16(data)?;
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
            let payload = read_i64(data)?;
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
            let view_distance = read_i8(data)?;
            let chat_mode = read_varint(data)?;
            let chat_colors = read_u8(data)? != 0;
            let skin_parts = read_u8(data)?;
            let main_hand = read_varint(data)?;
            let text_filtering = read_u8(data)? != 0;
            let allow_listing = read_u8(data)? != 0;
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
        0x04 => {
            // Chat Command (serverbound)
            let command = read_string(data, 256)?;
            // Skip remaining fields (timestamp, salt, signatures, etc.)
            // We only need the command text
            data.advance(data.remaining());
            Ok(InternalPacket::ChatCommand { command })
        }
        0x06 => {
            // Chat Message (serverbound)
            let message = read_string(data, 256)?;
            let timestamp = read_i64(data)?;
            let salt = read_i64(data)?;
            let has_signature = read_u8(data)? != 0;
            let signature = if has_signature {
                Some(read_bytes(data, 256)?)
            } else {
                None
            };
            let offset = read_varint(data)?;
            let acknowledged_vec = read_bytes(data, 3)?;
            let mut acknowledged = [0u8; 3];
            acknowledged.copy_from_slice(&acknowledged_vec);
            Ok(InternalPacket::ChatMessage {
                message,
                timestamp,
                salt,
                has_signature,
                signature,
                offset,
                acknowledged,
            })
        }
        0x08 => {
            // Chunk Batch Received — just acknowledge, read the chunks_per_tick float
            let _chunks_per_tick = read_f32(data)?;
            Ok(InternalPacket::Unknown {
                packet_id: id,
                data: vec![],
            })
        }
        0x09 => {
            // Client Command (respawn / request stats)
            let action = read_varint(data)?;
            Ok(InternalPacket::ClientCommand { action })
        }
        0x18 => {
            let id = read_i64(data)?;
            Ok(InternalPacket::KeepAliveServerbound { id })
        }
        0x1A => {
            let x = read_f64(data)?;
            let y = read_f64(data)?;
            let z = read_f64(data)?;
            let on_ground = read_u8(data)? != 0;
            Ok(InternalPacket::PlayerPosition { x, y, z, on_ground })
        }
        0x1B => {
            let x = read_f64(data)?;
            let y = read_f64(data)?;
            let z = read_f64(data)?;
            let yaw = read_f32(data)?;
            let pitch = read_f32(data)?;
            let on_ground = read_u8(data)? != 0;
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
            let yaw = read_f32(data)?;
            let pitch = read_f32(data)?;
            let on_ground = read_u8(data)? != 0;
            Ok(InternalPacket::PlayerRotation {
                yaw,
                pitch,
                on_ground,
            })
        }
        0x1D => {
            let on_ground = read_u8(data)? != 0;
            Ok(InternalPacket::PlayerOnGround { on_ground })
        }
        0x24 => {
            // block_dig (Player Action)
            let status = read_varint(data)?;
            let position = BlockPos::decode(read_u64(data)?);
            let face = read_u8(data)?;
            let sequence = read_varint(data)?;
            Ok(InternalPacket::BlockDig { status, position, face, sequence })
        }
        0x25 => {
            // Player Command (sprint/sneak/etc.)
            let entity_id = read_varint(data)?;
            let action = read_varint(data)?;
            let jump_boost = read_varint(data)?;
            Ok(InternalPacket::PlayerCommand { entity_id, action, data: jump_boost })
        }
        0x38 => {
            // block_place (Use Item On)
            let hand = read_varint(data)?;
            let position = BlockPos::decode(read_u64(data)?);
            let face = read_u8(data)?;
            let cursor_x = read_f32(data)?;
            let cursor_y = read_f32(data)?;
            let cursor_z = read_f32(data)?;
            let inside_block = read_u8(data)? != 0;
            let sequence = read_varint(data)?;
            Ok(InternalPacket::BlockPlace { hand, position, face, cursor_x, cursor_y, cursor_z, inside_block, sequence })
        }
        0x39 => {
            // Use Item (right-click in air: eat, drink, shoot)
            let hand = read_varint(data)?;
            let sequence = read_varint(data)?;
            // yRot and xRot follow but we don't need them
            Ok(InternalPacket::UseItem { hand, sequence })
        }
        0x0E => {
            // Container Click
            let window_id = read_u8(data)?;
            let state_id = read_varint(data)?;
            let slot = read_i16(data)?;
            let button = read_i8(data)?;
            let mode = read_varint(data)?;
            let count = read_varint(data)? as usize;
            let mut changed_slots = Vec::with_capacity(count);
            for _ in 0..count {
                let loc = read_i16(data)?;
                let item = read_slot(data).map_err(|e| anyhow::anyhow!("{}", e))?;
                changed_slots.push((loc, item));
            }
            let carried_item = read_slot(data).map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(InternalPacket::ContainerClick {
                window_id, state_id, slot, button, mode, changed_slots, carried_item,
            })
        }
        0x0F => {
            // Close Container (serverbound)
            let container_id = read_u8(data)?;
            Ok(InternalPacket::ClientCloseContainer { container_id })
        }
        0x2A => {
            // Rename Item (serverbound) — anvil rename field
            let name = read_string(data, 50).map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(InternalPacket::RenameItem { name })
        }
        0x2F => {
            // SetHeldItem (serverbound)
            let slot_id = read_i16(data)?;
            Ok(InternalPacket::HeldItemChange { slot: slot_id })
        }
        0x32 => {
            // CreativeInventoryAction
            let slot = read_i16(data)?;
            let item = read_slot(data).map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(InternalPacket::CreativeInventoryAction { slot, item })
        }
        0x16 => {
            // Interact Entity
            let entity_id = read_varint(data)?;
            let action_type = read_varint(data)?;
            let (target_x, target_y, target_z, hand) = match action_type {
                0 => {
                    // INTERACT: hand follows
                    let hand = read_varint(data)?;
                    (0.0, 0.0, 0.0, hand)
                }
                1 => {
                    // ATTACK: no extra data
                    (0.0, 0.0, 0.0, 0)
                }
                2 => {
                    // INTERACT_AT: x, y, z, hand
                    let tx = read_f32(data)?;
                    let ty = read_f32(data)?;
                    let tz = read_f32(data)?;
                    let hand = read_varint(data)?;
                    (tx, ty, tz, hand)
                }
                _ => (0.0, 0.0, 0.0, 0),
            };
            let sneaking = if data.len() >= 1 { read_u8(data)? != 0 } else { false };
            Ok(InternalPacket::InteractEntity {
                entity_id, action_type, target_x, target_y, target_z, hand, sneaking,
            })
        }
        0x35 => {
            // Sign Update (serverbound) — client finished editing a sign
            let position = BlockPos::decode(read_u64(data)?);
            let is_front_text = read_u8(data)? != 0;
            let line1 = read_string(data, 384)?;
            let line2 = read_string(data, 384)?;
            let line3 = read_string(data, 384)?;
            let line4 = read_string(data, 384)?;
            Ok(InternalPacket::SignUpdate {
                position,
                is_front_text,
                lines: [line1, line2, line3, line4],
            })
        }
        0x36 => {
            // Swing (arm animation)
            let hand = read_varint(data)?;
            Ok(InternalPacket::Swing { hand })
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
        InternalPacket::BlockUpdate { position, block_id } => {
            write_varint(&mut buf, PLAY_BLOCK_UPDATE);
            buf.put_u64(position.encode());
            write_varint(&mut buf, *block_id);
        }
        InternalPacket::AcknowledgeBlockChange { sequence } => {
            write_varint(&mut buf, PLAY_ACK_BLOCK_CHANGE);
            write_varint(&mut buf, *sequence);
        }
        InternalPacket::ChunkBatchStart => {
            write_varint(&mut buf, 0x0D);
        }
        InternalPacket::ChunkBatchFinished { batch_size } => {
            write_varint(&mut buf, 0x0C);
            write_varint(&mut buf, *batch_size);
        }
        InternalPacket::SystemChatMessage { content, overlay } => {
            write_varint(&mut buf, PLAY_SYSTEM_CHAT);
            // Content is an NBT text component (anonymous NBT in 1.20.3+)
            let nbt = NbtValue::Compound(vec![
                ("text".into(), NbtValue::String(content.text.clone())),
            ]);
            let mut nbt_buf = BytesMut::new();
            nbt.write_root_network(&mut nbt_buf);
            buf.extend_from_slice(&nbt_buf);
            buf.put_u8(*overlay as u8);
        }
        InternalPacket::PlayerInfoUpdate { actions, players } => {
            write_varint(&mut buf, PLAY_PLAYER_INFO);
            buf.put_u8(*actions);
            write_varint(&mut buf, players.len() as i32);
            for player in players {
                write_uuid(&mut buf, &player.uuid);
                if actions & player_info_actions::ADD_PLAYER != 0 {
                    write_string(&mut buf, player.name.as_deref().unwrap_or(""));
                    // Properties
                    let props = &player.properties;
                    write_varint(&mut buf, props.len() as i32);
                    for (name, value, signature) in props {
                        write_string(&mut buf, name);
                        write_string(&mut buf, value);
                        if let Some(sig) = signature {
                            buf.put_u8(1);
                            write_string(&mut buf, sig);
                        } else {
                            buf.put_u8(0);
                        }
                    }
                }
                if actions & player_info_actions::INITIALIZE_CHAT != 0 {
                    // No chat session — write false
                    buf.put_u8(0);
                }
                if actions & player_info_actions::UPDATE_GAME_MODE != 0 {
                    write_varint(&mut buf, player.game_mode.unwrap_or(0));
                }
                if actions & player_info_actions::UPDATE_LISTED != 0 {
                    buf.put_u8(player.listed.unwrap_or(true) as u8);
                }
                if actions & player_info_actions::UPDATE_LATENCY != 0 {
                    write_varint(&mut buf, player.ping.unwrap_or(0));
                }
                if actions & player_info_actions::UPDATE_DISPLAY_NAME != 0 {
                    if let Some(ref display) = player.display_name {
                        buf.put_u8(1); // has display name
                        let nbt = NbtValue::Compound(vec![
                            ("text".into(), NbtValue::String(display.text.clone())),
                        ]);
                        let mut nbt_buf = BytesMut::new();
                        nbt.write_root_network(&mut nbt_buf);
                        buf.extend_from_slice(&nbt_buf);
                    } else {
                        buf.put_u8(0); // no display name
                    }
                }
            }
        }
        InternalPacket::PlayerInfoRemove { uuids } => {
            write_varint(&mut buf, PLAY_PLAYER_REMOVE);
            write_varint(&mut buf, uuids.len() as i32);
            for uuid in uuids {
                write_uuid(&mut buf, uuid);
            }
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
        InternalPacket::SpawnEntity {
            entity_id, entity_uuid, entity_type, x, y, z,
            pitch, yaw, head_yaw, data, velocity_x, velocity_y, velocity_z,
        } => {
            write_varint(&mut buf, PLAY_SPAWN_ENTITY);
            write_varint(&mut buf, *entity_id);
            write_uuid(&mut buf, entity_uuid);
            write_varint(&mut buf, *entity_type);
            buf.put_f64(*x);
            buf.put_f64(*y);
            buf.put_f64(*z);
            buf.put_u8(*pitch);
            buf.put_u8(*yaw);
            buf.put_u8(*head_yaw);
            write_varint(&mut buf, *data);
            buf.put_i16(*velocity_x);
            buf.put_i16(*velocity_y);
            buf.put_i16(*velocity_z);
        }
        InternalPacket::RemoveEntities { entity_ids } => {
            write_varint(&mut buf, PLAY_REMOVE_ENTITIES);
            write_varint(&mut buf, entity_ids.len() as i32);
            for &eid in entity_ids {
                write_varint(&mut buf, eid);
            }
        }
        InternalPacket::UpdateEntityPosition { entity_id, delta_x, delta_y, delta_z, on_ground } => {
            write_varint(&mut buf, PLAY_UPDATE_ENTITY_POS);
            write_varint(&mut buf, *entity_id);
            buf.put_i16(*delta_x);
            buf.put_i16(*delta_y);
            buf.put_i16(*delta_z);
            buf.put_u8(*on_ground as u8);
        }
        InternalPacket::UpdateEntityPositionAndRotation { entity_id, delta_x, delta_y, delta_z, yaw, pitch, on_ground } => {
            write_varint(&mut buf, PLAY_UPDATE_ENTITY_POS_ROT);
            write_varint(&mut buf, *entity_id);
            buf.put_i16(*delta_x);
            buf.put_i16(*delta_y);
            buf.put_i16(*delta_z);
            buf.put_u8(*yaw);
            buf.put_u8(*pitch);
            buf.put_u8(*on_ground as u8);
        }
        InternalPacket::UpdateEntityRotation { entity_id, yaw, pitch, on_ground } => {
            write_varint(&mut buf, PLAY_UPDATE_ENTITY_ROT);
            write_varint(&mut buf, *entity_id);
            buf.put_u8(*yaw);
            buf.put_u8(*pitch);
            buf.put_u8(*on_ground as u8);
        }
        InternalPacket::SetHeadRotation { entity_id, head_yaw } => {
            write_varint(&mut buf, PLAY_SET_HEAD_ROTATION);
            write_varint(&mut buf, *entity_id);
            buf.put_u8(*head_yaw);
        }
        InternalPacket::TeleportEntity { entity_id, x, y, z, yaw, pitch, on_ground } => {
            write_varint(&mut buf, PLAY_TELEPORT_ENTITY);
            write_varint(&mut buf, *entity_id);
            buf.put_f64(*x);
            buf.put_f64(*y);
            buf.put_f64(*z);
            buf.put_u8(*yaw);
            buf.put_u8(*pitch);
            buf.put_u8(*on_ground as u8);
        }
        InternalPacket::SetContainerContent { window_id, state_id, slots, carried_item } => {
            write_varint(&mut buf, PLAY_SET_CONTAINER_CONTENT);
            buf.put_u8(*window_id);
            write_varint(&mut buf, *state_id);
            write_varint(&mut buf, slots.len() as i32);
            for slot in slots {
                write_slot(&mut buf, slot);
            }
            write_slot(&mut buf, carried_item);
        }
        InternalPacket::SetContainerSlot { window_id, state_id, slot, item } => {
            write_varint(&mut buf, PLAY_SET_CONTAINER_SLOT);
            buf.put_i8(*window_id);
            write_varint(&mut buf, *state_id);
            buf.put_i16(*slot);
            write_slot(&mut buf, item);
        }
        InternalPacket::SetHeldItem { slot } => {
            write_varint(&mut buf, PLAY_SET_HELD_ITEM);
            buf.put_i8(*slot);
        }
        InternalPacket::DeclareCommands { nodes, root_index } => {
            write_varint(&mut buf, PLAY_DECLARE_COMMANDS);
            write_varint(&mut buf, nodes.len() as i32);
            for node in nodes {
                buf.put_u8(node.flags);
                write_varint(&mut buf, node.children.len() as i32);
                for &child in &node.children {
                    write_varint(&mut buf, child);
                }
                // Literal (type 1) and argument (type 2) nodes have a name
                if let Some(name) = &node.name {
                    write_string(&mut buf, name);
                }
                // Argument nodes (type 2) have a parser ID + optional properties
                if let Some(parser) = &node.parser {
                    write_varint(&mut buf, parser_to_id(parser));
                    if let Some(props) = &node.parser_properties {
                        buf.extend_from_slice(props);
                    }
                }
            }
            write_varint(&mut buf, *root_index);
        }
        InternalPacket::UpdateTime { world_age, time_of_day } => {
            write_varint(&mut buf, PLAY_UPDATE_TIME);
            buf.put_i64(*world_age);
            buf.put_i64(*time_of_day);
        }
        InternalPacket::SetBlockDestroyStage { entity_id, position, destroy_stage } => {
            write_varint(&mut buf, PLAY_BLOCK_DESTROY_STAGE);
            write_varint(&mut buf, *entity_id);
            buf.put_u64(position.encode());
            buf.put_i8(*destroy_stage);
        }
        InternalPacket::SetEntityMetadata { entity_id, metadata } => {
            write_varint(&mut buf, PLAY_SET_ENTITY_METADATA);
            write_varint(&mut buf, *entity_id);
            for entry in metadata {
                buf.put_u8(entry.index);
                write_varint(&mut buf, entry.type_id);
                buf.extend_from_slice(&entry.data);
            }
            buf.put_u8(0xFF); // terminator
        }
        InternalPacket::SetEquipment { entity_id, equipment } => {
            write_varint(&mut buf, PLAY_SET_EQUIPMENT);
            write_varint(&mut buf, *entity_id);
            for (i, (slot, item)) in equipment.iter().enumerate() {
                let is_last = i == equipment.len() - 1;
                let slot_byte = if is_last { *slot } else { *slot | 0x80 };
                buf.put_u8(slot_byte);
                write_slot(&mut buf, item);
            }
        }
        InternalPacket::SetEntityVelocity { entity_id, velocity_x, velocity_y, velocity_z } => {
            write_varint(&mut buf, PLAY_SET_ENTITY_VELOCITY);
            write_varint(&mut buf, *entity_id);
            buf.put_i16(*velocity_x);
            buf.put_i16(*velocity_y);
            buf.put_i16(*velocity_z);
        }
        InternalPacket::SetHealth { health, food, saturation } => {
            write_varint(&mut buf, PLAY_SET_HEALTH);
            buf.put_f32(*health);
            write_varint(&mut buf, *food);
            buf.put_f32(*saturation);
        }
        InternalPacket::HurtAnimation { entity_id, yaw } => {
            write_varint(&mut buf, PLAY_HURT_ANIMATION);
            write_varint(&mut buf, *entity_id);
            buf.put_f32(*yaw);
        }
        InternalPacket::EntityEvent { entity_id, event_id } => {
            write_varint(&mut buf, PLAY_ENTITY_EVENT);
            buf.put_i32(*entity_id); // raw i32, NOT VarInt
            buf.put_i8(*event_id);
        }
        InternalPacket::PlayerCombatKill { player_id, message } => {
            write_varint(&mut buf, PLAY_PLAYER_COMBAT_KILL);
            write_varint(&mut buf, *player_id);
            // Death message as NBT text component
            let nbt = NbtValue::Compound(vec![
                ("text".into(), NbtValue::String(message.text.clone())),
            ]);
            let mut nbt_buf = BytesMut::new();
            nbt.write_root_network(&mut nbt_buf);
            buf.extend_from_slice(&nbt_buf);
        }
        InternalPacket::Respawn {
            dimension_type, dimension_name, hashed_seed,
            game_mode, previous_game_mode, is_debug, is_flat,
            data_to_keep, last_death_x, last_death_y, last_death_z,
            last_death_dimension, portal_cooldown,
        } => {
            write_varint(&mut buf, PLAY_RESPAWN);
            // CommonPlayerSpawnInfo (same structure as JoinGame)
            write_varint(&mut buf, *dimension_type);
            write_string(&mut buf, dimension_name);
            buf.put_i64(*hashed_seed);
            buf.put_u8(*game_mode);
            buf.put_i8(*previous_game_mode);
            buf.put_u8(*is_debug as u8);
            buf.put_u8(*is_flat as u8);
            // Death location
            if let (Some(dx), Some(dy), Some(dz), Some(dim)) =
                (last_death_x, last_death_y, last_death_z, last_death_dimension)
            {
                buf.put_u8(1); // has death location
                write_string(&mut buf, dim);
                buf.put_u64(BlockPos::new(*dx, *dy, *dz).encode());
            } else {
                buf.put_u8(0); // no death location
            }
            write_varint(&mut buf, *portal_cooldown);
            buf.put_u8(*data_to_keep);
        }
        InternalPacket::OpenScreen { container_id, menu_type, title } => {
            write_varint(&mut buf, PLAY_OPEN_SCREEN);
            write_varint(&mut buf, *container_id);
            write_varint(&mut buf, *menu_type);
            // Title as NBT text component
            let nbt = NbtValue::Compound(vec![
                ("text".into(), NbtValue::String(title.text.clone())),
            ]);
            let mut nbt_buf = BytesMut::new();
            nbt.write_root_network(&mut nbt_buf);
            buf.extend_from_slice(&nbt_buf);
        }
        InternalPacket::ContainerClose { container_id } => {
            write_varint(&mut buf, PLAY_CONTAINER_CLOSE);
            write_varint(&mut buf, *container_id);
        }
        InternalPacket::SetContainerData { container_id, property, value } => {
            write_varint(&mut buf, PLAY_SET_CONTAINER_DATA);
            buf.put_u8(*container_id);
            buf.put_i16(*property);
            buf.put_i16(*value);
        }
        InternalPacket::EntityAnimation { entity_id, animation } => {
            write_varint(&mut buf, PLAY_ENTITY_ANIMATION);
            write_varint(&mut buf, *entity_id);
            buf.put_u8(*animation);
        }
        InternalPacket::TakeItemEntity { collected_entity_id, collector_entity_id, item_count } => {
            write_varint(&mut buf, PLAY_TAKE_ITEM_ENTITY);
            write_varint(&mut buf, *collected_entity_id);
            write_varint(&mut buf, *collector_entity_id);
            write_varint(&mut buf, *item_count);
        }
        InternalPacket::SoundEffect { sound_name, source, x, y, z, volume, pitch, seed } => {
            write_varint(&mut buf, PLAY_SOUND_EFFECT);
            // Inline SoundEvent (Holder type = DIRECT)
            write_varint(&mut buf, 0); // 0 = inline/direct, not a registry reference
            write_string(&mut buf, sound_name); // resource location
            buf.put_u8(0); // Optional<Float> = empty (no fixed range)
            // SoundSource enum ordinal
            write_varint(&mut buf, *source as i32);
            // Fixed-point coordinates (x * 8)
            buf.put_i32((*x * 8.0) as i32);
            buf.put_i32((*y * 8.0) as i32);
            buf.put_i32((*z * 8.0) as i32);
            buf.put_f32(*volume);
            buf.put_f32(*pitch);
            buf.put_i64(*seed);
        }
        InternalPacket::WorldEvent { event, position, data, disable_relative } => {
            write_varint(&mut buf, PLAY_WORLD_EVENT);
            buf.put_i32(*event);
            // Position: packed as u64
            let pos_val = ((position.x as i64 & 0x3FFFFFF) << 38)
                | ((position.z as i64 & 0x3FFFFFF) << 12)
                | (position.y as i64 & 0xFFF);
            buf.put_i64(pos_val);
            buf.put_i32(*data);
            buf.put_u8(if *disable_relative { 1 } else { 0 });
        }
        InternalPacket::SetExperience { progress, level, total_xp } => {
            write_varint(&mut buf, PLAY_SET_EXPERIENCE);
            buf.put_f32(*progress);
            write_varint(&mut buf, *level);
            write_varint(&mut buf, *total_xp);
        }
        InternalPacket::AddExperienceOrb { entity_id, x, y, z, value } => {
            write_varint(&mut buf, PLAY_ADD_EXPERIENCE_ORB);
            write_varint(&mut buf, *entity_id);
            buf.put_f64(*x);
            buf.put_f64(*y);
            buf.put_f64(*z);
            buf.put_i16(*value);
        }
        InternalPacket::UpdateMobEffect { entity_id, effect_id, amplifier, duration, flags } => {
            write_varint(&mut buf, PLAY_UPDATE_MOB_EFFECT);
            write_varint(&mut buf, *entity_id);
            // Effect ID uses Holder encoding: registry reference = id + 1
            write_varint(&mut buf, *effect_id + 1);
            write_varint(&mut buf, *amplifier);
            write_varint(&mut buf, *duration);
            buf.put_u8(*flags);
        }
        InternalPacket::RemoveMobEffect { entity_id, effect_id } => {
            write_varint(&mut buf, PLAY_REMOVE_MOB_EFFECT);
            write_varint(&mut buf, *entity_id);
            // Effect ID uses Holder encoding: registry reference = id + 1
            write_varint(&mut buf, *effect_id + 1);
        }
        InternalPacket::OpenSignEditor { position, is_front_text } => {
            write_varint(&mut buf, PLAY_OPEN_SIGN_EDITOR);
            buf.put_u64(position.encode());
            buf.put_u8(*is_front_text as u8);
        }
        InternalPacket::BlockEntityData { position, block_entity_type, nbt } => {
            write_varint(&mut buf, PLAY_BLOCK_ENTITY_DATA);
            buf.put_u64(position.encode());
            write_varint(&mut buf, *block_entity_type);
            let mut nbt_buf = BytesMut::new();
            nbt.write_root_network(&mut nbt_buf);
            buf.extend_from_slice(&nbt_buf);
        }
        _ => bail!("Cannot encode {:?} in Play state", std::mem::discriminant(packet)),
    }
    Ok(buf)
}

/// Map parser name to protocol 767 parser ID.
fn parser_to_id(parser: &str) -> i32 {
    match parser {
        "brigadier:bool" => 0,
        "brigadier:float" => 1,
        "brigadier:double" => 2,
        "brigadier:integer" => 3,
        "brigadier:long" => 4,
        "brigadier:string" => 5,
        "minecraft:entity" => 6,
        "minecraft:game_profile" => 7,
        "minecraft:block_pos" => 8,
        "minecraft:time" => 42,
        _ => 5, // fallback to string
    }
}

/// Build entity metadata entries for an item entity.
/// Index 0: byte flags (0x00), Index 8: item_stack (Slot).
/// Build metadata entries to set a player's sleeping pose + sleeping position.
/// Pose index 6 = type 21 (Pose), value 2 (SLEEPING).
/// Sleeping pos index 14 = type 11 (Optional<BlockPos>), value present + encoded BlockPos.
pub fn build_sleeping_metadata(bed_pos: &BlockPos) -> Vec<EntityMetadataEntry> {
    use pickaxe_protocol_core::EntityMetadataEntry;

    // Index 6: Pose = SLEEPING (ordinal 2), type_id 21 (Pose = VarInt)
    let mut pose_data = Vec::new();
    let mut pose_buf = BytesMut::new();
    write_varint(&mut pose_buf, 2); // Pose.SLEEPING = 2
    pose_data.extend_from_slice(&pose_buf);

    let pose_entry = EntityMetadataEntry {
        index: 6,
        type_id: 21,
        data: pose_data,
    };

    // Index 14: Sleeping position = Optional<BlockPos>, type_id 11
    // Encoding: boolean present (true) + packed BlockPos (i64)
    let mut pos_data = Vec::new();
    pos_data.push(1u8); // present = true
    let packed = ((bed_pos.x as i64 & 0x3FFFFFF) << 38)
        | ((bed_pos.z as i64 & 0x3FFFFFF) << 12)
        | (bed_pos.y as i64 & 0xFFF);
    pos_data.extend_from_slice(&packed.to_be_bytes());

    let pos_entry = EntityMetadataEntry {
        index: 14,
        type_id: 11,
        data: pos_data,
    };

    vec![pose_entry, pos_entry]
}

/// Build metadata entries to clear sleeping pose (set to STANDING) and clear sleeping position.
pub fn build_wake_metadata() -> Vec<EntityMetadataEntry> {
    use pickaxe_protocol_core::EntityMetadataEntry;

    // Index 6: Pose = STANDING (ordinal 0)
    let mut pose_data = Vec::new();
    let mut pose_buf = BytesMut::new();
    write_varint(&mut pose_buf, 0); // Pose.STANDING = 0
    pose_data.extend_from_slice(&pose_buf);

    let pose_entry = EntityMetadataEntry {
        index: 6,
        type_id: 21,
        data: pose_data,
    };

    // Index 14: Sleeping position = Optional<BlockPos> = absent
    let pos_entry = EntityMetadataEntry {
        index: 14,
        type_id: 11,
        data: vec![0u8], // present = false
    };

    vec![pose_entry, pos_entry]
}

pub fn build_item_metadata(item: &pickaxe_types::ItemStack) -> Vec<EntityMetadataEntry> {
    use pickaxe_protocol_core::EntityMetadataEntry;

    // Index 0: entity flags byte — type 0 (Byte), value 0x00
    let flags_entry = EntityMetadataEntry {
        index: 0,
        type_id: 0,
        data: vec![0x00],
    };

    // Index 8: item stack — type 7 (Slot)
    let mut slot_buf = BytesMut::new();
    write_slot(&mut slot_buf, &Some(item.clone()));
    let item_entry = EntityMetadataEntry {
        index: 8,
        type_id: 7,
        data: slot_buf.to_vec(),
    };

    vec![flags_entry, item_entry]
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

