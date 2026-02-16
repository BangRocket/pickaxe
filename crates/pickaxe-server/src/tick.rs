use crate::config::ServerConfig;
use crate::ecs::*;
use hecs::World;
use pickaxe_protocol_core::{player_info_actions, CommandNode, InternalPacket, PlayerInfoEntry};
use pickaxe_protocol_v1_21::V1_21Adapter;
use pickaxe_scripting::ScriptRuntime;
use pickaxe_types::{BlockPos, GameMode, GameProfile, TextComponent, Vec3d};
use pickaxe_world::{generate_flat_chunk, Chunk};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use pickaxe_types::ChunkPos;

/// Packets decoded by the reader task, sent to the tick loop.
#[derive(Debug)]
pub struct InboundPacket {
    pub entity_id: i32,
    pub packet: InternalPacket,
}

/// A new player ready to enter play state.
pub struct NewPlayer {
    pub entity_id: i32,
    pub profile: GameProfile,
    pub packet_tx: mpsc::UnboundedSender<InternalPacket>,
    pub packet_rx: mpsc::UnboundedReceiver<InboundPacket>,
}

/// World state: chunk storage.
pub struct WorldState {
    chunks: HashMap<ChunkPos, Chunk>,
    pub world_age: i64,
    pub time_of_day: i64,
    pub tick_count: u64,
}

impl WorldState {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
            world_age: 0,
            time_of_day: 0,
            tick_count: 0,
        }
    }

    pub fn get_chunk_packet(&mut self, chunk_x: i32, chunk_z: i32) -> InternalPacket {
        let pos = ChunkPos::new(chunk_x, chunk_z);
        let chunk = self.chunks.entry(pos).or_insert_with(generate_flat_chunk);
        chunk.to_packet(chunk_x, chunk_z)
    }

    pub fn set_block(&mut self, pos: &BlockPos, state_id: i32) -> i32 {
        let chunk_pos = pos.chunk_pos();
        let local_x = (pos.x.rem_euclid(16)) as usize;
        let local_z = (pos.z.rem_euclid(16)) as usize;
        let chunk = self
            .chunks
            .entry(chunk_pos)
            .or_insert_with(generate_flat_chunk);
        chunk.set_block(local_x, pos.y, local_z, state_id)
    }

    pub fn get_block(&mut self, pos: &BlockPos) -> i32 {
        let chunk_pos = pos.chunk_pos();
        let local_x = (pos.x.rem_euclid(16)) as usize;
        let local_z = (pos.z.rem_euclid(16)) as usize;
        let chunk = self
            .chunks
            .entry(chunk_pos)
            .or_insert_with(generate_flat_chunk);
        chunk.get_block(local_x, pos.y, local_z)
    }
}

/// The main game loop. Runs at 20 TPS on the main thread.
/// Owns the hecs World, the Lua ScriptRuntime, and all game state.
pub async fn run_tick_loop(
    config: Arc<ServerConfig>,
    scripting: ScriptRuntime,
    mut new_player_rx: mpsc::UnboundedReceiver<NewPlayer>,
    player_count: Arc<std::sync::atomic::AtomicUsize>,
    lua_commands: crate::bridge::LuaCommands,
) {
    let adapter = V1_21Adapter::new();
    let mut world = World::new();
    let mut world_state = WorldState::new();

    // Collect inbound packet receivers from all active players
    // We store them separately since hecs components must be Send
    let mut inbound_receivers: HashMap<i32, mpsc::UnboundedReceiver<InboundPacket>> =
        HashMap::new();

    let tick_duration = Duration::from_millis(50); // 20 TPS
    let mut tick_count: u64 = 0;

    info!("Tick loop started (20 TPS)");

    loop {
        let tick_start = Instant::now();

        // 1. Accept new players
        while let Ok(new_player) = new_player_rx.try_recv() {
            handle_new_player(
                &config,
                &adapter,
                &mut world,
                &mut world_state,
                &mut inbound_receivers,
                new_player,
                &scripting,
                &lua_commands,
            );
        }

        // 2. Process inbound packets from all players
        let mut packets: Vec<InboundPacket> = Vec::new();
        let mut disconnected: Vec<i32> = Vec::new();

        for (&eid, rx) in inbound_receivers.iter_mut() {
            loop {
                match rx.try_recv() {
                    Ok(pkt) => packets.push(pkt),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        disconnected.push(eid);
                        break;
                    }
                }
            }
        }

        // 3. Handle disconnected players
        for eid in &disconnected {
            handle_disconnect(&mut world, &mut world_state, &mut inbound_receivers, *eid, &adapter, &scripting);
        }

        // Update player count
        let count = world.query::<&Profile>().iter().count();
        player_count.store(count, Ordering::Relaxed);

        // Update tick count in world state so it's available in process_packet
        world_state.tick_count = tick_count;

        // 4. Process packets
        for pkt in packets {
            process_packet(
                &config,
                &adapter,
                &mut world,
                &mut world_state,
                pkt,
                &scripting,
                &lua_commands,
            );
        }

        // 5. Tick systems
        tick_keep_alive(&adapter, &mut world, tick_count);
        tick_entity_tracking(&mut world);
        tick_entity_movement_broadcast(&mut world);
        tick_world_time(&world, &mut world_state, tick_count);
        tick_block_breaking(&mut world, tick_count);

        tick_count += 1;

        // Sleep for remainder of tick
        let elapsed = tick_start.elapsed();
        if elapsed < tick_duration {
            tokio::time::sleep(tick_duration - elapsed).await;
        } else if tick_count % 100 == 0 {
            // Only warn occasionally to avoid log spam
            warn!(
                "Tick {} took {:?} (over 50ms budget)",
                tick_count, elapsed
            );
        }
    }
}

fn handle_new_player(
    config: &ServerConfig,
    _adapter: &V1_21Adapter,
    world: &mut World,
    world_state: &mut WorldState,
    inbound_receivers: &mut HashMap<i32, mpsc::UnboundedReceiver<InboundPacket>>,
    new_player: NewPlayer,
    scripting: &ScriptRuntime,
    lua_commands: &crate::bridge::LuaCommands,
) {
    let entity_id = new_player.entity_id;
    let profile = new_player.profile.clone();
    let sender = new_player.packet_tx;

    info!("{} entering play state (eid={})", profile.name, entity_id);

    let view_distance = config.view_distance as i32;

    // Send Join Game
    let _ = sender.send(InternalPacket::JoinGame {
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
        game_mode: GameMode::Survival,
        previous_game_mode: -1,
        is_debug: false,
        is_flat: true,
        portal_cooldown: 0,
        enforces_secure_chat: false,
    });

    // Declare commands for tab completion (includes Lua-registered commands)
    let _ = sender.send(build_command_tree(lua_commands));

    // Send current world time
    let _ = sender.send(InternalPacket::UpdateTime {
        world_age: world_state.world_age,
        time_of_day: world_state.time_of_day,
    });

    // Spawn position
    let spawn_pos = Vec3d::new(0.5, -59.0, 0.5);
    let center_cx = (spawn_pos.x as i32) >> 4;
    let center_cz = (spawn_pos.z as i32) >> 4;

    // Set center chunk
    let _ = sender.send(InternalPacket::SetCenterChunk {
        chunk_x: center_cx,
        chunk_z: center_cz,
    });

    // Send chunks using batch protocol
    send_chunks_around(&sender, world_state, center_cx, center_cz, view_distance);

    // Teleport player to spawn
    let _ = sender.send(InternalPacket::SynchronizePlayerPosition {
        position: spawn_pos,
        yaw: 0.0,
        pitch: 0.0,
        flags: 0,
        teleport_id: 1,
    });

    // Start waiting for level chunks
    let _ = sender.send(InternalPacket::GameEvent {
        event: 13,
        value: 0.0,
    });

    // Default spawn position (must come after GameEvent)
    let _ = sender.send(InternalPacket::SetDefaultSpawnPosition {
        position: BlockPos::new(0, -60, 0),
        angle: 0.0,
    });

    // Send tab list: add this player to all existing players, and all existing to this player
    // First, send all existing players to the new player
    let mut existing_entries: Vec<PlayerInfoEntry> = Vec::new();
    for (_eid, (p, gm)) in world.query::<(&Profile, &PlayerGameMode)>().iter() {
        existing_entries.push(PlayerInfoEntry {
            uuid: p.0.uuid,
            name: Some(p.0.name.clone()),
            properties: p
                .0
                .properties
                .iter()
                .map(|pr| (pr.name.clone(), pr.value.clone(), pr.signature.clone()))
                .collect(),
            game_mode: Some(gm.0.id() as i32),
            listed: Some(true),
            ping: Some(0),
            display_name: None,
        });
    }

    // Add the new player's own entry
    let new_entry = PlayerInfoEntry {
        uuid: profile.uuid,
        name: Some(profile.name.clone()),
        properties: profile
            .properties
            .iter()
            .map(|pr| (pr.name.clone(), pr.value.clone(), pr.signature.clone()))
            .collect(),
        game_mode: Some(GameMode::Survival.id() as i32),
        listed: Some(true),
        ping: Some(0),
        display_name: None,
    };

    // Send all existing + self to new player
    let mut all_entries = existing_entries;
    all_entries.push(new_entry.clone());

    let actions = player_info_actions::ADD_PLAYER
        | player_info_actions::UPDATE_GAME_MODE
        | player_info_actions::UPDATE_LISTED
        | player_info_actions::UPDATE_LATENCY;

    let _ = sender.send(InternalPacket::PlayerInfoUpdate {
        actions,
        players: all_entries,
    });

    // Broadcast the new player to all existing players
    broadcast_to_all(
        world,
        &InternalPacket::PlayerInfoUpdate {
            actions,
            players: vec![new_entry],
        },
    );

    // Send initial inventory (empty)
    let _ = sender.send(InternalPacket::SetContainerContent {
        window_id: 0,
        state_id: 1,
        slots: vec![None; 46],
        carried_item: None,
    });

    // Spawn ECS entity
    world.spawn((
        EntityId(entity_id),
        Profile(profile.clone()),
        Position(spawn_pos),
        Rotation {
            yaw: 0.0,
            pitch: 0.0,
        },
        OnGround(true),
        PlayerGameMode(GameMode::Survival),
        ConnectionSender(sender),
        ChunkPosition {
            chunk_x: center_cx,
            chunk_z: center_cz,
        },
        ViewDistance(view_distance),
        KeepAlive::new(),
        TrackedEntities::new(),
        PreviousPosition(spawn_pos),
        PreviousRotation {
            yaw: 0.0,
            pitch: 0.0,
        },
        Inventory::new(),
        HeldSlot(0),
    ));

    inbound_receivers.insert(entity_id, new_player.packet_rx);

    // Fire Lua event
    scripting.fire_event_in_context(
        "player_join",
        &[("name", &profile.name)],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );
}

fn handle_disconnect(
    world: &mut World,
    world_state: &mut WorldState,
    inbound_receivers: &mut HashMap<i32, mpsc::UnboundedReceiver<InboundPacket>>,
    entity_id: i32,
    _adapter: &V1_21Adapter,
    scripting: &ScriptRuntime,
) {
    inbound_receivers.remove(&entity_id);

    // Find and remove the entity
    let mut to_despawn = None;
    let mut player_uuid = None;
    let mut player_name = String::new();

    for (e, (eid, profile)) in world.query::<(&EntityId, &Profile)>().iter() {
        if eid.0 == entity_id {
            to_despawn = Some(e);
            player_uuid = Some(profile.0.uuid);
            player_name = profile.0.name.clone();
            break;
        }
    }

    if let Some(entity) = to_despawn {
        let _ = world.despawn(entity);
    }

    if let Some(uuid) = player_uuid {
        info!("{} disconnected", player_name);

        // Broadcast tab list removal
        broadcast_to_all(
            world,
            &InternalPacket::PlayerInfoRemove {
                uuids: vec![uuid],
            },
        );

        // Remove from all players' tracked entities and send despawn
        for (_e, (tracked, sender)) in world
            .query::<(&mut TrackedEntities, &ConnectionSender)>()
            .iter()
        {
            if tracked.visible.remove(&entity_id) {
                let _ = sender.0.send(InternalPacket::RemoveEntities {
                    entity_ids: vec![entity_id],
                });
            }
        }

        // Fire Lua event
        scripting.fire_event_in_context(
            "player_leave",
            &[("name", &player_name)],
            world as *mut _ as *mut (),
            world_state as *mut _ as *mut (),
        );
    }
}

fn process_packet(
    _config: &ServerConfig,
    _adapter: &V1_21Adapter,
    world: &mut World,
    world_state: &mut WorldState,
    pkt: InboundPacket,
    scripting: &ScriptRuntime,
    lua_commands: &crate::bridge::LuaCommands,
) {
    let entity_id = pkt.entity_id;

    // Find the hecs entity for this player
    let entity = {
        let mut found = None;
        for (e, eid) in world.query::<&EntityId>().iter() {
            if eid.0 == entity_id {
                found = Some(e);
                break;
            }
        }
        match found {
            Some(e) => e,
            None => return, // Player already disconnected
        }
    };

    match pkt.packet {
        InternalPacket::ConfirmTeleportation { teleport_id } => {
            debug!("Teleport confirmed: {}", teleport_id);
        }

        InternalPacket::PlayerPosition {
            x,
            y,
            z,
            on_ground,
        } => {
            if let Ok(mut pos) = world.get::<&mut Position>(entity) {
                pos.0 = Vec3d::new(x, y, z);
            }
            if let Ok(mut og) = world.get::<&mut OnGround>(entity) {
                og.0 = on_ground;
            }
            handle_chunk_updates(world, world_state, entity);
            fire_move_event(world, world_state, entity, x, y, z, scripting);
        }

        InternalPacket::PlayerPositionAndRotation {
            x,
            y,
            z,
            yaw,
            pitch,
            on_ground,
        } => {
            if let Ok(mut pos) = world.get::<&mut Position>(entity) {
                pos.0 = Vec3d::new(x, y, z);
            }
            if let Ok(mut rot) = world.get::<&mut Rotation>(entity) {
                rot.yaw = yaw;
                rot.pitch = pitch;
            }
            if let Ok(mut og) = world.get::<&mut OnGround>(entity) {
                og.0 = on_ground;
            }
            handle_chunk_updates(world, world_state, entity);
            fire_move_event(world, world_state, entity, x, y, z, scripting);
        }

        InternalPacket::PlayerRotation {
            yaw,
            pitch,
            on_ground,
        } => {
            if let Ok(mut rot) = world.get::<&mut Rotation>(entity) {
                rot.yaw = yaw;
                rot.pitch = pitch;
            }
            if let Ok(mut og) = world.get::<&mut OnGround>(entity) {
                og.0 = on_ground;
            }
        }

        InternalPacket::PlayerOnGround { on_ground } => {
            if let Ok(mut og) = world.get::<&mut OnGround>(entity) {
                og.0 = on_ground;
            }
        }

        InternalPacket::KeepAliveServerbound { id: ka_id } => {
            if let Ok(mut ka) = world.get::<&mut KeepAlive>(entity) {
                if ka.pending == Some(ka_id) {
                    ka.pending = None;
                    ka.last_response = Instant::now();
                }
            }
        }

        InternalPacket::BlockDig {
            status,
            position,
            sequence,
            ..
        } => {
            let game_mode = world
                .get::<&PlayerGameMode>(entity)
                .map(|gm| gm.0)
                .unwrap_or(GameMode::Survival);

            match status {
                // Started Digging
                0 => {
                    if game_mode == GameMode::Creative {
                        // Creative mode: instant break
                        complete_block_break(
                            world, world_state, entity, entity_id, &position, sequence,
                            scripting,
                        );
                    } else {
                        // Survival mode: check block hardness
                        let block_state = world_state.get_block(&position);
                        let held_item_id = {
                            let slot =
                                world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                            world
                                .get::<&Inventory>(entity)
                                .ok()
                                .and_then(|inv| inv.held_item(slot).as_ref().map(|i| i.item_id))
                        };
                        match calculate_break_ticks(block_state, held_item_id) {
                            None => {
                                // Unbreakable block, just ack
                                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                    let _ = sender.0.send(
                                        InternalPacket::AcknowledgeBlockChange { sequence },
                                    );
                                }
                            }
                            Some(0) => {
                                // Instant break (hardness == 0)
                                complete_block_break(
                                    world, world_state, entity, entity_id, &position,
                                    sequence, scripting,
                                );
                            }
                            Some(ticks) => {
                                // Timed break: insert BreakingBlock component
                                let _ = world.insert_one(
                                    entity,
                                    BreakingBlock {
                                        position,
                                        block_state,
                                        started_tick: world_state.tick_count,
                                        total_ticks: ticks,
                                        last_stage: -1,
                                    },
                                );
                                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                    let _ = sender.0.send(
                                        InternalPacket::AcknowledgeBlockChange { sequence },
                                    );
                                }
                            }
                        }
                    }
                }
                // Cancelled Digging
                1 => {
                    let _ = world.remove_one::<BreakingBlock>(entity);
                    // Clear destroy stage animation
                    broadcast_to_all(
                        world,
                        &InternalPacket::SetBlockDestroyStage {
                            entity_id,
                            position,
                            destroy_stage: -1,
                        },
                    );
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ =
                            sender
                                .0
                                .send(InternalPacket::AcknowledgeBlockChange { sequence });
                    }
                }
                // Finished Digging
                2 => {
                    let valid = if let Ok(breaking) = world.get::<&BreakingBlock>(entity) {
                        let elapsed = world_state
                            .tick_count
                            .saturating_sub(breaking.started_tick);
                        // Allow 2 tick tolerance
                        elapsed + 2 >= breaking.total_ticks
                    } else {
                        false
                    };

                    let _ = world.remove_one::<BreakingBlock>(entity);

                    if valid {
                        complete_block_break(
                            world, world_state, entity, entity_id, &position, sequence,
                            scripting,
                        );
                    } else {
                        // Too fast or no breaking component — just ack without breaking
                        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                            let _ = sender
                                .0
                                .send(InternalPacket::AcknowledgeBlockChange { sequence });
                        }
                    }
                }
                _ => {}
            }
        }

        InternalPacket::BlockPlace {
            position,
            face,
            sequence,
            ..
        } => {
            // Look up the held item to determine what block to place
            let block_id = {
                let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                let inv = world.get::<&Inventory>(entity);
                match inv {
                    Ok(inv) => {
                        match inv.held_item(held_slot) {
                            Some(item) => {
                                pickaxe_data::item_id_to_block_state(item.item_id).unwrap_or(0)
                            }
                            None => 0,
                        }
                    }
                    Err(_) => 0,
                }
            };

            if block_id == 0 {
                // Nothing to place (empty hand or non-block item)
                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                    let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                }
                return;
            }

            let target = offset_by_face(&position, face);

            let name = world
                .get::<&Profile>(entity)
                .map(|p| p.0.name.clone())
                .unwrap_or_default();

            // Fire event BEFORE the place — handlers can cancel
            let cancelled = scripting.fire_event_in_context(
                "block_place",
                &[
                    ("name", &name),
                    ("x", &target.x.to_string()),
                    ("y", &target.y.to_string()),
                    ("z", &target.z.to_string()),
                    ("block_id", &block_id.to_string()),
                ],
                world as *mut _ as *mut (),
                world_state as *mut _ as *mut (),
            );

            if cancelled {
                // Just ack without placing
                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                    let _ = sender
                        .0
                        .send(InternalPacket::AcknowledgeBlockChange { sequence });
                }
                return;
            }

            world_state.set_block(&target, block_id);
            // Send to placing player
            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                let _ = sender.0.send(InternalPacket::BlockUpdate {
                    position: target,
                    block_id,
                });
                let _ = sender
                    .0
                    .send(InternalPacket::AcknowledgeBlockChange { sequence });
            }
            // Broadcast to others
            broadcast_except(
                world,
                entity_id,
                &InternalPacket::BlockUpdate {
                    position: target,
                    block_id,
                },
            );

            debug!("{} placed block at {:?}", name, target);
        }

        InternalPacket::ChatMessage { message, .. } => {
            let name = world
                .get::<&Profile>(entity)
                .map(|p| p.0.name.clone())
                .unwrap_or_default();
            info!("<{}> {}", name, message);

            // Fire Lua event
            let cancelled = scripting.fire_event_in_context(
                "player_chat",
                &[("name", &name), ("message", &message)],
                world as *mut _ as *mut (),
                world_state as *mut _ as *mut (),
            );

            if !cancelled {
                let chat_text = format!("<{}> {}", name, message);
                broadcast_to_all(
                    world,
                    &InternalPacket::SystemChatMessage {
                        content: TextComponent::plain(&chat_text),
                        overlay: false,
                    },
                );
            }
        }

        InternalPacket::ChatCommand { command } => {
            let name = world
                .get::<&Profile>(entity)
                .map(|p| p.0.name.clone())
                .unwrap_or_default();
            info!("{} issued command: /{}", name, command);

            scripting.fire_event_in_context(
                "player_command",
                &[("name", &name), ("command", &command)],
                world as *mut _ as *mut (),
                world_state as *mut _ as *mut (),
            );

            let parts: Vec<&str> = command.splitn(2, ' ').collect();
            let cmd_name = parts[0];
            let args = if parts.len() > 1 { parts[1] } else { "" };

            match cmd_name {
                "gamemode" | "gm" => cmd_gamemode(world, entity, args),
                "tp" | "teleport" => cmd_tp(world, entity, args),
                "give" => cmd_give(world, entity, args),
                "kill" => cmd_kill(world, entity),
                "say" => cmd_say(world, args, &name),
                "help" => cmd_help(world, entity, lua_commands),
                "time" => cmd_time(world, entity, args, world_state),
                _ => {
                    // Check Lua-registered commands
                    let handled = if let Ok(cmds) = lua_commands.lock() {
                        if let Some(lua_cmd) = cmds.iter().find(|c| c.name == cmd_name) {
                            let lua = scripting.lua();
                            // Set game context so bridge APIs work inside command handlers
                            lua.set_app_data(pickaxe_scripting::bridge::LuaGameContext {
                                world_ptr: world as *mut _ as *mut (),
                                world_state_ptr: world_state as *mut _ as *mut (),
                            });
                            let func: mlua::Result<mlua::Function> =
                                lua.registry_value(&lua_cmd.handler_key);
                            let result = if let Ok(func) = func {
                                if let Err(e) = func.call::<()>((name.clone(), args.to_string())) {
                                    warn!("Lua command /{} error: {}", cmd_name, e);
                                    send_message(
                                        world,
                                        entity,
                                        &format!("Command error: {}", e),
                                    );
                                }
                                true
                            } else {
                                false
                            };
                            lua.remove_app_data::<pickaxe_scripting::bridge::LuaGameContext>();
                            result
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !handled {
                        send_message(
                            world,
                            entity,
                            &format!("Unknown command: /{}", cmd_name),
                        );
                    }
                }
            }
        }

        InternalPacket::HeldItemChange { slot } => {
            if (0..=8).contains(&slot) {
                if let Ok(mut held) = world.get::<&mut HeldSlot>(entity) {
                    held.0 = slot as u8;
                }
            }
        }

        InternalPacket::CreativeInventoryAction { slot, item } => {
            if slot >= 0 {
                if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                    inv.set_slot(slot as usize, item);
                }
            }
        }

        InternalPacket::Unknown { .. } => {}
        _ => {}
    }
}

fn fire_move_event(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    x: f64,
    y: f64,
    z: f64,
    scripting: &ScriptRuntime,
) {
    let name = world
        .get::<&Profile>(entity)
        .map(|p| p.0.name.clone())
        .unwrap_or_default();
    scripting.fire_event_in_context(
        "player_move",
        &[
            ("name", &name),
            ("x", &format!("{:.1}", x)),
            ("y", &format!("{:.1}", y)),
            ("z", &format!("{:.1}", z)),
        ],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );
}

fn tick_keep_alive(_adapter: &V1_21Adapter, world: &mut World, tick_count: u64) {
    // Send keep-alive every 300 ticks (15 seconds)
    if tick_count % 300 != 0 {
        return;
    }

    let now = Instant::now();
    let mut to_kick: Vec<i32> = Vec::new();

    for (_e, (eid, ka, sender)) in world
        .query::<(&EntityId, &mut KeepAlive, &ConnectionSender)>()
        .iter()
    {
        // Kick if no response for 30 seconds
        if ka.pending.is_some() && now.duration_since(ka.last_response).as_secs() >= 30 {
            let _ = sender.0.send(InternalPacket::Disconnect {
                reason: TextComponent::plain("Timed out"),
            });
            to_kick.push(eid.0);
            continue;
        }

        // Send new keep-alive
        let ka_id = now.elapsed().as_millis() as i64;
        let _ = sender
            .0
            .send(InternalPacket::KeepAliveClientbound { id: ka_id });
        ka.pending = Some(ka_id);
        ka.last_sent = now;
    }
}

fn tick_entity_tracking(world: &mut World) {
    use std::collections::HashSet;

    // Collect all player data
    let mut player_data: Vec<(hecs::Entity, i32, Vec3d, f32, f32, bool, uuid::Uuid, i32, i32)> =
        Vec::new();
    for (e, (eid, pos, rot, og, profile, cp, _vd)) in world
        .query::<(
            &EntityId,
            &Position,
            &Rotation,
            &OnGround,
            &Profile,
            &ChunkPosition,
            &ViewDistance,
        )>()
        .iter()
    {
        player_data.push((
            e,
            eid.0,
            pos.0,
            rot.yaw,
            rot.pitch,
            og.0,
            profile.0.uuid,
            cp.chunk_x,
            cp.chunk_z,
        ));
    }

    for i in 0..player_data.len() {
        let (observer_entity, _observer_eid, _, _, _, _, _, obs_cx, obs_cz) = player_data[i];

        let obs_vd = match world.get::<&ViewDistance>(observer_entity) {
            Ok(vd) => vd.0,
            Err(_) => continue,
        };

        let mut should_see: HashSet<i32> = HashSet::new();
        for j in 0..player_data.len() {
            if i == j {
                continue;
            }
            let (_, target_eid, _, _, _, _, _, tgt_cx, tgt_cz) = player_data[j];
            if (tgt_cx - obs_cx).abs() <= obs_vd && (tgt_cz - obs_cz).abs() <= obs_vd {
                should_see.insert(target_eid);
            }
        }

        let currently_tracked: HashSet<i32> = match world.get::<&TrackedEntities>(observer_entity) {
            Ok(te) => te.visible.clone(),
            Err(_) => continue,
        };

        let observer_sender = match world.get::<&ConnectionSender>(observer_entity) {
            Ok(s) => s.0.clone(),
            Err(_) => continue,
        };

        // Spawn new entities
        for &eid in should_see.difference(&currently_tracked) {
            if let Some(&(_, _, pos, yaw, pitch, _, uuid, _, _)) =
                player_data.iter().find(|d| d.1 == eid)
            {
                let _ = observer_sender.send(InternalPacket::SpawnEntity {
                    entity_id: eid,
                    entity_uuid: uuid,
                    entity_type: 128, // player entity type in 1.21.1
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                    pitch: degrees_to_angle(pitch),
                    yaw: degrees_to_angle(yaw),
                    head_yaw: degrees_to_angle(yaw),
                    data: 0,
                    velocity_x: 0,
                    velocity_y: 0,
                    velocity_z: 0,
                });
                let _ = observer_sender.send(InternalPacket::SetHeadRotation {
                    entity_id: eid,
                    head_yaw: degrees_to_angle(yaw),
                });
            }
        }

        // Despawn removed entities
        let to_remove: Vec<i32> = currently_tracked.difference(&should_see).copied().collect();
        if !to_remove.is_empty() {
            let _ = observer_sender.send(InternalPacket::RemoveEntities {
                entity_ids: to_remove,
            });
        }

        // Update tracked set
        if let Ok(mut te) = world.get::<&mut TrackedEntities>(observer_entity) {
            te.visible = should_see;
        }
    }
}

fn tick_entity_movement_broadcast(world: &mut World) {
    // Collect entities that moved or rotated
    let mut movers: Vec<(i32, Vec3d, Vec3d, f32, f32, f32, f32, bool)> = Vec::new();

    for (_e, (eid, pos, prev_pos, rot, prev_rot, og)) in world
        .query::<(
            &EntityId,
            &Position,
            &PreviousPosition,
            &Rotation,
            &PreviousRotation,
            &OnGround,
        )>()
        .iter()
    {
        let pos_changed =
            pos.0.x != prev_pos.0.x || pos.0.y != prev_pos.0.y || pos.0.z != prev_pos.0.z;
        let rot_changed = rot.yaw != prev_rot.yaw || rot.pitch != prev_rot.pitch;
        if pos_changed || rot_changed {
            movers.push((
                eid.0,
                pos.0,
                prev_pos.0,
                rot.yaw,
                rot.pitch,
                prev_rot.yaw,
                prev_rot.pitch,
                og.0,
            ));
        }
    }

    // For each mover, send packets to all observers tracking them
    for &(mover_eid, new_pos, old_pos, yaw, pitch, _old_yaw, _old_pitch, on_ground) in &movers {
        let dx = ((new_pos.x - old_pos.x) * 4096.0) as i16;
        let dy = ((new_pos.y - old_pos.y) * 4096.0) as i16;
        let dz = ((new_pos.z - old_pos.z) * 4096.0) as i16;

        let pos_changed = dx != 0 || dy != 0 || dz != 0;

        let needs_teleport = (new_pos.x - old_pos.x).abs() > 8.0
            || (new_pos.y - old_pos.y).abs() > 8.0
            || (new_pos.z - old_pos.z).abs() > 8.0;

        for (_e, (eid, tracked, sender)) in world
            .query::<(&EntityId, &TrackedEntities, &ConnectionSender)>()
            .iter()
        {
            if eid.0 == mover_eid {
                continue;
            }
            if !tracked.visible.contains(&mover_eid) {
                continue;
            }

            if needs_teleport {
                let _ = sender.0.send(InternalPacket::TeleportEntity {
                    entity_id: mover_eid,
                    x: new_pos.x,
                    y: new_pos.y,
                    z: new_pos.z,
                    yaw: degrees_to_angle(yaw),
                    pitch: degrees_to_angle(pitch),
                    on_ground,
                });
            } else if pos_changed {
                let _ = sender.0.send(InternalPacket::UpdateEntityPositionAndRotation {
                    entity_id: mover_eid,
                    delta_x: dx,
                    delta_y: dy,
                    delta_z: dz,
                    yaw: degrees_to_angle(yaw),
                    pitch: degrees_to_angle(pitch),
                    on_ground,
                });
            } else {
                let _ = sender.0.send(InternalPacket::UpdateEntityRotation {
                    entity_id: mover_eid,
                    yaw: degrees_to_angle(yaw),
                    pitch: degrees_to_angle(pitch),
                    on_ground,
                });
            }

            // Always send head rotation
            let _ = sender.0.send(InternalPacket::SetHeadRotation {
                entity_id: mover_eid,
                head_yaw: degrees_to_angle(yaw),
            });
        }
    }

    // Update previous positions and rotations
    for (_e, (pos, prev_pos, rot, prev_rot)) in world
        .query::<(
            &Position,
            &mut PreviousPosition,
            &Rotation,
            &mut PreviousRotation,
        )>()
        .iter()
    {
        prev_pos.0 = pos.0;
        prev_rot.yaw = rot.yaw;
        prev_rot.pitch = rot.pitch;
    }
}

/// Advance world time each tick. Broadcast UpdateTime every 20 ticks (1 second).
fn tick_world_time(world: &World, world_state: &mut WorldState, tick_count: u64) {
    world_state.world_age += 1;
    world_state.time_of_day = (world_state.time_of_day + 1) % 24000;

    // Broadcast time update every 20 ticks (once per second)
    if tick_count % 20 == 0 {
        broadcast_to_all(world, &InternalPacket::UpdateTime {
            world_age: world_state.world_age,
            time_of_day: world_state.time_of_day,
        });
    }
}

/// Calculate how many ticks it takes to break a block in survival mode.
/// Returns None if the block is unbreakable, Some(0) for instant break, Some(ticks) otherwise.
fn calculate_break_ticks(block_state: i32, held_item_id: Option<i32>) -> Option<u64> {
    let (hardness, diggable) = pickaxe_data::block_state_to_hardness(block_state)?;
    if !diggable || hardness < 0.0 {
        return None;
    }
    if hardness == 0.0 {
        return Some(0);
    }
    let has_correct_tool =
        if let Some(required) = pickaxe_data::block_state_to_harvest_tools(block_state) {
            held_item_id
                .map(|id| required.contains(&id))
                .unwrap_or(false)
        } else {
            true
        };
    let seconds = if has_correct_tool {
        hardness * 1.5
    } else {
        hardness * 5.0
    };
    Some((seconds * 20.0).ceil() as u64)
}

/// Complete a block break: fire pre-event, set to air, send updates, handle drops.
/// If the Lua event is cancelled, sends block correction to prevent desync.
fn complete_block_break(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    entity_id: i32,
    position: &BlockPos,
    sequence: i32,
    scripting: &ScriptRuntime,
) {
    let old_block = world_state.get_block(position);

    let name = world
        .get::<&Profile>(entity)
        .map(|p| p.0.name.clone())
        .unwrap_or_default();

    // Fire event BEFORE the break — handlers can cancel
    let cancelled = scripting.fire_event_in_context(
        "block_break",
        &[
            ("name", &name),
            ("x", &position.x.to_string()),
            ("y", &position.y.to_string()),
            ("z", &position.z.to_string()),
            ("block_id", &old_block.to_string()),
        ],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );

    if cancelled {
        // Send block correction (restore original) + ack to prevent desync
        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
            let _ = sender.0.send(InternalPacket::BlockUpdate {
                position: *position,
                block_id: old_block,
            });
            let _ = sender
                .0
                .send(InternalPacket::AcknowledgeBlockChange { sequence });
        }
        // Clear destroy stage animation
        broadcast_to_all(
            world,
            &InternalPacket::SetBlockDestroyStage {
                entity_id,
                position: *position,
                destroy_stage: -1,
            },
        );
        return;
    }

    // Proceed with the break
    world_state.set_block(position, 0);

    // Send block update + ack to the breaking player
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::BlockUpdate {
            position: *position,
            block_id: 0,
        });
        let _ = sender
            .0
            .send(InternalPacket::AcknowledgeBlockChange { sequence });
    }

    // Broadcast block update to other players
    broadcast_except(
        world,
        entity_id,
        &InternalPacket::BlockUpdate {
            position: *position,
            block_id: 0,
        },
    );

    // Clear destroy stage animation for all players
    broadcast_to_all(
        world,
        &InternalPacket::SetBlockDestroyStage {
            entity_id,
            position: *position,
            destroy_stage: -1,
        },
    );

    // Handle drops in survival mode
    let game_mode = world
        .get::<&PlayerGameMode>(entity)
        .map(|gm| gm.0)
        .unwrap_or(GameMode::Survival);

    if game_mode == GameMode::Survival {
        // Check if player has the correct tool for drops
        let has_correct_tool =
            if let Some(required) = pickaxe_data::block_state_to_harvest_tools(old_block) {
                let held_item_id = {
                    let slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                    world
                        .get::<&Inventory>(entity)
                        .ok()
                        .and_then(|inv| inv.held_item(slot).as_ref().map(|i| i.item_id))
                };
                held_item_id
                    .map(|id| required.contains(&id))
                    .unwrap_or(false)
            } else {
                true // No tool requirement
            };

        if has_correct_tool {
            let drops = pickaxe_data::block_state_to_drops(old_block);
            for &drop_item_id in drops {
                // Find first empty slot: hotbar (36-44) then main inventory (9-35)
                let slot_index = {
                    let inv = match world.get::<&Inventory>(entity) {
                        Ok(inv) => inv,
                        Err(_) => continue,
                    };
                    let mut found = None;
                    for i in 36..=44 {
                        if inv.slots[i].is_none() {
                            found = Some(i);
                            break;
                        }
                    }
                    if found.is_none() {
                        for i in 9..=35 {
                            if inv.slots[i].is_none() {
                                found = Some(i);
                                break;
                            }
                        }
                    }
                    found
                };

                if let Some(slot_index) = slot_index {
                    let item = pickaxe_types::ItemStack::new(drop_item_id, 1);
                    let state_id = {
                        let mut inv = match world.get::<&mut Inventory>(entity) {
                            Ok(inv) => inv,
                            Err(_) => continue,
                        };
                        inv.set_slot(slot_index, Some(item.clone()));
                        inv.state_id
                    };

                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::SetContainerSlot {
                            window_id: 0,
                            state_id,
                            slot: slot_index as i16,
                            item: Some(item),
                        });
                    }
                }
                // If inventory full, silently lose the drop (for now)
            }
        }
    }

    debug!("{} broke block at {:?} (was {})", name, position, old_block);
}

/// Update destroy stage animation for all players currently breaking blocks.
fn tick_block_breaking(world: &mut World, tick_count: u64) {
    let mut updates: Vec<(i32, BlockPos, i8)> = Vec::new();
    for (_entity, (eid, breaking)) in world.query::<(&EntityId, &BreakingBlock)>().iter() {
        let elapsed = tick_count.saturating_sub(breaking.started_tick);
        let progress = elapsed as f64 / breaking.total_ticks as f64;
        let stage = (progress * 10.0).min(9.0) as i8;
        if stage != breaking.last_stage {
            updates.push((eid.0, breaking.position, stage));
        }
    }

    for (eid, pos, stage) in &updates {
        broadcast_to_all(
            world,
            &InternalPacket::SetBlockDestroyStage {
                entity_id: *eid,
                position: *pos,
                destroy_stage: *stage,
            },
        );
    }

    for (_entity, breaking) in world.query::<&mut BreakingBlock>().iter() {
        let elapsed = tick_count.saturating_sub(breaking.started_tick);
        let progress = elapsed as f64 / breaking.total_ticks as f64;
        breaking.last_stage = (progress * 10.0).min(9.0) as i8;
    }
}

// ── Command handlers ──────────────────────────────────────────────────

fn cmd_gamemode(world: &mut World, entity: hecs::Entity, args: &str) {
    if !is_op(world, entity) {
        send_message(world, entity, "You don't have permission to use this command.");
        return;
    }

    let mode = match args.trim() {
        "survival" | "s" | "0" => GameMode::Survival,
        "creative" | "c" | "1" => GameMode::Creative,
        "adventure" | "a" | "2" => GameMode::Adventure,
        "spectator" | "sp" | "3" => GameMode::Spectator,
        _ => {
            send_message(
                world,
                entity,
                "Usage: /gamemode <survival|creative|adventure|spectator>",
            );
            return;
        }
    };

    if let Ok(mut gm) = world.get::<&mut PlayerGameMode>(entity) {
        gm.0 = mode;
    }

    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::GameEvent {
            event: 3,
            value: mode.id() as f32,
        });
    }

    let uuid = world
        .get::<&Profile>(entity)
        .map(|p| p.0.uuid)
        .unwrap_or(uuid::Uuid::nil());
    broadcast_to_all(
        world,
        &InternalPacket::PlayerInfoUpdate {
            actions: player_info_actions::UPDATE_GAME_MODE,
            players: vec![PlayerInfoEntry {
                uuid,
                name: None,
                properties: vec![],
                game_mode: Some(mode.id() as i32),
                listed: None,
                ping: None,
                display_name: None,
            }],
        },
    );

    let name = world
        .get::<&Profile>(entity)
        .map(|p| p.0.name.clone())
        .unwrap_or_default();
    send_message(world, entity, &format!("Game mode set to {:?}", mode));
    info!("{} changed game mode to {:?}", name, mode);
}

fn cmd_tp(world: &mut World, entity: hecs::Entity, args: &str) {
    let parts: Vec<&str> = args.split_whitespace().collect();

    let (x, y, z) = match parts.len() {
        3 => {
            let x: f64 = match parts[0].parse() {
                Ok(v) => v,
                Err(_) => {
                    send_message(world, entity, "Invalid x coordinate");
                    return;
                }
            };
            let y: f64 = match parts[1].parse() {
                Ok(v) => v,
                Err(_) => {
                    send_message(world, entity, "Invalid y coordinate");
                    return;
                }
            };
            let z: f64 = match parts[2].parse() {
                Ok(v) => v,
                Err(_) => {
                    send_message(world, entity, "Invalid z coordinate");
                    return;
                }
            };
            (x, y, z)
        }
        1 => {
            let target_name = parts[0];
            let mut found = None;
            for (_e, (profile, pos)) in world.query::<(&Profile, &Position)>().iter() {
                if profile.0.name.eq_ignore_ascii_case(target_name) {
                    found = Some(pos.0);
                    break;
                }
            }
            match found {
                Some(pos) => (pos.x, pos.y, pos.z),
                None => {
                    send_message(
                        world,
                        entity,
                        &format!("Player '{}' not found", target_name),
                    );
                    return;
                }
            }
        }
        _ => {
            send_message(world, entity, "Usage: /tp <x> <y> <z> or /tp <player>");
            return;
        }
    };

    if let Ok(mut pos) = world.get::<&mut Position>(entity) {
        pos.0 = Vec3d::new(x, y, z);
    }

    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SynchronizePlayerPosition {
            position: Vec3d::new(x, y, z),
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
            teleport_id: 2,
        });
    }

    send_message(
        world,
        entity,
        &format!("Teleported to {:.1}, {:.1}, {:.1}", x, y, z),
    );
}

fn cmd_give(world: &mut World, entity: hecs::Entity, args: &str) {
    if !is_op(world, entity) {
        send_message(world, entity, "You don't have permission to use this command.");
        return;
    }

    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() {
        send_message(world, entity, "Usage: /give <item_name> [count]");
        return;
    }

    let item_name = parts[0].strip_prefix("minecraft:").unwrap_or(parts[0]);
    let count = if parts.len() > 1 {
        parts[1].parse::<i8>().unwrap_or(1).max(1)
    } else {
        1
    };

    let item_id = match pickaxe_data::item_name_to_id(item_name) {
        Some(id) => id,
        None => {
            send_message(world, entity, &format!("Unknown item: {}", item_name));
            return;
        }
    };

    let slot_index = {
        let inv = match world.get::<&Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return,
        };
        let mut found = None;
        for i in 36..=44 {
            if inv.slots[i].is_none() {
                found = Some(i);
                break;
            }
        }
        if found.is_none() {
            for i in 9..=35 {
                if inv.slots[i].is_none() {
                    found = Some(i);
                    break;
                }
            }
        }
        match found {
            Some(i) => i,
            None => {
                send_message(world, entity, "Inventory is full!");
                return;
            }
        }
    };

    let state_id = {
        let mut inv = match world.get::<&mut Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return,
        };
        let item = pickaxe_types::ItemStack::new(item_id, count);
        inv.set_slot(slot_index, Some(item));
        inv.state_id
    };

    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SetContainerSlot {
            window_id: 0,
            state_id,
            slot: slot_index as i16,
            item: Some(pickaxe_types::ItemStack::new(item_id, count)),
        });
    }

    let display_name = pickaxe_data::item_id_to_name(item_id).unwrap_or("unknown");
    send_message(
        world,
        entity,
        &format!("Gave {} x{}", display_name, count),
    );
}

fn cmd_kill(world: &mut World, entity: hecs::Entity) {
    let spawn = Vec3d::new(0.5, -59.0, 0.5);
    if let Ok(mut pos) = world.get::<&mut Position>(entity) {
        pos.0 = spawn;
    }
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SynchronizePlayerPosition {
            position: spawn,
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
            teleport_id: 3,
        });
    }
    send_message(world, entity, "Killed! (respawned at spawn)");
}

fn cmd_say(world: &World, message: &str, sender_name: &str) {
    if message.is_empty() {
        return;
    }
    let chat_text = format!("[{}] {}", sender_name, message);
    broadcast_to_all(
        world,
        &InternalPacket::SystemChatMessage {
            content: TextComponent::plain(&chat_text),
            overlay: false,
        },
    );
}

fn cmd_help(world: &World, entity: hecs::Entity, lua_commands: &crate::bridge::LuaCommands) {
    let help_text = [
        "=== Pickaxe Server Commands ===",
        "/gamemode <mode> - Change game mode (survival/creative/adventure/spectator)",
        "/tp <x> <y> <z> - Teleport to coordinates",
        "/tp <player> - Teleport to player",
        "/give <item> [count] - Give item to yourself",
        "/kill - Respawn at spawn point",
        "/say <message> - Broadcast a message",
        "/time set <day|night|noon|midnight|value> - Set time of day",
        "/time add <value> - Add to time of day",
        "/time query [daytime|gametime|day] - Query current time",
        "/help - Show this help",
    ];
    for line in &help_text {
        send_message(world, entity, line);
    }
    if let Ok(cmds) = lua_commands.lock() {
        if !cmds.is_empty() {
            send_message(world, entity, "=== Mod Commands ===");
            for cmd in cmds.iter() {
                send_message(world, entity, &format!("/{}", cmd.name));
            }
        }
    }
}

fn cmd_time(world: &World, entity: hecs::Entity, args: &str, world_state: &mut WorldState) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() {
        send_message(world, entity, "Usage: /time <set|add|query> [value]");
        return;
    }

    match parts[0] {
        "set" => {
            if !is_op(world, entity) {
                send_message(world, entity, "You don't have permission to use this command.");
                return;
            }
            if parts.len() < 2 {
                send_message(world, entity, "Usage: /time set <day|night|noon|midnight|value>");
                return;
            }
            let time = match parts[1] {
                "day" | "sunrise" => 0,
                "noon" => 6000,
                "sunset" => 12000,
                "night" | "midnight" => 18000,
                other => match other.parse::<i64>() {
                    Ok(v) => v.rem_euclid(24000),
                    Err(_) => {
                        send_message(world, entity, "Invalid time value.");
                        return;
                    }
                },
            };
            world_state.time_of_day = time;
            broadcast_to_all(world, &InternalPacket::UpdateTime {
                world_age: world_state.world_age,
                time_of_day: world_state.time_of_day,
            });
            send_message(world, entity, &format!("Set time to {}", time));
        }
        "add" => {
            if !is_op(world, entity) {
                send_message(world, entity, "You don't have permission to use this command.");
                return;
            }
            if parts.len() < 2 {
                send_message(world, entity, "Usage: /time add <value>");
                return;
            }
            let amount = match parts[1].parse::<i64>() {
                Ok(v) => v,
                Err(_) => {
                    send_message(world, entity, "Invalid time value.");
                    return;
                }
            };
            world_state.time_of_day = (world_state.time_of_day + amount).rem_euclid(24000);
            broadcast_to_all(world, &InternalPacket::UpdateTime {
                world_age: world_state.world_age,
                time_of_day: world_state.time_of_day,
            });
            send_message(world, entity, &format!("Added {} to time (now {})", amount, world_state.time_of_day));
        }
        "query" => {
            if parts.len() < 2 {
                send_message(world, entity, &format!("Time of day: {}, World age: {}", world_state.time_of_day, world_state.world_age));
                return;
            }
            match parts[1] {
                "daytime" => send_message(world, entity, &format!("Time of day: {}", world_state.time_of_day)),
                "gametime" => send_message(world, entity, &format!("World age: {}", world_state.world_age)),
                "day" => send_message(world, entity, &format!("Day: {}", world_state.world_age / 24000)),
                _ => send_message(world, entity, "Usage: /time query <daytime|gametime|day>"),
            }
        }
        _ => {
            send_message(world, entity, "Usage: /time <set|add|query> [value]");
        }
    }
}

fn handle_chunk_updates(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
) {
    // Get current position and chunk state
    let (pos, old_cx, old_cz, vd) = {
        let Ok(pos) = world.get::<&Position>(entity) else {
            return;
        };
        let Ok(cp) = world.get::<&ChunkPosition>(entity) else {
            return;
        };
        let Ok(vd) = world.get::<&ViewDistance>(entity) else {
            return;
        };
        (pos.0, cp.chunk_x, cp.chunk_z, vd.0)
    };

    let new_cx = (pos.x as i32) >> 4;
    let new_cz = (pos.z as i32) >> 4;

    if new_cx == old_cx && new_cz == old_cz {
        return;
    }

    // Update chunk position component
    if let Ok(mut cp) = world.get::<&mut ChunkPosition>(entity) {
        cp.chunk_x = new_cx;
        cp.chunk_z = new_cz;
    }

    let Ok(sender) = world.get::<&ConnectionSender>(entity) else {
        return;
    };
    let sender = &sender.0;

    // Send Set Center Chunk
    let _ = sender.send(InternalPacket::SetCenterChunk {
        chunk_x: new_cx,
        chunk_z: new_cz,
    });

    // Unload old chunks
    for cx in (old_cx - vd)..=(old_cx + vd) {
        for cz in (old_cz - vd)..=(old_cz + vd) {
            if (cx - new_cx).abs() > vd || (cz - new_cz).abs() > vd {
                let _ = sender.send(InternalPacket::UnloadChunk {
                    chunk_x: cx,
                    chunk_z: cz,
                });
            }
        }
    }

    // Send new chunks in batch
    send_new_chunks(sender, world_state, old_cx, old_cz, new_cx, new_cz, vd);
}

fn send_chunks_around(
    sender: &mpsc::UnboundedSender<InternalPacket>,
    world_state: &mut WorldState,
    center_cx: i32,
    center_cz: i32,
    view_distance: i32,
) {
    // We need to send chunk batch start/finished via raw packets.
    // Since we're going through the packet channel now, we use a special mechanism.
    // The writer task will handle the batch framing when it sees the ChunkBatchStart/Finished markers.
    let _ = sender.send(InternalPacket::ChunkBatchStart);

    let mut count = 0i32;
    for cx in (center_cx - view_distance)..=(center_cx + view_distance) {
        for cz in (center_cz - view_distance)..=(center_cz + view_distance) {
            let chunk_packet = world_state.get_chunk_packet(cx, cz);
            let _ = sender.send(chunk_packet);
            count += 1;
        }
    }

    let _ = sender.send(InternalPacket::ChunkBatchFinished { batch_size: count });
}

fn send_new_chunks(
    sender: &mpsc::UnboundedSender<InternalPacket>,
    world_state: &mut WorldState,
    old_cx: i32,
    old_cz: i32,
    new_cx: i32,
    new_cz: i32,
    vd: i32,
) {
    let _ = sender.send(InternalPacket::ChunkBatchStart);

    let mut count = 0i32;
    for cx in (new_cx - vd)..=(new_cx + vd) {
        for cz in (new_cz - vd)..=(new_cz + vd) {
            if (cx - old_cx).abs() > vd || (cz - old_cz).abs() > vd {
                let chunk_packet = world_state.get_chunk_packet(cx, cz);
                let _ = sender.send(chunk_packet);
                count += 1;
            }
        }
    }

    let _ = sender.send(InternalPacket::ChunkBatchFinished { batch_size: count });
}

/// Send a packet to all players.
fn broadcast_to_all(world: &World, packet: &InternalPacket) {
    for (_e, sender) in world.query::<&ConnectionSender>().iter() {
        let _ = sender.0.send(packet.clone());
    }
}

/// Send a packet to all players except the one with the given entity ID.
fn broadcast_except(world: &World, except_eid: i32, packet: &InternalPacket) {
    for (_e, (eid, sender)) in world.query::<(&EntityId, &ConnectionSender)>().iter() {
        if eid.0 != except_eid {
            let _ = sender.0.send(packet.clone());
        }
    }
}

/// Offset a block position by the given face direction.
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

/// Build the Declare Commands packet with the full command tree.
fn build_command_tree(lua_commands: &crate::bridge::LuaCommands) -> InternalPacket {
    let mut nodes: Vec<CommandNode> = Vec::new();

    // Helper: create a literal node (type=1)
    let lit = |name: &str, executable: bool, children: Vec<i32>| -> CommandNode {
        CommandNode {
            flags: 0x01 | if executable { 0x04 } else { 0 },
            children,
            name: Some(name.to_string()),
            parser: None,
            parser_properties: None,
        }
    };

    // Node 0: Root
    // Children will be filled in after we know all top-level command indices.
    nodes.push(CommandNode {
        flags: 0x00,
        children: vec![],
        name: None,
        parser: None,
        parser_properties: None,
    });

    // Simple commands: literal + executable, no subcommands
    let simple_cmds = ["gamemode", "gm", "tp", "teleport", "give", "kill", "say", "help"];
    let mut root_children: Vec<i32> = Vec::new();
    for cmd in &simple_cmds {
        let idx = nodes.len() as i32;
        root_children.push(idx);
        nodes.push(lit(cmd, true, vec![]));
    }

    // /time command with subcommands
    // /time set <day|night|noon|midnight|sunset|sunrise|value>
    // /time add <value>
    // /time query <daytime|gametime|day>

    // time set options
    let set_opts = ["day", "night", "noon", "midnight", "sunset", "sunrise"];
    let mut set_children: Vec<i32> = Vec::new();
    for opt in &set_opts {
        let idx = nodes.len() as i32;
        set_children.push(idx);
        nodes.push(lit(opt, true, vec![]));
    }
    let set_idx = nodes.len() as i32;
    nodes.push(lit("set", false, set_children));

    // time add (executable — takes a number typed by user)
    let add_idx = nodes.len() as i32;
    nodes.push(lit("add", true, vec![]));

    // time query options
    let query_opts = ["daytime", "gametime", "day"];
    let mut query_children: Vec<i32> = Vec::new();
    for opt in &query_opts {
        let idx = nodes.len() as i32;
        query_children.push(idx);
        nodes.push(lit(opt, true, vec![]));
    }
    let query_idx = nodes.len() as i32;
    nodes.push(lit("query", true, query_children));

    let time_idx = nodes.len() as i32;
    root_children.push(time_idx);
    nodes.push(lit("time", false, vec![set_idx, add_idx, query_idx]));

    // Add Lua-registered commands
    if let Ok(cmds) = lua_commands.lock() {
        for cmd in cmds.iter() {
            let idx = nodes.len() as i32;
            root_children.push(idx);
            nodes.push(lit(&cmd.name, true, vec![]));
        }
    }

    // Patch root children
    nodes[0].children = root_children;

    InternalPacket::DeclareCommands {
        nodes,
        root_index: 0,
    }
}

/// Send a system chat message to a specific player entity.
fn send_message(world: &World, entity: hecs::Entity, message: &str) {
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SystemChatMessage {
            content: TextComponent::plain(message),
            overlay: false,
        });
    }
}

/// Check if a player is an operator.
/// Re-reads config/ops.toml so changes take effect without a restart.
fn is_op(world: &World, entity: hecs::Entity) -> bool {
    let name = match world.get::<&Profile>(entity) {
        Ok(p) => p.0.name.clone(),
        Err(_) => return false,
    };
    let ops = crate::config::load_ops();
    ops.iter().any(|op| op.eq_ignore_ascii_case(&name))
}

/// Get the player count.
pub fn player_count(world: &World) -> usize {
    world.query::<&Profile>().iter().count()
}

/// Convert degrees to MC protocol angle (256ths of a turn).
fn degrees_to_angle(degrees: f32) -> u8 {
    ((degrees / 360.0) * 256.0) as i32 as u8
}
