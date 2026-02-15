use pickaxe_protocol_core::InternalPacket;
use pickaxe_types::{BlockPos, ChunkPos, GameProfile};
use pickaxe_world::{generate_flat_chunk, Chunk};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::RwLock;

/// Shared server state accessible from all connection tasks.
pub struct ServerState {
    next_eid: AtomicI32,
    players: RwLock<HashMap<i32, PlayerEntry>>,
    /// Persistent world: chunk data keyed by chunk position.
    chunks: RwLock<HashMap<ChunkPos, Chunk>>,
}

struct PlayerEntry {
    pub profile: GameProfile,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            next_eid: AtomicI32::new(1),
            players: RwLock::new(HashMap::new()),
            chunks: RwLock::new(HashMap::new()),
        }
    }

    pub fn next_entity_id(&self) -> i32 {
        self.next_eid.fetch_add(1, Ordering::Relaxed)
    }

    pub fn add_player(&self, entity_id: i32, profile: &GameProfile) {
        let mut players = self.players.write().unwrap();
        players.insert(entity_id, PlayerEntry {
            profile: profile.clone(),
        });
    }

    pub fn remove_player(&self, entity_id: i32) {
        let mut players = self.players.write().unwrap();
        players.remove(&entity_id);
    }

    pub fn player_count(&self) -> usize {
        self.players.read().unwrap().len()
    }

    /// Get a chunk packet, generating the flat chunk on first access.
    pub fn get_chunk_packet(&self, chunk_x: i32, chunk_z: i32) -> InternalPacket {
        let pos = ChunkPos::new(chunk_x, chunk_z);
        let mut chunks = self.chunks.write().unwrap();
        let chunk = chunks.entry(pos).or_insert_with(generate_flat_chunk);
        chunk.to_packet(chunk_x, chunk_z)
    }

    /// Set a block in the world. Returns the old block state ID.
    pub fn set_block(&self, pos: &BlockPos, state_id: i32) -> i32 {
        let chunk_pos = pos.chunk_pos();
        let local_x = (pos.x.rem_euclid(16)) as usize;
        let local_z = (pos.z.rem_euclid(16)) as usize;
        let mut chunks = self.chunks.write().unwrap();
        let chunk = chunks.entry(chunk_pos).or_insert_with(generate_flat_chunk);
        chunk.set_block(local_x, pos.y, local_z, state_id)
    }

    /// Get a block from the world.
    pub fn get_block(&self, pos: &BlockPos) -> i32 {
        let chunk_pos = pos.chunk_pos();
        let local_x = (pos.x.rem_euclid(16)) as usize;
        let local_z = (pos.z.rem_euclid(16)) as usize;
        let chunks = self.chunks.read().unwrap();
        if let Some(chunk) = chunks.get(&chunk_pos) {
            chunk.get_block(local_x, pos.y, local_z)
        } else {
            // Chunk not loaded yet — generate to check
            drop(chunks);
            let mut chunks = self.chunks.write().unwrap();
            let chunk = chunks.entry(chunk_pos).or_insert_with(generate_flat_chunk);
            chunk.get_block(local_x, pos.y, local_z)
        }
    }
}
