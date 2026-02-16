use crate::ecs::*;
use hecs::World;
use mlua::Lua;
use pickaxe_protocol_core::InternalPacket;
use pickaxe_scripting::bridge::LuaGameContext;
use pickaxe_types::{BlockPos, GameMode, TextComponent, Vec3d};

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
fn give_item_to_player(world: &mut World, entity: hecs::Entity, item_id: i32, count: i8) -> bool {
    let slot_index = {
        let inv = match world.get::<&Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return false,
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
            None => return false, // inventory full
        }
    };

    let item = pickaxe_types::ItemStack::new(item_id, count);
    let state_id = {
        let mut inv = match world.get::<&mut Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return false,
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

    pickaxe.set("players", players_table).map_err(lua_err)?;
    Ok(())
}
