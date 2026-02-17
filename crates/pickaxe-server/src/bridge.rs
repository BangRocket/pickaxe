use crate::ecs::*;
use hecs::World;
use mlua::Lua;
use pickaxe_protocol_core::InternalPacket;
use pickaxe_scripting::bridge::LuaGameContext;
use pickaxe_types::{BlockPos, GameMode, ItemStack, TextComponent, Vec3d};
use rand::Rng;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

/// A command registered by a Lua mod.
pub struct LuaCommand {
    pub name: String,
    pub handler_key: mlua::RegistryKey,
}

/// Shared storage for Lua-registered commands.
pub type LuaCommands = Arc<Mutex<Vec<LuaCommand>>>;

/// Override for a block's properties (hardness, drops, harvest tools).
pub struct BlockOverride {
    pub hardness: Option<f64>,
    pub drops: Option<Vec<i32>>,
    pub harvest_tools: Option<Vec<i32>>,
}

/// Shared storage for Lua block overrides, keyed by block name.
pub type BlockOverrides = Arc<Mutex<HashMap<String, BlockOverride>>>;

fn lua_err(e: mlua::Error) -> anyhow::Error {
    anyhow::anyhow!("{}", e)
}

/// Helper to get the game context from app_data.
fn get_context(lua: &Lua) -> mlua::Result<mlua::AppDataRef<'_, LuaGameContext>> {
    lua.app_data_ref::<LuaGameContext>()
        .ok_or_else(|| mlua::Error::runtime("Game API not available outside event handlers"))
}

/// Helper to run a closure with access to `&mut WorldState` from app_data.
fn with_world_state<F, R>(lua: &Lua, f: F) -> mlua::Result<R>
where
    F: FnOnce(&mut crate::tick::WorldState) -> R,
{
    let ctx = get_context(lua)?;
    let ws = unsafe { &mut *(ctx.world_state_ptr as *mut crate::tick::WorldState) };
    Ok(f(ws))
}

/// Helper to run a closure with access to `&mut hecs::World` from app_data.
fn with_world<F, R>(lua: &Lua, f: F) -> mlua::Result<R>
where
    F: FnOnce(&mut World) -> R,
{
    let ctx = get_context(lua)?;
    let world = unsafe { &mut *(ctx.world_ptr as *mut World) };
    Ok(f(world))
}

/// Helper to run a closure with access to both `&mut World` and `&mut WorldState`.
fn with_game<F, R>(lua: &Lua, f: F) -> mlua::Result<R>
where
    F: FnOnce(&mut World, &mut crate::tick::WorldState) -> R,
{
    let ctx = get_context(lua)?;
    let world = unsafe { &mut *(ctx.world_ptr as *mut World) };
    let ws = unsafe { &mut *(ctx.world_state_ptr as *mut crate::tick::WorldState) };
    Ok(f(world, ws))
}

/// Find a player entity by name.
fn find_player_by_name(world: &World, name: &str) -> Option<hecs::Entity> {
    world
        .query::<&Profile>()
        .iter()
        .find(|(_, p)| p.0.name.eq_ignore_ascii_case(name))
        .map(|(e, _)| e)
}

/// Give an item to a player entity, returning true on success.
/// Stacks into existing matching slots before using empty ones.
fn give_item_to_player(world: &mut World, entity: hecs::Entity, item_id: i32, count: i8) -> bool {
    let max_stack = pickaxe_data::item_id_to_stack_size(item_id).unwrap_or(64);
    let slot_index = {
        let inv = match world.get::<&Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return false,
        };
        match inv.find_slot_for_item(item_id, max_stack) {
            Some(i) => i,
            None => return false, // inventory full
        }
    };

    let (item, state_id) = {
        let mut inv = match world.get::<&mut Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return false,
        };
        let new_item = match &inv.slots[slot_index] {
            Some(existing) => pickaxe_types::ItemStack::new(
                item_id,
                existing.count.saturating_add(count),
            ),
            None => pickaxe_types::ItemStack::new(item_id, count),
        };
        inv.set_slot(slot_index, Some(new_item.clone()));
        (new_item, inv.state_id)
    };

    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SetContainerSlot {
            window_id: 0,
            state_id,
            slot: slot_index as i16,
            item: Some(item),
        });
    }

    true
}

// ── World API ──────────────────────────────────────────────────────────

/// Register `pickaxe.world` API on the Lua VM.
pub fn register_world_api(lua: &Lua) -> anyhow::Result<()> {
    let pickaxe: mlua::Table = lua.globals().get("pickaxe").map_err(lua_err)?;
    let world_table = lua.create_table().map_err(lua_err)?;

    world_table
        .set(
            "get_block",
            lua.create_function(|lua, (x, y, z): (i32, i32, i32)| {
                with_world_state(lua, |ws| ws.get_block(&BlockPos::new(x, y, z)))
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    world_table
        .set(
            "set_block",
            lua.create_function(|lua, (x, y, z, state_id): (i32, i32, i32, i32)| {
                with_world_state(lua, |ws| ws.set_block(&BlockPos::new(x, y, z), state_id))
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    world_table
        .set(
            "get_time",
            lua.create_function(|lua, ()| with_world_state(lua, |ws| ws.time_of_day))
                .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    world_table
        .set(
            "set_time",
            lua.create_function(|lua, time: i64| {
                with_world_state(lua, |ws| {
                    ws.time_of_day = time.rem_euclid(24000);
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    pickaxe.set("world", world_table).map_err(lua_err)?;
    Ok(())
}

// ── Players API ────────────────────────────────────────────────────────

/// Register `pickaxe.players` API on the Lua VM.
pub fn register_players_api(lua: &Lua) -> anyhow::Result<()> {
    let pickaxe: mlua::Table = lua.globals().get("pickaxe").map_err(lua_err)?;
    let players_table = lua.create_table().map_err(lua_err)?;

    // pickaxe.players.list() -> {"Steve", "Alex", ...}
    players_table
        .set(
            "list",
            lua.create_function(|lua, ()| {
                with_world(lua, |world| {
                    let names: Vec<String> = world
                        .query::<&Profile>()
                        .iter()
                        .map(|(_, p)| p.0.name.clone())
                        .collect();
                    names
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.get(name) -> {name, x, y, z, game_mode, held_slot} or nil
    players_table
        .set(
            "get",
            lua.create_function(|lua, name: String| {
                with_world(lua, |world| -> Option<mlua::Value> {
                    let entity = find_player_by_name(world, &name)?;
                    let pos = world.get::<&Position>(entity).ok()?;
                    let gm = world.get::<&PlayerGameMode>(entity).ok()?;
                    let held = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);

                    let table = lua.create_table().ok()?;
                    let _ = table.set("name", world.get::<&Profile>(entity).ok()?.0.name.clone());
                    let _ = table.set("x", pos.0.x);
                    let _ = table.set("y", pos.0.y);
                    let _ = table.set("z", pos.0.z);
                    let _ = table.set(
                        "game_mode",
                        match gm.0 {
                            GameMode::Survival => "survival",
                            GameMode::Creative => "creative",
                            GameMode::Adventure => "adventure",
                            GameMode::Spectator => "spectator",
                        },
                    );
                    let _ = table.set("held_slot", held);
                    // Health and food data
                    if let Ok(h) = world.get::<&Health>(entity) {
                        let _ = table.set("health", h.current);
                        let _ = table.set("max_health", h.max);
                    }
                    if let Ok(f) = world.get::<&FoodData>(entity) {
                        let _ = table.set("food", f.food_level);
                        let _ = table.set("saturation", f.saturation);
                        let _ = table.set("exhaustion", f.exhaustion);
                    }
                    Some(mlua::Value::Table(table))
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.teleport(name, x, y, z) -> bool
    players_table
        .set(
            "teleport",
            lua.create_function(|lua, (name, x, y, z): (String, f64, f64, f64)| {
                with_world(lua, |world| {
                    let entity = match find_player_by_name(world, &name) {
                        Some(e) => e,
                        None => return false,
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
                            teleport_id: 99,
                        });
                    }
                    true
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.set_game_mode(name, mode) -> bool
    players_table
        .set(
            "set_game_mode",
            lua.create_function(|lua, (name, mode_str): (String, String)| {
                with_world(lua, |world| {
                    let mode = match mode_str.as_str() {
                        "survival" | "s" | "0" => GameMode::Survival,
                        "creative" | "c" | "1" => GameMode::Creative,
                        "adventure" | "a" | "2" => GameMode::Adventure,
                        "spectator" | "sp" | "3" => GameMode::Spectator,
                        _ => return false,
                    };
                    let entity = match find_player_by_name(world, &name) {
                        Some(e) => e,
                        None => return false,
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
                    true
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.send_message(name, text) -> bool
    players_table
        .set(
            "send_message",
            lua.create_function(|lua, (name, text): (String, String)| {
                with_world(lua, |world| {
                    let entity = match find_player_by_name(world, &name) {
                        Some(e) => e,
                        None => return false,
                    };
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::SystemChatMessage {
                            content: TextComponent::plain(&text),
                            overlay: false,
                        });
                        true
                    } else {
                        false
                    }
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.broadcast(text)
    players_table
        .set(
            "broadcast",
            lua.create_function(|lua, text: String| {
                with_world(lua, |world| {
                    let packet = InternalPacket::SystemChatMessage {
                        content: TextComponent::plain(&text),
                        overlay: false,
                    };
                    for (_e, sender) in world.query::<&ConnectionSender>().iter() {
                        let _ = sender.0.send(packet.clone());
                    }
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.give(name, item_name, count) -> bool
    players_table
        .set(
            "give",
            lua.create_function(|lua, (name, item_name, count): (String, String, Option<i8>)| {
                with_world(lua, |world| {
                    let item_name = item_name
                        .strip_prefix("minecraft:")
                        .unwrap_or(&item_name);
                    let item_id = match pickaxe_data::item_name_to_id(item_name) {
                        Some(id) => id,
                        None => return false,
                    };
                    let entity = match find_player_by_name(world, &name) {
                        Some(e) => e,
                        None => return false,
                    };
                    give_item_to_player(world, entity, item_id, count.unwrap_or(1).max(1))
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.is_op(name) -> bool
    players_table
        .set(
            "is_op",
            lua.create_function(|_lua, name: String| {
                let ops = crate::config::load_ops();
                Ok(ops.iter().any(|op| op.eq_ignore_ascii_case(&name)))
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.get_health(name) -> {health, max_health, food, saturation, exhaustion} or nil
    players_table
        .set(
            "get_health",
            lua.create_function(|lua, name: String| {
                with_world(lua, |world| -> Option<mlua::Value> {
                    let entity = find_player_by_name(world, &name)?;
                    let h = world.get::<&Health>(entity).ok()?;
                    let f = world.get::<&FoodData>(entity).ok()?;
                    let table = lua.create_table().ok()?;
                    let _ = table.set("health", h.current);
                    let _ = table.set("max_health", h.max);
                    let _ = table.set("food", f.food_level);
                    let _ = table.set("saturation", f.saturation);
                    let _ = table.set("exhaustion", f.exhaustion);
                    Some(mlua::Value::Table(table))
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.set_health(name, health) -> bool
    players_table
        .set(
            "set_health",
            lua.create_function(|lua, (name, health): (String, f32)| {
                with_world(lua, |world| {
                    let entity = match find_player_by_name(world, &name) {
                        Some(e) => e,
                        None => return false,
                    };
                    let max = world.get::<&Health>(entity).map(|h| h.max).unwrap_or(20.0);
                    if let Ok(mut h) = world.get::<&mut Health>(entity) {
                        h.current = health.clamp(0.0, max);
                    }
                    // Send update to client
                    let (food, sat) = world
                        .get::<&FoodData>(entity)
                        .map(|f| (f.food_level, f.saturation))
                        .unwrap_or((20, 5.0));
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::SetHealth {
                            health: health.clamp(0.0, max),
                            food,
                            saturation: sat,
                        });
                    }
                    true
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.set_food(name, food, saturation?) -> bool
    players_table
        .set(
            "set_food",
            lua.create_function(|lua, (name, food, saturation): (String, i32, Option<f32>)| {
                with_world(lua, |world| {
                    let entity = match find_player_by_name(world, &name) {
                        Some(e) => e,
                        None => return false,
                    };
                    let sat = saturation.unwrap_or_else(|| {
                        world.get::<&FoodData>(entity).map(|f| f.saturation).unwrap_or(5.0)
                    });
                    if let Ok(mut f) = world.get::<&mut FoodData>(entity) {
                        f.food_level = food.clamp(0, 20);
                        f.saturation = sat.clamp(0.0, food as f32);
                    }
                    // Send update to client
                    let health = world.get::<&Health>(entity).map(|h| h.current).unwrap_or(20.0);
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::SetHealth {
                            health,
                            food: food.clamp(0, 20),
                            saturation: sat.clamp(0.0, food as f32),
                        });
                    }
                    true
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.damage(name, amount, source?) -> bool
    // Respects invulnerability, triggers death if health reaches 0.
    // Does NOT fire player_damage Lua event (caller IS Lua — avoids re-entrancy).
    players_table
        .set(
            "damage",
            lua.create_function(|lua, (name, amount, source): (String, f32, Option<String>)| {
                with_world(lua, |world| {
                    let entity = match find_player_by_name(world, &name) {
                        Some(e) => e,
                        None => return false,
                    };
                    let source_str = source.as_deref().unwrap_or("lua");

                    // Check creative immunity (except void)
                    let gm = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
                    if gm == GameMode::Creative && source_str != "void" {
                        return false;
                    }

                    // Check invulnerability (MC: invulnerableTime > 10)
                    let invuln = world.get::<&Health>(entity).map(|h| h.invulnerable_ticks > 10).unwrap_or(false);
                    if invuln {
                        return false;
                    }

                    // Apply damage
                    let (new_health, is_dead) = {
                        let mut h = match world.get::<&mut Health>(entity) {
                            Ok(h) => h,
                            Err(_) => return false,
                        };
                        h.current = (h.current - amount).max(0.0);
                        h.invulnerable_ticks = 20;
                        (h.current, h.current <= 0.0)
                    };

                    // Damage exhaustion (MC: 0.1 per damage event)
                    if let Ok(mut f) = world.get::<&mut FoodData>(entity) {
                        f.exhaustion = (f.exhaustion + 0.1).min(40.0);
                    }

                    // Send health update
                    let (food, sat) = world
                        .get::<&FoodData>(entity)
                        .map(|f| (f.food_level, f.saturation))
                        .unwrap_or((20, 5.0));
                    let eid = world.get::<&EntityId>(entity).map(|e| e.0).unwrap_or(0);
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::SetHealth {
                            health: new_health,
                            food,
                            saturation: sat,
                        });
                    }

                    // Broadcast hurt animation
                    let hurt = InternalPacket::HurtAnimation { entity_id: eid, yaw: 0.0 };
                    for (_e, sender) in world.query::<&ConnectionSender>().iter() {
                        let _ = sender.0.send(hurt.clone());
                    }

                    // Handle death
                    if is_dead {
                        let death_msg = format!("{} died", name);
                        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                            let _ = sender.0.send(InternalPacket::PlayerCombatKill {
                                player_id: eid,
                                message: TextComponent::plain(&death_msg),
                            });
                        }
                        let death_event = InternalPacket::EntityEvent { entity_id: eid, event_id: 3 };
                        let death_chat = InternalPacket::SystemChatMessage {
                            content: TextComponent::plain(&death_msg),
                            overlay: false,
                        };
                        for (_e, sender) in world.query::<&ConnectionSender>().iter() {
                            let _ = sender.0.send(death_event.clone());
                            let _ = sender.0.send(death_chat.clone());
                        }
                    }

                    true
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.heal(name, amount) -> bool
    players_table
        .set(
            "heal",
            lua.create_function(|lua, (name, amount): (String, f32)| {
                with_world(lua, |world| {
                    let entity = match find_player_by_name(world, &name) {
                        Some(e) => e,
                        None => return false,
                    };
                    let max = world.get::<&Health>(entity).map(|h| h.max).unwrap_or(20.0);
                    if let Ok(mut h) = world.get::<&mut Health>(entity) {
                        h.current = (h.current + amount).min(max);
                    }
                    let (health, food, sat) = {
                        let h = world.get::<&Health>(entity).map(|h| h.current).unwrap_or(20.0);
                        let (f, s) = world.get::<&FoodData>(entity).map(|f| (f.food_level, f.saturation)).unwrap_or((20, 5.0));
                        (h, f, s)
                    };
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::SetHealth { health, food, saturation: sat });
                    }
                    true
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.get_exhaustion(name) -> number or nil
    players_table
        .set(
            "get_exhaustion",
            lua.create_function(|lua, name: String| {
                with_world(lua, |world| -> Option<f32> {
                    let entity = find_player_by_name(world, &name)?;
                    let f = world.get::<&FoodData>(entity).ok()?;
                    Some(f.exhaustion)
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.set_exhaustion(name, value) -> bool
    players_table
        .set(
            "set_exhaustion",
            lua.create_function(|lua, (name, value): (String, f32)| {
                with_world(lua, |world| {
                    let entity = match find_player_by_name(world, &name) {
                        Some(e) => e,
                        None => return false,
                    };
                    if let Ok(mut f) = world.get::<&mut FoodData>(entity) {
                        f.exhaustion = value.clamp(0.0, 40.0);
                    }
                    true
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.players.feed(name, nutrition, saturation_modifier) -> bool
    // MC formula: foodLevel = clamp(foodLevel + nutrition, 0, 20)
    //             saturation = clamp(saturation + nutrition * saturation_modifier * 2.0, 0, foodLevel)
    players_table
        .set(
            "feed",
            lua.create_function(|lua, (name, nutrition, sat_mod): (String, i32, f32)| {
                with_world(lua, |world| {
                    let entity = match find_player_by_name(world, &name) {
                        Some(e) => e,
                        None => return false,
                    };
                    if let Ok(mut f) = world.get::<&mut FoodData>(entity) {
                        f.food_level = (f.food_level + nutrition).clamp(0, 20);
                        f.saturation = (f.saturation + nutrition as f32 * sat_mod * 2.0)
                            .clamp(0.0, f.food_level as f32);
                    }
                    // Send update to client
                    let (health, food, sat) = {
                        let h = world.get::<&Health>(entity).map(|h| h.current).unwrap_or(20.0);
                        let (fl, s) = world
                            .get::<&FoodData>(entity)
                            .map(|f| (f.food_level, f.saturation))
                            .unwrap_or((20, 5.0));
                        (h, fl, s)
                    };
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::SetHealth {
                            health,
                            food,
                            saturation: sat,
                        });
                    }
                    true
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    pickaxe.set("players", players_table).map_err(lua_err)?;
    Ok(())
}

// ── Commands API ──────────────────────────────────────────────────────

/// Register `pickaxe.commands` API on the Lua VM.
pub fn register_commands_api(lua: &Lua, lua_commands: LuaCommands) -> anyhow::Result<()> {
    let pickaxe: mlua::Table = lua.globals().get("pickaxe").map_err(lua_err)?;
    let commands_table = lua.create_table().map_err(lua_err)?;

    // pickaxe.commands.register(name, handler)
    commands_table
        .set(
            "register",
            lua.create_function(move |lua, (name, handler): (String, mlua::Function)| {
                let key = lua
                    .create_registry_value(handler)
                    .map_err(|e| mlua::Error::runtime(format!("Failed to store handler: {}", e)))?;
                let mut cmds = lua_commands
                    .lock()
                    .map_err(|e| mlua::Error::runtime(format!("Lock poisoned: {}", e)))?;
                cmds.push(LuaCommand {
                    name: name.clone(),
                    handler_key: key,
                });
                Ok(())
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    pickaxe.set("commands", commands_table).map_err(lua_err)?;
    Ok(())
}

// ── Blocks API ────────────────────────────────────────────────────────

/// Register `pickaxe.blocks` API on the Lua VM.
pub fn register_blocks_api(lua: &Lua, overrides: BlockOverrides) -> anyhow::Result<()> {
    let pickaxe: mlua::Table = lua.globals().get("pickaxe").map_err(lua_err)?;
    let blocks_table = lua.create_table().map_err(lua_err)?;

    // pickaxe.blocks.register(name, props)
    // props = { hardness = 1.5, drops = {"cobblestone"}, harvest_tools = {"wooden_pickaxe", ...} }
    let overrides_clone = overrides.clone();
    blocks_table
        .set(
            "register",
            lua.create_function(move |_lua, (name, props): (String, mlua::Table)| {
                let hardness: Option<f64> = props.get("hardness").unwrap_or(None);

                let drops: Option<Vec<i32>> = match props.get::<Option<mlua::Table>>("drops") {
                    Ok(Some(tbl)) => {
                        let mut ids = Vec::new();
                        for pair in tbl.sequence_values::<String>() {
                            if let Ok(item_name) = pair {
                                let clean = item_name
                                    .strip_prefix("minecraft:")
                                    .unwrap_or(&item_name)
                                    .to_string();
                                if let Some(id) = pickaxe_data::item_name_to_id(&clean) {
                                    ids.push(id);
                                }
                            }
                        }
                        Some(ids)
                    }
                    _ => None,
                };

                let harvest_tools: Option<Vec<i32>> =
                    match props.get::<Option<mlua::Table>>("harvest_tools") {
                        Ok(Some(tbl)) => {
                            let mut ids = Vec::new();
                            for pair in tbl.sequence_values::<String>() {
                                if let Ok(tool_name) = pair {
                                    let clean = tool_name
                                        .strip_prefix("minecraft:")
                                        .unwrap_or(&tool_name)
                                        .to_string();
                                    if let Some(id) = pickaxe_data::item_name_to_id(&clean) {
                                        ids.push(id);
                                    }
                                }
                            }
                            Some(ids)
                        }
                        _ => None,
                    };

                let mut map = overrides_clone
                    .lock()
                    .map_err(|e| mlua::Error::runtime(format!("Lock poisoned: {}", e)))?;
                map.insert(
                    name,
                    BlockOverride {
                        hardness,
                        drops,
                        harvest_tools,
                    },
                );
                Ok(())
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.blocks.get_hardness(name) -> number or nil
    let overrides_clone = overrides.clone();
    blocks_table
        .set(
            "get_hardness",
            lua.create_function(move |_lua, name: String| {
                let map = overrides_clone
                    .lock()
                    .map_err(|e| mlua::Error::runtime(format!("Lock poisoned: {}", e)))?;
                Ok(map.get(&name).and_then(|o| o.hardness))
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.blocks.get_drops(name) -> {item_name, ...} or nil
    let overrides_clone = overrides.clone();
    blocks_table
        .set(
            "get_drops",
            lua.create_function(move |_lua, name: String| {
                let map = overrides_clone
                    .lock()
                    .map_err(|e| mlua::Error::runtime(format!("Lock poisoned: {}", e)))?;
                Ok(map.get(&name).and_then(|o| {
                    o.drops.as_ref().map(|ids| {
                        ids.iter()
                            .filter_map(|id| pickaxe_data::item_id_to_name(*id))
                            .map(|s| s.to_string())
                            .collect::<Vec<String>>()
                    })
                }))
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    pickaxe.set("blocks", blocks_table).map_err(lua_err)?;
    Ok(())
}

// ── Entities API ──────────────────────────────────────────────────────

/// Helper context that also includes next_eid for entity spawning.
pub struct LuaEntitiesContext {
    pub next_eid: Arc<AtomicI32>,
}

/// Register `pickaxe.entities` API on the Lua VM.
pub fn register_entities_api(lua: &Lua, next_eid: Arc<AtomicI32>) -> anyhow::Result<()> {
    let pickaxe: mlua::Table = lua.globals().get("pickaxe").map_err(lua_err)?;
    let entities_table = lua.create_table().map_err(lua_err)?;

    // Store next_eid in app data for entity spawning
    lua.set_app_data(LuaEntitiesContext {
        next_eid: next_eid.clone(),
    });

    // pickaxe.entities.spawn_item(x, y, z, item_name, count) -> entity_id or nil
    entities_table
        .set(
            "spawn_item",
            lua.create_function(
                |lua, (x, y, z, item_name, count): (f64, f64, f64, String, Option<i8>)| {
                    let item_name = item_name
                        .strip_prefix("minecraft:")
                        .unwrap_or(&item_name)
                        .to_string();
                    let item_id = match pickaxe_data::item_name_to_id(&item_name) {
                        Some(id) => id,
                        None => return Ok(mlua::Value::Nil),
                    };
                    let count = count.unwrap_or(1).max(1);

                    let next_eid = lua
                        .app_data_ref::<LuaEntitiesContext>()
                        .ok_or_else(|| mlua::Error::runtime("Entities context not available"))?
                        .next_eid
                        .clone();

                    let ctx = get_context(lua)?;
                    let world = unsafe { &mut *(ctx.world_ptr as *mut World) };
                    let ws = unsafe { &mut *(ctx.world_state_ptr as *mut crate::tick::WorldState) };

                    // We need a scripting reference, but we can't get it from here easily.
                    // Instead, directly spawn the entity without firing the Lua event
                    // (the caller is already in Lua context).
                    let eid = next_eid.fetch_add(1, Ordering::Relaxed);
                    let uuid = uuid::Uuid::new_v4();

                    let mut rng = rand::thread_rng();
                    let vx = rng.gen_range(-0.1..0.1);
                    let vz = rng.gen_range(-0.1..0.1);

                    world.spawn((
                        EntityId(eid),
                        EntityUuid(uuid),
                        Position(Vec3d::new(x, y, z)),
                        PreviousPosition(Vec3d::new(x, y, z)),
                        Velocity(Vec3d::new(vx, 0.2, vz)),
                        OnGround(false),
                        ItemEntity {
                            item: ItemStack::new(item_id, count),
                            pickup_delay: 10,
                            age: 0,
                        },
                        Rotation {
                            yaw: 0.0,
                            pitch: 0.0,
                        },
                    ));

                    let _ = ws; // touched for consistency
                    Ok(mlua::Value::Integer(eid as i64))
                },
            )
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.entities.remove(entity_id) -> bool
    entities_table
        .set(
            "remove",
            lua.create_function(|lua, entity_id: i32| {
                with_world(lua, |world| {
                    let mut found = None;
                    for (e, eid) in world.query::<&EntityId>().iter() {
                        if eid.0 == entity_id {
                            // Only remove non-player entities
                            if world.get::<&Profile>(e).is_err() {
                                found = Some(e);
                            }
                            break;
                        }
                    }
                    if let Some(entity) = found {
                        // Remove from tracked entities
                        for (_e, tracked) in world.query::<&mut TrackedEntities>().iter() {
                            tracked.visible.remove(&entity_id);
                        }
                        // Broadcast removal
                        for (_e, sender) in world.query::<&ConnectionSender>().iter() {
                            let _ = sender.0.send(InternalPacket::RemoveEntities {
                                entity_ids: vec![entity_id],
                            });
                        }
                        let _ = world.despawn(entity);
                        true
                    } else {
                        false
                    }
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.entities.get(entity_id) -> table or nil
    entities_table
        .set(
            "get",
            lua.create_function(|lua, entity_id: i32| {
                with_world(lua, |world| -> Option<mlua::Value> {
                    for (e, eid) in world.query::<&EntityId>().iter() {
                        if eid.0 == entity_id {
                            let table = lua.create_table().ok()?;
                            let _ = table.set("id", entity_id);

                            if let Ok(pos) = world.get::<&Position>(e) {
                                let _ = table.set("x", pos.0.x);
                                let _ = table.set("y", pos.0.y);
                                let _ = table.set("z", pos.0.z);
                            }

                            if world.get::<&Profile>(e).is_ok() {
                                let _ = table.set("type", "player");
                            } else if let Ok(item_ent) = world.get::<&ItemEntity>(e) {
                                let _ = table.set("type", "item");
                                let _ = table.set("item_id", item_ent.item.item_id);
                                let _ = table.set("item_count", item_ent.item.count);
                                let item_name =
                                    pickaxe_data::item_id_to_name(item_ent.item.item_id)
                                        .unwrap_or("unknown");
                                let _ = table.set("item_name", item_name);
                                let _ = table.set("age", item_ent.age);
                            }

                            return Some(mlua::Value::Table(table));
                        }
                    }
                    None
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.entities.set_velocity(entity_id, vx, vy, vz) -> bool
    entities_table
        .set(
            "set_velocity",
            lua.create_function(|lua, (entity_id, vx, vy, vz): (i32, f64, f64, f64)| {
                with_world(lua, |world| {
                    for (e, eid) in world.query::<&EntityId>().iter() {
                        if eid.0 == entity_id {
                            if let Ok(mut vel) = world.get::<&mut Velocity>(e) {
                                vel.0 = Vec3d::new(vx, vy, vz);
                                return true;
                            }
                            break;
                        }
                    }
                    false
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    // pickaxe.entities.list() -> table of entity tables
    entities_table
        .set(
            "list",
            lua.create_function(|lua, ()| {
                with_world(lua, |world| -> Vec<mlua::Value> {
                    let mut result = Vec::new();
                    for (_e, (eid, pos, item_ent)) in world
                        .query::<(&EntityId, &Position, &ItemEntity)>()
                        .iter()
                    {
                        if let Ok(table) = lua.create_table() {
                            let _ = table.set("id", eid.0);
                            let _ = table.set("type", "item");
                            let _ = table.set("x", pos.0.x);
                            let _ = table.set("y", pos.0.y);
                            let _ = table.set("z", pos.0.z);
                            let _ = table.set("item_id", item_ent.item.item_id);
                            let _ = table.set("item_count", item_ent.item.count);
                            let item_name =
                                pickaxe_data::item_id_to_name(item_ent.item.item_id)
                                    .unwrap_or("unknown");
                            let _ = table.set("item_name", item_name);
                            result.push(mlua::Value::Table(table));
                        }
                    }
                    result
                })
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    pickaxe.set("entities", entities_table).map_err(lua_err)?;
    Ok(())
}
