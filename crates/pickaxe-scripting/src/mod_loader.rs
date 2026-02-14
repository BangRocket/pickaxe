use serde::Deserialize;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Parsed pickaxe.toml manifest.
#[derive(Debug, Clone)]
pub struct ModManifest {
    pub mod_info: ModInfo,
    pub entrypoint: PathBuf,
    pub base_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModInfo {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    #[serde(rename = "mod")]
    mod_section: ModSection,
}

#[derive(Debug, Deserialize)]
struct ModSection {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    entrypoint: Option<EntrypointSection>,
    #[serde(default)]
    load_order: Option<LoadOrderSection>,
}

#[derive(Debug, Deserialize)]
struct EntrypointSection {
    main: String,
}

#[derive(Debug, Deserialize)]
struct LoadOrderSection {
    #[serde(default)]
    after: Vec<String>,
}

/// Discover mods in a directory (each subdirectory with a pickaxe.toml).
pub fn discover_mods(dir: &Path) -> anyhow::Result<Vec<ModManifest>> {
    let mut mods = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            let manifest_path = path.join("pickaxe.toml");
            if manifest_path.exists() {
                match parse_manifest(&manifest_path, &path) {
                    Ok(manifest) => {
                        debug!("Found mod: {} at {:?}", manifest.mod_info.id, path);
                        mods.push(manifest);
                    }
                    Err(e) => {
                        tracing::error!("Failed to parse {:?}: {}", manifest_path, e);
                    }
                }
            }
        }
    }

    Ok(mods)
}

fn parse_manifest(manifest_path: &Path, base_dir: &Path) -> anyhow::Result<ModManifest> {
    let contents = std::fs::read_to_string(manifest_path)?;
    let file: ManifestFile = toml::from_str(&contents)?;

    let entrypoint_file = file
        .mod_section
        .entrypoint
        .as_ref()
        .map(|e| e.main.as_str())
        .unwrap_or("init.lua");

    Ok(ModManifest {
        mod_info: ModInfo {
            id: file.mod_section.id,
            name: file.mod_section.name,
            version: file.mod_section.version,
        },
        entrypoint: base_dir.join(entrypoint_file),
        base_dir: base_dir.to_path_buf(),
    })
}

/// Sort mods topologically based on load_order.after declarations.
/// For now, a simple sort: "core" first, "vanilla" second, everything else after.
pub fn sort_mods(mut mods: Vec<ModManifest>) -> Vec<ModManifest> {
    mods.sort_by(|a, b| {
        let order_a = mod_sort_key(&a.mod_info.id);
        let order_b = mod_sort_key(&b.mod_info.id);
        order_a.cmp(&order_b)
    });
    mods
}

fn mod_sort_key(id: &str) -> u32 {
    match id {
        "pickaxe-core" => 0,
        "pickaxe-vanilla" | "vanilla" => 1,
        _ => 2,
    }
}
