use crate::mod_loader::ModManifest;
use mlua::Lua;
use tracing::debug;

/// Load a mod by executing its entrypoint Lua file.
pub fn load_mod(lua: &Lua, manifest: &ModManifest) -> anyhow::Result<()> {
    let entrypoint = &manifest.entrypoint;

    if !entrypoint.exists() {
        return Err(anyhow::anyhow!(
            "Entrypoint not found: {:?}",
            entrypoint
        ));
    }

    let source = std::fs::read_to_string(entrypoint)?;
    let chunk_name = format!(
        "@{}/{}",
        manifest.mod_info.id,
        entrypoint.file_name().unwrap().to_string_lossy()
    );

    debug!(
        "Executing {} entrypoint: {:?}",
        manifest.mod_info.id, entrypoint
    );

    lua.load(&source)
        .set_name(&chunk_name)
        .exec()
        .map_err(|e| anyhow::anyhow!("Lua error: {}", e))?;

    Ok(())
}
