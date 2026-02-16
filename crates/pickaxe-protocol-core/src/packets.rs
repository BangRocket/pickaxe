use pickaxe_nbt::NbtValue;
use pickaxe_types::{BlockPos, GameMode, GameProfile, ItemStack, TextComponent, Vec3d};
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
    /// System chat message (0x6C clientbound, protocol 767)
    SystemChatMessage {
        /// NBT text component content
        content: TextComponent,
        /// If true, displayed as action bar overlay
        overlay: bool,
    },

    /// Player Info Update (0x3E clientbound, protocol 767)
    /// Bitmask-driven: only sends fields indicated by actions.
    PlayerInfoUpdate {
        actions: u8,
        players: Vec<PlayerInfoEntry>,
    },

    /// Player Info Remove (0x3D clientbound, protocol 767)
    PlayerInfoRemove {
        uuids: Vec<Uuid>,
    },

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

    BlockUpdate {
        position: BlockPos,
        block_id: i32,
    },
    AcknowledgeBlockChange {
        sequence: i32,
    },
    /// Set Block Destroy Stage (0x06 clientbound, protocol 767)
    SetBlockDestroyStage {
        entity_id: i32,
        position: BlockPos,
        /// 0-9 for destroy stages, -1 to remove animation
        destroy_stage: i8,
    },
    /// Chunk Batch Start (0x0D clientbound, protocol 767) — empty packet.
    ChunkBatchStart,
    /// Chunk Batch Finished (0x0C clientbound, protocol 767).
    ChunkBatchFinished {
        batch_size: i32,
    },

    /// Update Time (0x64 clientbound, protocol 767)
    UpdateTime {
        world_age: i64,
        time_of_day: i64,
    },

    /// Spawn Entity (0x01 clientbound, protocol 767)
    SpawnEntity {
        entity_id: i32,
        entity_uuid: Uuid,
        entity_type: i32,    // entity type ID (player=128 in 1.21.1)
        x: f64,
        y: f64,
        z: f64,
        pitch: u8,           // angle as 256ths of a turn
        yaw: u8,
        head_yaw: u8,
        data: i32,            // extra data (0 for players)
        velocity_x: i16,
        velocity_y: i16,
        velocity_z: i16,
    },

    /// Remove Entities (0x42 clientbound, protocol 767)
    RemoveEntities {
        entity_ids: Vec<i32>,
    },

    /// Update Entity Position (0x2E clientbound, protocol 767)
    /// Relative move in 1/4096ths of a block. Max ~8 blocks per packet.
    UpdateEntityPosition {
        entity_id: i32,
        delta_x: i16,
        delta_y: i16,
        delta_z: i16,
        on_ground: bool,
    },

    /// Update Entity Position and Rotation (0x2F clientbound, protocol 767)
    UpdateEntityPositionAndRotation {
        entity_id: i32,
        delta_x: i16,
        delta_y: i16,
        delta_z: i16,
        yaw: u8,
        pitch: u8,
        on_ground: bool,
    },

    /// Update Entity Rotation (0x30 clientbound, protocol 767)
    UpdateEntityRotation {
        entity_id: i32,
        yaw: u8,
        pitch: u8,
        on_ground: bool,
    },

    /// Set Head Rotation (0x48 clientbound, protocol 767)
    SetHeadRotation {
        entity_id: i32,
        head_yaw: u8,
    },

    /// Teleport Entity (0x70 clientbound, protocol 767)
    TeleportEntity {
        entity_id: i32,
        x: f64,
        y: f64,
        z: f64,
        yaw: u8,
        pitch: u8,
        on_ground: bool,
    },

    /// Declare Commands (0x11 CB) — command tree for tab completion.
    DeclareCommands {
        nodes: Vec<CommandNode>,
        root_index: i32,
    },

    /// Set Container Content (0x13 CB) — sends entire inventory.
    SetContainerContent {
        window_id: u8,
        state_id: i32,
        slots: Vec<Option<ItemStack>>,
        carried_item: Option<ItemStack>,
    },

    /// Set Container Slot (0x15 CB) — update a single slot.
    SetContainerSlot {
        window_id: i8,
        state_id: i32,
        slot: i16,
        item: Option<ItemStack>,
    },

    /// Set Held Item (0x53 CB) — tell client which hotbar slot.
    SetHeldItem {
        slot: i8,
    },

    /// Set Entity Metadata (0x58 CB) — entity metadata entries.
    SetEntityMetadata {
        entity_id: i32,
        metadata: Vec<EntityMetadataEntry>,
    },

    /// Set Entity Velocity (0x5A CB) — entity velocity in 1/8000 blocks/tick.
    SetEntityVelocity {
        entity_id: i32,
        velocity_x: i16,
        velocity_y: i16,
        velocity_z: i16,
    },

    // === Play (serverbound) ===
    /// Chat Message (0x06 serverbound, protocol 767)
    ChatMessage {
        message: String,
        timestamp: i64,
        salt: i64,
        has_signature: bool,
        signature: Option<Vec<u8>>,
        offset: i32,
        acknowledged: [u8; 3],
    },

    /// Chat Command (0x04 serverbound, protocol 767)
    ChatCommand {
        command: String,
    },

    /// Set Held Item (0x2F SB) — player changed hotbar selection.
    HeldItemChange {
        slot: i16,
    },

    /// Creative Inventory Action (0x32 SB) — creative mode item set.
    CreativeInventoryAction {
        slot: i16,
        item: Option<ItemStack>,
    },

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
    BlockDig {
        status: i32,
        position: BlockPos,
        face: u8,
        sequence: i32,
    },
    BlockPlace {
        hand: i32,
        position: BlockPos,
        face: u8,
        cursor_x: f32,
        cursor_y: f32,
        cursor_z: f32,
        inside_block: bool,
        sequence: i32,
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

/// Player Info Update action bitmask flags.
pub mod player_info_actions {
    pub const ADD_PLAYER: u8 = 0x01;
    pub const INITIALIZE_CHAT: u8 = 0x02;
    pub const UPDATE_GAME_MODE: u8 = 0x04;
    pub const UPDATE_LISTED: u8 = 0x08;
    pub const UPDATE_LATENCY: u8 = 0x10;
    pub const UPDATE_DISPLAY_NAME: u8 = 0x20;
}

/// A single player entry in a PlayerInfoUpdate packet.
#[derive(Debug, Clone)]
pub struct PlayerInfoEntry {
    pub uuid: Uuid,
    /// Present when ADD_PLAYER action is set.
    pub name: Option<String>,
    /// Properties (name, value, signature) — present with ADD_PLAYER.
    pub properties: Vec<(String, String, Option<String>)>,
    /// Present when UPDATE_GAME_MODE action is set.
    pub game_mode: Option<i32>,
    /// Present when UPDATE_LISTED action is set.
    pub listed: Option<bool>,
    /// Present when UPDATE_LATENCY action is set.
    pub ping: Option<i32>,
    /// Present when UPDATE_DISPLAY_NAME action is set.
    pub display_name: Option<TextComponent>,
}

/// A single entity metadata entry for SetEntityMetadata.
#[derive(Debug, Clone)]
pub struct EntityMetadataEntry {
    pub index: u8,
    pub type_id: i32,
    pub data: Vec<u8>,
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

/// A node in the Declare Commands tree (0x11).
/// See <https://wiki.vg/Command_Data> for the wire format.
#[derive(Debug, Clone)]
pub struct CommandNode {
    /// Flags byte: bits 0-1 = node type (0=root, 1=literal, 2=argument),
    /// bit 2 = is_executable, bit 3 = has_redirect, bit 4 = has_suggestions.
    pub flags: u8,
    pub children: Vec<i32>,
    /// Name of the node (for literal and argument nodes).
    pub name: Option<String>,
    /// Parser identifier string (for argument nodes, e.g. "brigadier:integer").
    pub parser: Option<String>,
    /// Extra parser properties (raw bytes, parser-specific).
    pub parser_properties: Option<Vec<u8>>,
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
