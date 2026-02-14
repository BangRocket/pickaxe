use pickaxe_protocol_core::InternalPacket;
use pickaxe_types::GameProfile;
use pickaxe_world::generate_flat_chunk;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::RwLock;

/// Shared server state accessible from all connection tasks.
pub struct ServerState {
    next_eid: AtomicI32,
    players: RwLock<HashMap<i32, PlayerEntry>>,
    flat_chunk_template: InternalPacket,
}

struct PlayerEntry {
    pub profile: GameProfile,
}

impl ServerState {
    pub fn new() -> Self {
        let chunk = generate_flat_chunk();
        let flat_chunk_template = chunk.to_packet(0, 0);

        Self {
            next_eid: AtomicI32::new(1),
            players: RwLock::new(HashMap::new()),
            flat_chunk_template,
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

    pub fn get_chunk_packet(&self, chunk_x: i32, chunk_z: i32) -> InternalPacket {
        match &self.flat_chunk_template {
            InternalPacket::ChunkDataAndUpdateLight {
                heightmaps,
                data,
                block_entities,
                light_data,
                ..
            } => InternalPacket::ChunkDataAndUpdateLight {
                chunk_x,
                chunk_z,
                heightmaps: heightmaps.clone(),
                data: data.clone(),
                block_entities: block_entities.clone(),
                light_data: light_data.clone(),
            },
            _ => unreachable!(),
        }
    }
}
