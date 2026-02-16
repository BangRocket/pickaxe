/// Raw pointers to game state, valid only during fire_event scope.
/// Uses `*mut ()` to avoid adding hecs/game dependencies to pickaxe-scripting.
///
/// Safety: pointers are set before a synchronous Lua call and cleared after.
/// Only accessed from the main thread during synchronous Lua event dispatch.
pub struct LuaGameContext {
    pub world_ptr: *mut (),
    pub world_state_ptr: *mut (),
}

// Safety: only accessed from the main thread during synchronous Lua calls
unsafe impl Send for LuaGameContext {}
unsafe impl Sync for LuaGameContext {}
