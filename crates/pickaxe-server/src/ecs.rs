use pickaxe_protocol_core::InternalPacket;
use pickaxe_types::{GameMode, GameProfile, Vec3d};
use std::collections::HashSet;
use tokio::sync::mpsc;

/// Network entity ID assigned by the server.
pub struct EntityId(pub i32);

/// Player's game profile (UUID + name + properties).
pub struct Profile(pub GameProfile);

/// Current position in the world.
pub struct Position(pub Vec3d);

/// Current look direction.
pub struct Rotation {
    pub yaw: f32,
    pub pitch: f32,
}

/// Whether the entity is on the ground.
pub struct OnGround(pub bool);

/// Player's current game mode.
pub struct PlayerGameMode(pub GameMode);

/// Channel to send packets to this player's network writer task.
pub struct ConnectionSender(pub mpsc::UnboundedSender<InternalPacket>);

/// Current chunk coordinates (for chunk streaming).
pub struct ChunkPosition {
    pub chunk_x: i32,
    pub chunk_z: i32,
}

/// Player's view distance.
pub struct ViewDistance(pub i32);

/// Keep-alive tracking for a player connection.
pub struct KeepAlive {
    pub last_response: std::time::Instant,
    pub last_sent: std::time::Instant,
    pub pending: Option<i64>,
}

impl KeepAlive {
    pub fn new() -> Self {
        let now = std::time::Instant::now();
        Self {
            last_response: now,
            last_sent: now,
            pending: None,
        }
    }
}

/// Tracks which entity IDs this player can currently see.
pub struct TrackedEntities {
    pub visible: HashSet<i32>,
}

impl TrackedEntities {
    pub fn new() -> Self {
        Self {
            visible: HashSet::new(),
        }
    }
}

/// Previous position — used to compute deltas for relative move packets.
pub struct PreviousPosition(pub Vec3d);

/// Previous rotation — used to detect rotation changes.
pub struct PreviousRotation {
    pub yaw: f32,
    pub pitch: f32,
}
