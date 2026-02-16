use mlua::Lua;
use pickaxe_scripting::bridge::LuaGameContext;
use pickaxe_types::BlockPos;

/// Helper to run a closure with access to `&mut WorldState` from app_data.
fn with_world_state<F, R>(lua: &Lua, f: F) -> mlua::Result<R>
where
    F: FnOnce(&mut crate::tick::WorldState) -> R,
{
    let ctx = lua
        .app_data_ref::<LuaGameContext>()
        .ok_or_else(|| mlua::Error::runtime("Game API not available outside event handlers"))?;
    let ws = unsafe { &mut *(ctx.world_state_ptr as *mut crate::tick::WorldState) };
    Ok(f(ws))
}

/// Register `pickaxe.world` API on the Lua VM.
pub fn register_world_api(lua: &Lua) -> anyhow::Result<()> {
    let pickaxe: mlua::Table = lua
        .globals()
        .get("pickaxe")
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let world_table = lua
        .create_table()
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // pickaxe.world.get_block(x, y, z) -> block_state_id
    world_table
        .set(
            "get_block",
            lua.create_function(|lua, (x, y, z): (i32, i32, i32)| {
                with_world_state(lua, |ws| ws.get_block(&BlockPos::new(x, y, z)))
            })
            .map_err(|e| anyhow::anyhow!("{}", e))?,
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // pickaxe.world.set_block(x, y, z, state_id) -> old_state_id
    world_table
        .set(
            "set_block",
            lua.create_function(|lua, (x, y, z, state_id): (i32, i32, i32, i32)| {
                with_world_state(lua, |ws| ws.set_block(&BlockPos::new(x, y, z), state_id))
            })
            .map_err(|e| anyhow::anyhow!("{}", e))?,
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // pickaxe.world.get_time() -> time_of_day
    world_table
        .set(
            "get_time",
            lua.create_function(|lua, ()| with_world_state(lua, |ws| ws.time_of_day))
                .map_err(|e| anyhow::anyhow!("{}", e))?,
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // pickaxe.world.set_time(time_of_day)
    world_table
        .set(
            "set_time",
            lua.create_function(|lua, time: i64| {
                with_world_state(lua, |ws| {
                    ws.time_of_day = time.rem_euclid(24000);
                })
            })
            .map_err(|e| anyhow::anyhow!("{}", e))?,
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    pickaxe
        .set("world", world_table)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(())
}
