use pickaxe_nbt::NbtValue;
use pickaxe_types::{GameMode, GameProfile, TextComponent, Vec3d};
use uuid::Uuid;

/// Version-independent internal packet representation.
/// Protocol adapters convert between wire format and these.
#[derive(Debug, Clone)]
pub enum InternalPacket {
    // === Handshaking (serverbound) ===
    Handshake {
        protocol_version: i32,
        server_address: String,
        server_port: u16,
        next_state: i32,
    },

    // === Status ===
    StatusRequest,
    StatusResponse {
        json: String,
    },
    PingRequest {
        payload: i64,
    },
    PongResponse {
        payload: i64,
    },

    // === Login (serverbound) ===
    LoginStart {
        name: String,
        uuid: Uuid,
    },
    EncryptionResponse {
        shared_secret: Vec<u8>,
        verify_token: Vec<u8>,
    },
    LoginAcknowledged,

    // === Login (clientbound) ===
    EncryptionRequest {
        server_id: String,
        public_key: Vec<u8>,
        verify_token: Vec<u8>,
    },
    SetCompression {
        threshold: i32,
    },
    LoginSuccess {
        profile: GameProfile,
    },

    // === Configuration (serverbound) ===
    ClientInformation {
        locale: String,
        view_distance: i8,
        chat_mode: i32,
        chat_colors: bool,
        skin_parts: u8,
        main_hand: i32,
        text_filtering: bool,
        allow_listing: bool,
    },
    PluginMessage {
        channel: String,
        data: Vec<u8>,
    },
    FinishConfigurationAck,
    KnownPacksResponse {
        packs: Vec<KnownPack>,
    },

    // === Configuration (clientbound) ===
    RegistryData {
        registry_id: String,
        entries: Vec<RegistryEntry>,
    },
    FinishConfiguration,
    KnownPacksRequest {
        packs: Vec<KnownPack>,
    },

    // === Play (clientbound) ===
    JoinGame {
        entity_id: i32,
        is_hardcore: bool,
        dimension_names: Vec<String>,
        max_players: i32,
        view_distance: i32,
        simulation_distance: i32,
        reduced_debug_info: bool,
        enable_respawn_screen: bool,
        do_limited_crafting: bool,
        dimension_type: i32,
        dimension_name: String,
        hashed_seed: i64,
        game_mode: GameMode,
        previous_game_mode: i8,
        is_debug: bool,
        is_flat: bool,
        portal_cooldown: i32,
        sea_level: i32,
        enforces_secure_chat: bool,
    },
    SynchronizePlayerPosition {
        position: Vec3d,
        yaw: f32,
        pitch: f32,
        flags: u8,
        teleport_id: i32,
    },
    SetCenterChunk {
        chunk_x: i32,
        chunk_z: i32,
    },
    ChunkDataAndUpdateLight {
        chunk_x: i32,
        chunk_z: i32,
        heightmaps: NbtValue,
        data: Vec<u8>,
        block_entities: Vec<u8>,
        light_data: ChunkLightData,
    },
    UnloadChunk {
        chunk_x: i32,
        chunk_z: i32,
    },
    KeepAliveClientbound {
        id: i64,
    },
    GameEvent {
        event: u8,
        value: f32,
    },
    SetDefaultSpawnPosition {
        position: pickaxe_types::BlockPos,
        angle: f32,
    },

    // === Play (serverbound) ===
    ConfirmTeleportation {
        teleport_id: i32,
    },
    PlayerPosition {
        x: f64,
        y: f64,
        z: f64,
        on_ground: bool,
    },
    PlayerPositionAndRotation {
        x: f64,
        y: f64,
        z: f64,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    },
    PlayerRotation {
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    },
    PlayerOnGround {
        on_ground: bool,
    },
    KeepAliveServerbound {
        id: i64,
    },

    // === Shared ===
    Disconnect {
        reason: TextComponent,
    },

    /// Unknown / unhandled packet — raw bytes preserved.
    Unknown {
        packet_id: i32,
        data: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub struct KnownPack {
    pub namespace: String,
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct RegistryEntry {
    pub id: String,
    pub data: Option<NbtValue>,
}

#[derive(Debug, Clone)]
pub struct ChunkLightData {
    pub sky_light_mask: Vec<i64>,
    pub block_light_mask: Vec<i64>,
    pub empty_sky_light_mask: Vec<i64>,
    pub empty_block_light_mask: Vec<i64>,
    pub sky_light_arrays: Vec<Vec<u8>>,
    pub block_light_arrays: Vec<Vec<u8>>,
}

impl Default for ChunkLightData {
    fn default() -> Self {
        Self {
            sky_light_mask: Vec::new(),
            block_light_mask: Vec::new(),
            empty_sky_light_mask: Vec::new(),
            empty_block_light_mask: Vec::new(),
            sky_light_arrays: Vec::new(),
            block_light_arrays: Vec::new(),
        }
    }
}
