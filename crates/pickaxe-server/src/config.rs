use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_max_players")]
    pub max_players: u32,
    #[serde(default = "default_motd")]
    pub motd: String,
    #[serde(default)]
    pub online_mode: bool,
    #[serde(default = "default_view_distance")]
    pub view_distance: u32,
}

fn default_bind() -> String {
    "0.0.0.0".into()
}

fn default_port() -> u16 {
    25565
}

fn default_max_players() -> u32 {
    20
}

fn default_motd() -> String {
    "A Pickaxe Server".into()
}

fn default_view_distance() -> u32 {
    8
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
            max_players: default_max_players(),
            motd: default_motd(),
            online_mode: false,
            view_distance: default_view_distance(),
        }
    }
}

impl ServerConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            let contents = std::fs::read_to_string(path)?;
            let config: ServerConfig = toml::from_str(&contents)?;
            Ok(config)
        } else {
            tracing::info!("No config file found at {}, using defaults", path.display());
            Ok(Self::default())
        }
    }
}

/// Separate ops file — hot-reloaded on every permission check.
#[derive(Debug, Deserialize, Default)]
struct OpsConfig {
    #[serde(default)]
    ops: Vec<String>,
}

/// Read the operator list from config/ops.toml. Returns an empty list on error.
pub fn load_ops() -> Vec<String> {
    let path = Path::new("config/ops.toml");
    if path.exists() {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str::<OpsConfig>(&s).ok())
            .map(|c| c.ops)
            .unwrap_or_default()
    } else {
        Vec::new()
    }
}
