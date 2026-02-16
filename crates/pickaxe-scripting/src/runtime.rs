use crate::mod_loader;
use mlua::{Lua, RegistryKey};
use pickaxe_events::{EventBus, OverrideRegistry, Priority};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::{error, info};

/// Convert mlua::Error to anyhow::Error by stringifying it.
fn lua_err(e: mlua::Error) -> anyhow::Error {
    anyhow::anyhow!("{}", e)
}

/// The script runtime: owns the Lua VM, event bus, and callback registry.
pub struct ScriptRuntime {
    lua: Lua,
    pub event_bus: Arc<Mutex<EventBus>>,
    pub override_registry: Arc<Mutex<OverrideRegistry>>,
    callbacks: Arc<Mutex<HashMap<u64, RegistryKey>>>,
}

impl ScriptRuntime {
    pub fn new() -> anyhow::Result<Self> {
        let lua = Lua::new();
        let event_bus = Arc::new(Mutex::new(EventBus::new()));
        let override_registry = Arc::new(Mutex::new(OverrideRegistry::new()));
        let callbacks = Arc::new(Mutex::new(HashMap::new()));

        setup_globals(&lua, event_bus.clone(), callbacks.clone())?;

        Ok(Self {
            lua,
            event_bus,
            override_registry,
            callbacks,
        })
    }

    /// Discover and load mods from the given directories.
    pub fn load_mods(&self, mod_dirs: &[&Path]) -> anyhow::Result<()> {
        let mut manifests = Vec::new();
        for dir in mod_dirs {
            if dir.exists() {
                match mod_loader::discover_mods(dir) {
                    Ok(mut found) => manifests.append(&mut found),
                    Err(e) => error!("Failed to scan mod directory {:?}: {}", dir, e),
                }
            }
        }

        manifests = mod_loader::sort_mods(manifests);

        for manifest in &manifests {
            info!(
                "Loading mod: {} v{}",
                manifest.mod_info.name, manifest.mod_info.version
            );
            if let Err(e) = crate::sandbox::load_mod(&self.lua, manifest) {
                error!("Failed to load mod '{}': {}", manifest.mod_info.id, e);
            }
        }

        let bus = self.event_bus.lock().unwrap();
        info!(
            "Scripting initialized: {} events, {} listeners",
            bus.event_count(),
            bus.listener_count()
        );

        Ok(())
    }

    /// Access the underlying Lua VM.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Fire an event with string key-value data. Returns true if cancelled.
    pub fn fire_event(&self, event_name: &str, data: &[(&str, &str)]) -> bool {
        let bus = self.event_bus.lock().unwrap();
        let listeners: Vec<_> = bus.get_listeners(event_name).to_vec();
        drop(bus);

        if listeners.is_empty() {
            return false;
        }

        let table = match self.lua.create_table() {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to create event table: {}", e);
                return false;
            }
        };
        for (key, value) in data {
            let _ = table.set(*key, *value);
        }

        let callbacks = self.callbacks.lock().unwrap();
        let mut cancelled = false;

        for listener in &listeners {
            if let Some(reg_key) = callbacks.get(&listener.listener_id) {
                let result: Result<Option<String>, mlua::Error> = (|| {
                    let func: mlua::Function = self.lua.registry_value(reg_key)?;
                    func.call(table.clone())
                })();

                match result {
                    Ok(Some(ref s)) if s == "cancel" => {
                        if listener.priority != pickaxe_events::Priority::Monitor {
                            cancelled = true;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        error!(
                            "Error in '{}' handler from mod '{}': {}",
                            event_name, listener.mod_id, e
                        );
                    }
                }
            }
        }

        cancelled
    }
}

fn setup_globals(
    lua: &Lua,
    event_bus: Arc<Mutex<EventBus>>,
    callbacks: Arc<Mutex<HashMap<u64, RegistryKey>>>,
) -> anyhow::Result<()> {
    let pickaxe = lua.create_table().map_err(lua_err)?;

    // pickaxe.log(message)
    let log_fn = lua
        .create_function(|_, msg: String| {
            info!("[Lua] {}", msg);
            Ok(())
        })
        .map_err(lua_err)?;
    pickaxe.set("log", log_fn).map_err(lua_err)?;

    // pickaxe.events table
    let events_table = lua.create_table().map_err(lua_err)?;

    // pickaxe.events.on(event_name, callback, options?)
    let bus_clone = event_bus.clone();
    let cb_clone = callbacks.clone();
    let events_on = lua
        .create_function(
            move |lua_ctx,
                  (event_name, callback, options): (
                String,
                mlua::Function,
                Option<mlua::Table>,
            )| {
                let priority = if let Some(ref opts) = options {
                    let p: Option<String> = opts.get("priority").unwrap_or(None);
                    p.map(|s| Priority::from_str(&s))
                        .unwrap_or(Priority::Normal)
                } else {
                    Priority::Normal
                };

                let mod_id = if let Some(ref opts) = options {
                    let m: Option<String> = opts.get("mod_id").unwrap_or(None);
                    m.unwrap_or_else(|| "unknown".into())
                } else {
                    "unknown".into()
                };

                let listener_id = {
                    let mut bus = bus_clone.lock().unwrap();
                    bus.register(&event_name, &mod_id, priority)
                };

                let reg_key = lua_ctx.create_registry_value(callback)?;
                {
                    let mut cbs = cb_clone.lock().unwrap();
                    cbs.insert(listener_id, reg_key);
                }

                Ok(())
            },
        )
        .map_err(lua_err)?;
    events_table.set("on", events_on).map_err(lua_err)?;

    pickaxe.set("events", events_table).map_err(lua_err)?;
    lua.globals().set("pickaxe", pickaxe).map_err(lua_err)?;

    Ok(())
}
