use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A block position in the world (x, y, z integers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// Encode as a 64-bit long (protocol format).
    /// x: 26 bits, z: 26 bits, y: 12 bits
    pub fn encode(&self) -> u64 {
        ((self.x as u64 & 0x3FFFFFF) << 38)
            | ((self.z as u64 & 0x3FFFFFF) << 12)
            | (self.y as u64 & 0xFFF)
    }

    pub fn decode(val: u64) -> Self {
        let mut x = (val >> 38) as i32;
        let mut z = ((val >> 12) & 0x3FFFFFF) as i32;
        let mut y = (val & 0xFFF) as i32;
        if x >= 1 << 25 {
            x -= 1 << 26;
        }
        if z >= 1 << 25 {
            z -= 1 << 26;
        }
        if y >= 1 << 11 {
            y -= 1 << 12;
        }
        Self { x, y, z }
    }

    pub fn chunk_pos(&self) -> ChunkPos {
        ChunkPos {
            x: self.x >> 4,
            z: self.z >> 4,
        }
    }
}

/// A chunk position (x, z).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}

impl ChunkPos {
    pub fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }
}

/// A 3D position with double precision.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec3d {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3d {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn chunk_pos(&self) -> ChunkPos {
        ChunkPos {
            x: (self.x.floor() as i32) >> 4,
            z: (self.z.floor() as i32) >> 4,
        }
    }
}

/// A Minecraft resource identifier (e.g., "minecraft:stone").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Identifier {
    pub namespace: String,
    pub path: String,
}

impl Identifier {
    pub fn new(namespace: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            path: path.into(),
        }
    }

    pub fn minecraft(path: impl Into<String>) -> Self {
        Self::new("minecraft", path)
    }
}

impl std::fmt::Display for Identifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.namespace, self.path)
    }
}

impl std::str::FromStr for Identifier {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((ns, path)) = s.split_once(':') {
            Ok(Self::new(ns, path))
        } else {
            Ok(Self::minecraft(s))
        }
    }
}

/// A player's game profile (UUID + name + properties).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameProfile {
    pub uuid: Uuid,
    pub name: String,
    pub properties: Vec<ProfileProperty>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileProperty {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

/// Text component for chat messages (simplified JSON text).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextComponent {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub extra: Vec<TextComponent>,
}

impl TextComponent {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            color: None,
            bold: None,
            italic: None,
            extra: Vec::new(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| r#"{"text":""}"#.to_string())
    }
}

/// Game mode enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum GameMode {
    Survival = 0,
    Creative = 1,
    Adventure = 2,
    Spectator = 3,
}

impl GameMode {
    pub fn id(self) -> u8 {
        self as u8
    }
}

/// Hand enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Hand {
    Main = 0,
    Off = 1,
}

/// An item stack in an inventory slot.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemStack {
    /// Item registry ID (from PrismarineJS items.json).
    pub item_id: i32,
    /// Number of items in this stack (1-127).
    pub count: i8,
}

impl ItemStack {
    pub fn new(item_id: i32, count: i8) -> Self {
        Self { item_id, count }
    }
}
