use crate::config::ServerConfig;
use crate::ecs::*;
use bytes::BytesMut;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use hecs::World;
use pickaxe_nbt::{nbt_compound, nbt_list, NbtValue};
use pickaxe_protocol_core::{player_info_actions, CommandNode, InternalPacket, PlayerInfoEntry};
use pickaxe_protocol_v1_21::{build_item_metadata, V1_21Adapter};
use pickaxe_region::RegionStorage;
use pickaxe_scripting::ScriptRuntime;
use pickaxe_types::{BlockPos, GameMode, GameProfile, ItemStack, TextComponent, Vec3d};
use pickaxe_world::{generate_flat_chunk, Chunk};
use rand::Rng;
use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use pickaxe_types::ChunkPos;

/// Packets decoded by the reader task, sent to the tick loop.
#[derive(Debug)]
pub struct InboundPacket {
    pub entity_id: i32,
    pub packet: InternalPacket,
}

/// A new player ready to enter play state.
pub struct NewPlayer {
    pub entity_id: i32,
    pub profile: GameProfile,
    pub packet_tx: mpsc::UnboundedSender<InternalPacket>,
    pub packet_rx: mpsc::UnboundedReceiver<InboundPacket>,
}

/// Deserialized player save data loaded from disk.
struct PlayerSaveData {
    position: Vec3d,
    yaw: f32,
    pitch: f32,
    health: f32,
    food_level: i32,
    saturation: f32,
    exhaustion: f32,
    fall_distance: f32,
    held_slot: u8,
    game_mode: GameMode,
    slots: [Option<ItemStack>; 46],
    xp_level: i32,
    xp_progress: f32,
    xp_total: i32,
}

/// Serialize a block entity to vanilla-compatible NBT for chunk storage.
fn serialize_block_entity(pos: &BlockPos, be: &BlockEntity) -> NbtValue {
    match be {
        BlockEntity::Chest { inventory } => {
            let mut items = Vec::new();
            for (i, slot) in inventory.iter().enumerate() {
                if let Some(item) = slot {
                    let name = pickaxe_data::item_id_to_name(item.item_id).unwrap_or("air");
                    items.push(nbt_compound! {
                        "Slot" => NbtValue::Byte(i as i8),
                        "id" => NbtValue::String(format!("minecraft:{}", name)),
                        "Count" => NbtValue::Byte(item.count)
                    });
                }
            }
            nbt_compound! {
                "id" => NbtValue::String("minecraft:chest".into()),
                "x" => NbtValue::Int(pos.x),
                "y" => NbtValue::Int(pos.y),
                "z" => NbtValue::Int(pos.z),
                "Items" => NbtValue::List(items)
            }
        }
        BlockEntity::Furnace { input, fuel, output, burn_time, burn_duration: _, cook_progress, cook_total } => {
            let mut items = Vec::new();
            for (i, slot) in [input, fuel, output].iter().enumerate() {
                if let Some(item) = slot {
                    let name = pickaxe_data::item_id_to_name(item.item_id).unwrap_or("air");
                    items.push(nbt_compound! {
                        "Slot" => NbtValue::Byte(i as i8),
                        "id" => NbtValue::String(format!("minecraft:{}", name)),
                        "Count" => NbtValue::Byte(item.count)
                    });
                }
            }
            nbt_compound! {
                "id" => NbtValue::String("minecraft:furnace".into()),
                "x" => NbtValue::Int(pos.x),
                "y" => NbtValue::Int(pos.y),
                "z" => NbtValue::Int(pos.z),
                "Items" => NbtValue::List(items),
                "BurnTime" => NbtValue::Short(*burn_time),
                "CookTime" => NbtValue::Short(*cook_progress),
                "CookTimeTotal" => NbtValue::Short(*cook_total)
            }
        }
    }
}

/// Deserialize a block entity from vanilla NBT.
fn deserialize_block_entity(nbt: &NbtValue) -> Option<(BlockPos, BlockEntity)> {
    let id = nbt.get("id")?.as_str()?;
    let x = nbt.get("x")?.as_int()?;
    let y = nbt.get("y")?.as_int()?;
    let z = nbt.get("z")?.as_int()?;
    let pos = BlockPos::new(x, y, z);

    let short_id = id.strip_prefix("minecraft:").unwrap_or(id);

    match short_id {
        "chest" => {
            let mut inventory: [Option<ItemStack>; 27] = std::array::from_fn(|_| None);
            if let Some(items_list) = nbt.get("Items").and_then(|v| v.as_list()) {
                for item_nbt in items_list {
                    let slot = item_nbt.get("Slot").and_then(|v| v.as_byte())? as usize;
                    let item_id_str = item_nbt.get("id").and_then(|v| v.as_str())?;
                    let name = item_id_str.strip_prefix("minecraft:").unwrap_or(item_id_str);
                    let item_id = pickaxe_data::item_name_to_id(name)?;
                    let count = item_nbt.get("Count").and_then(|v| v.as_byte()).unwrap_or(1);
                    if slot < 27 {
                        inventory[slot] = Some(ItemStack { item_id, count });
                    }
                }
            }
            Some((pos, BlockEntity::Chest { inventory }))
        }
        "furnace" => {
            let mut input = None;
            let mut fuel = None;
            let mut output = None;
            if let Some(items_list) = nbt.get("Items").and_then(|v| v.as_list()) {
                for item_nbt in items_list {
                    let slot = item_nbt.get("Slot").and_then(|v| v.as_byte()).unwrap_or(-1);
                    let item_id_str = match item_nbt.get("id").and_then(|v| v.as_str()) {
                        Some(s) => s,
                        None => continue,
                    };
                    let name = item_id_str.strip_prefix("minecraft:").unwrap_or(item_id_str);
                    let item_id = match pickaxe_data::item_name_to_id(name) {
                        Some(id) => id,
                        None => continue,
                    };
                    let count = item_nbt.get("Count").and_then(|v| v.as_byte()).unwrap_or(1);
                    let stack = ItemStack { item_id, count };
                    match slot {
                        0 => input = Some(stack),
                        1 => fuel = Some(stack),
                        2 => output = Some(stack),
                        _ => {}
                    }
                }
            }
            let burn_time = nbt.get("BurnTime").and_then(|v| v.as_short()).unwrap_or(0);
            let cook_progress = nbt.get("CookTime").and_then(|v| v.as_short()).unwrap_or(0);
            let cook_total = nbt.get("CookTimeTotal").and_then(|v| v.as_short()).unwrap_or(200);
            Some((pos, BlockEntity::Furnace {
                input, fuel, output,
                burn_time, burn_duration: burn_time, cook_progress, cook_total,
            }))
        }
        _ => None,
    }
}

/// Serialize a player entity's ECS components to gzip-compressed vanilla-compatible NBT.
fn serialize_player_data(world: &World, entity: hecs::Entity) -> Option<Vec<u8>> {
    let pos = world.get::<&Position>(entity).ok()?;
    let rot = world.get::<&Rotation>(entity).ok()?;
    let on_ground = world.get::<&OnGround>(entity).ok()?;
    let health = world.get::<&Health>(entity).ok()?;
    let food = world.get::<&FoodData>(entity).ok()?;
    let fall_dist = world.get::<&FallDistance>(entity).ok()?;
    let inv = world.get::<&Inventory>(entity).ok()?;
    let held = world.get::<&HeldSlot>(entity).ok()?;
    let gm = world.get::<&PlayerGameMode>(entity).ok()?;
    let xp = world.get::<&ExperienceData>(entity).ok();

    // Build inventory NBT list with vanilla slot mapping
    let mut inv_items = Vec::new();
    for (ecs_slot, item) in inv.slots.iter().enumerate() {
        if let Some(stack) = item {
            let nbt_slot: i8 = match ecs_slot {
                36..=44 => (ecs_slot - 36) as i8,    // hotbar: ECS 36-44 → NBT 0-8
                9..=35 => ecs_slot as i8,              // main: ECS 9-35 → NBT 9-35
                5..=8 => (100 + (ecs_slot - 5)) as i8, // armor: ECS 5-8 → NBT 100-103
                45 => -106,                             // offhand: ECS 45 → NBT -106
                _ => continue,
            };
            let item_name = format!(
                "minecraft:{}",
                pickaxe_data::item_id_to_name(stack.item_id).unwrap_or("air")
            );
            inv_items.push(nbt_compound! {
                "Slot" => NbtValue::Byte(nbt_slot),
                "id" => NbtValue::String(item_name),
                "count" => NbtValue::Byte(stack.count)
            });
        }
    }

    let nbt = nbt_compound! {
        "DataVersion" => NbtValue::Int(3955),
        "Pos" => nbt_list![
            NbtValue::Double(pos.0.x),
            NbtValue::Double(pos.0.y),
            NbtValue::Double(pos.0.z)
        ],
        "Rotation" => nbt_list![
            NbtValue::Float(rot.yaw),
            NbtValue::Float(rot.pitch)
        ],
        "OnGround" => NbtValue::Byte(if on_ground.0 { 1 } else { 0 }),
        "Health" => NbtValue::Float(health.current),
        "FallDistance" => NbtValue::Float(fall_dist.0),
        "foodLevel" => NbtValue::Int(food.food_level),
        "foodSaturationLevel" => NbtValue::Float(food.saturation),
        "foodExhaustionLevel" => NbtValue::Float(food.exhaustion),
        "Inventory" => NbtValue::List(inv_items),
        "SelectedItemSlot" => NbtValue::Int(held.0 as i32),
        "playerGameType" => NbtValue::Int(gm.0.id() as i32),
        "Dimension" => NbtValue::String("minecraft:overworld".into()),
        "XpLevel" => NbtValue::Int(xp.as_ref().map(|x| x.level).unwrap_or(0)),
        "XpP" => NbtValue::Float(xp.as_ref().map(|x| x.progress).unwrap_or(0.0)),
        "XpTotal" => NbtValue::Int(xp.as_ref().map(|x| x.total_xp).unwrap_or(0))
    };

    let mut buf = BytesMut::new();
    nbt.write_root_named("", &mut buf);

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&buf).ok()?;
    Some(encoder.finish().ok()?)
}

/// Deserialize gzip-compressed vanilla NBT into a PlayerSaveData struct.
fn deserialize_player_data(data: &[u8]) -> Option<PlayerSaveData> {
    // Gzip decompress
    let mut decoder = GzDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed).ok()?;

    // Parse NBT
    let (_, nbt) = NbtValue::read_root_named(&decompressed).ok()?;

    // Extract position
    let pos_list = nbt.get("Pos")?.as_list()?;
    let x = pos_list.get(0)?.as_double()?;
    let y = pos_list.get(1)?.as_double()?;
    let z = pos_list.get(2)?.as_double()?;

    // Extract rotation
    let rot_list = nbt.get("Rotation")?.as_list()?;
    let yaw = rot_list.get(0)?.as_float()?;
    let pitch = rot_list.get(1)?.as_float()?;

    let health = nbt.get("Health")?.as_float()?;
    let fall_distance = nbt.get("FallDistance").and_then(|v| v.as_float()).unwrap_or(0.0);
    let food_level = nbt.get("foodLevel")?.as_int()?;
    let saturation = nbt.get("foodSaturationLevel")?.as_float()?;
    let exhaustion = nbt.get("foodExhaustionLevel").and_then(|v| v.as_float()).unwrap_or(0.0);
    let held_slot = nbt.get("SelectedItemSlot").and_then(|v| v.as_int()).unwrap_or(0) as u8;
    let game_type = nbt.get("playerGameType").and_then(|v| v.as_int()).unwrap_or(0);
    let game_mode = match game_type {
        1 => GameMode::Creative,
        2 => GameMode::Adventure,
        3 => GameMode::Spectator,
        _ => GameMode::Survival,
    };

    // Parse inventory
    let mut slots: [Option<ItemStack>; 46] = std::array::from_fn(|_| None);
    if let Some(inv_list) = nbt.get("Inventory").and_then(|v| v.as_list()) {
        for entry in inv_list {
            let nbt_slot = entry.get("Slot").and_then(|v| v.as_byte());
            let id_str = entry.get("id").and_then(|v| v.as_str());
            let count = entry.get("count").and_then(|v| v.as_byte()).unwrap_or(1);

            if let (Some(nbt_slot), Some(id_str)) = (nbt_slot, id_str) {
                // Strip "minecraft:" prefix
                let name = id_str.strip_prefix("minecraft:").unwrap_or(id_str);
                let item_id = match pickaxe_data::item_name_to_id(name) {
                    Some(id) => id,
                    None => continue,
                };

                // Map NBT slot → ECS slot
                let ecs_slot: usize = match nbt_slot {
                    0..=8 => (nbt_slot as usize) + 36,    // hotbar: NBT 0-8 → ECS 36-44
                    9..=35 => nbt_slot as usize,            // main: NBT 9-35 → ECS 9-35
                    100..=103 => (nbt_slot - 100) as usize + 5, // armor: NBT 100-103 → ECS 5-8
                    -106 => 45,                              // offhand: NBT -106 → ECS 45
                    _ => continue,
                };

                if ecs_slot < 46 {
                    slots[ecs_slot] = Some(ItemStack { item_id, count });
                }
            }
        }
    }

    let xp_level = nbt.get("XpLevel").and_then(|v| v.as_int()).unwrap_or(0);
    let xp_progress = nbt.get("XpP").and_then(|v| v.as_float()).unwrap_or(0.0);
    let xp_total = nbt.get("XpTotal").and_then(|v| v.as_int()).unwrap_or(0);

    Some(PlayerSaveData {
        position: Vec3d::new(x, y, z),
        yaw,
        pitch,
        health,
        food_level,
        saturation,
        exhaustion,
        fall_distance,
        held_slot,
        game_mode,
        slots,
        xp_level,
        xp_progress,
        xp_total,
    })
}

/// Save all currently-connected players' data.
fn save_all_players(world: &World, save_tx: &mpsc::UnboundedSender<SaveOp>) {
    for (entity, profile) in world.query::<&Profile>().iter() {
        if let Some(data) = serialize_player_data(world, entity) {
            let _ = save_tx.send(SaveOp::Player(profile.0.uuid, data));
        }
    }
}

/// Save all chunks that contain block entities.
fn save_block_entity_chunks(world_state: &WorldState) {
    use std::collections::HashSet;
    let mut saved_chunks = HashSet::new();
    for pos in world_state.block_entities.keys() {
        let chunk_pos = pos.chunk_pos();
        if saved_chunks.insert(chunk_pos) {
            world_state.queue_chunk_save(chunk_pos);
        }
    }
}

/// Serialize level.dat to gzip-compressed NBT (vanilla-compatible format).
fn serialize_level_dat(world_state: &WorldState, _config: &ServerConfig) -> Vec<u8> {
    let nbt = nbt_compound! {
        "DataVersion" => NbtValue::Int(3955),
        "Data" => nbt_compound! {
            "LevelName" => NbtValue::String("Pickaxe World".into()),
            "SpawnX" => NbtValue::Int(0),
            "SpawnY" => NbtValue::Int(-59),
            "SpawnZ" => NbtValue::Int(0),
            "Time" => NbtValue::Long(world_state.world_age),
            "DayTime" => NbtValue::Long(world_state.time_of_day),
            "GameType" => NbtValue::Int(0),
            "Difficulty" => NbtValue::Byte(2),
            "hardcore" => NbtValue::Byte(0),
            "allowCommands" => NbtValue::Byte(1),
            "Version" => nbt_compound! {
                "Name" => NbtValue::String("1.21.1".into()),
                "Id" => NbtValue::Int(767)
            }
        }
    };

    let mut buf = BytesMut::new();
    nbt.write_root_named("", &mut buf);

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    let _ = encoder.write_all(&buf);
    encoder.finish().unwrap_or_default()
}

/// Load world_age and time_of_day from a gzip-compressed level.dat file.
fn load_level_dat(path: &std::path::Path) -> Option<(i64, i64)> {
    let data = std::fs::read(path).ok()?;
    let mut decoder = GzDecoder::new(&data[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed).ok()?;

    let (_, nbt) = NbtValue::read_root_named(&decompressed).ok()?;
    let data_nbt = nbt.get("Data")?;
    let world_age = data_nbt.get("Time")?.as_long()?;
    let time_of_day = data_nbt.get("DayTime")?.as_long()?;
    Some((world_age, time_of_day))
}

/// Operations queued for the background saver task.
pub enum SaveOp {
    Chunk(i32, i32, Vec<u8>),
    Player(uuid::Uuid, Vec<u8>),
    LevelDat(Vec<u8>),
    Shutdown(tokio::sync::oneshot::Sender<()>),
}

/// Runs on a background Tokio blocking task. Processes SaveOps sequentially.
pub fn run_saver_task(
    mut rx: mpsc::UnboundedReceiver<SaveOp>,
    world_dir: PathBuf,
) {
    let region_dir = world_dir.join("region");
    let playerdata_dir = world_dir.join("playerdata");
    let _ = std::fs::create_dir_all(&region_dir);
    let _ = std::fs::create_dir_all(&playerdata_dir);

    let mut region_storage = match RegionStorage::new(region_dir) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to init region storage: {}", e);
            return;
        }
    };

    while let Some(op) = rx.blocking_recv() {
        match op {
            SaveOp::Chunk(cx, cz, data) => {
                if let Err(e) = region_storage.write_chunk(cx, cz, &data) {
                    tracing::error!("Failed to save chunk ({}, {}): {}", cx, cz, e);
                }
            }
            SaveOp::Player(uuid, data) => {
                let path = playerdata_dir.join(format!("{}.dat", uuid));
                let tmp_path = playerdata_dir.join(format!("{}.dat.tmp", uuid));
                if let Err(e) = std::fs::write(&tmp_path, &data) {
                    tracing::error!("Failed to write player data {}: {}", uuid, e);
                } else if let Err(e) = std::fs::rename(&tmp_path, &path) {
                    tracing::error!("Failed to rename player data {}: {}", uuid, e);
                }
            }
            SaveOp::LevelDat(data) => {
                let path = world_dir.join("level.dat");
                let tmp_path = world_dir.join("level.dat.tmp");
                if let Err(e) = std::fs::write(&tmp_path, &data) {
                    tracing::error!("Failed to write level.dat: {}", e);
                } else if let Err(e) = std::fs::rename(&tmp_path, &path) {
                    tracing::error!("Failed to rename level.dat: {}", e);
                }
            }
            SaveOp::Shutdown(done) => {
                tracing::info!("Saver task shutting down");
                let _ = done.send(());
                return;
            }
        }
    }
}

/// Block entity data for container blocks.
#[derive(Debug, Clone)]
pub enum BlockEntity {
    Chest {
        inventory: [Option<ItemStack>; 27],
    },
    Furnace {
        input: Option<ItemStack>,
        fuel: Option<ItemStack>,
        output: Option<ItemStack>,
        burn_time: i16,
        burn_duration: i16,
        cook_progress: i16,
        cook_total: i16,
    },
}

/// World state: chunk storage.
pub struct WorldState {
    chunks: HashMap<ChunkPos, Chunk>,
    pub world_age: i64,
    pub time_of_day: i64,
    pub tick_count: u64,
    region_storage: RegionStorage,
    pub save_tx: mpsc::UnboundedSender<SaveOp>,
    pub block_entities: HashMap<BlockPos, BlockEntity>,
}

impl WorldState {
    pub fn new(region_storage: RegionStorage, save_tx: mpsc::UnboundedSender<SaveOp>) -> Self {
        Self {
            chunks: HashMap::new(),
            world_age: 0,
            time_of_day: 0,
            tick_count: 0,
            region_storage,
            save_tx,
            block_entities: HashMap::new(),
        }
    }

    /// Ensures a chunk is loaded (from disk or generated) and returns a mutable reference.
    fn ensure_chunk(&mut self, pos: ChunkPos) -> &mut Chunk {
        if !self.chunks.contains_key(&pos) {
            // Try loading from disk
            if let Ok(Some(nbt_bytes)) = self.region_storage.read_chunk(pos.x, pos.z) {
                if let Ok((_, nbt)) = NbtValue::read_root_named(&nbt_bytes) {
                    if let Some(chunk) = Chunk::from_nbt(&nbt) {
                        // Load block entities from chunk NBT
                        if let Some(be_list) = nbt.get("block_entities").and_then(|v| v.as_list()) {
                            for be_nbt in be_list {
                                if let Some((be_pos, be)) = deserialize_block_entity(be_nbt) {
                                    self.block_entities.insert(be_pos, be);
                                }
                            }
                        }
                        self.chunks.insert(pos, chunk);
                        return self.chunks.get_mut(&pos).unwrap();
                    }
                }
            }
            // Generate (save deferred until modification via set_block)
            let chunk = generate_flat_chunk();
            self.chunks.insert(pos, chunk);
        }
        self.chunks.get_mut(&pos).unwrap()
    }

    /// Queue a chunk for background saving.
    fn queue_chunk_save(&self, pos: ChunkPos) {
        if let Some(chunk) = self.chunks.get(&pos) {
            let mut nbt = chunk.to_nbt(pos.x, pos.z, self.world_age);
            // Inject block entities for this chunk
            let chunk_min_x = pos.x * 16;
            let chunk_min_z = pos.z * 16;
            let mut be_list = Vec::new();
            for (be_pos, be) in &self.block_entities {
                if be_pos.x >= chunk_min_x && be_pos.x < chunk_min_x + 16
                    && be_pos.z >= chunk_min_z && be_pos.z < chunk_min_z + 16
                {
                    be_list.push(serialize_block_entity(be_pos, be));
                }
            }
            if let NbtValue::Compound(ref mut entries) = nbt {
                entries.push(("block_entities".into(), NbtValue::List(be_list)));
            }
            let mut buf = BytesMut::new();
            nbt.write_root_named("", &mut buf);
            let _ = self.save_tx.send(SaveOp::Chunk(pos.x, pos.z, buf.to_vec()));
        }
    }

    pub fn get_chunk_packet(&mut self, chunk_x: i32, chunk_z: i32) -> InternalPacket {
        let pos = ChunkPos::new(chunk_x, chunk_z);
        self.ensure_chunk(pos);
        self.chunks.get(&pos).unwrap().to_packet(chunk_x, chunk_z)
    }

    pub fn set_block(&mut self, pos: &BlockPos, state_id: i32) -> i32 {
        let chunk_pos = pos.chunk_pos();
        let local_x = (pos.x.rem_euclid(16)) as usize;
        let local_z = (pos.z.rem_euclid(16)) as usize;
        self.ensure_chunk(chunk_pos);
        let chunk = self.chunks.get_mut(&chunk_pos).unwrap();
        let old = chunk.set_block(local_x, pos.y, local_z, state_id);
        self.queue_chunk_save(chunk_pos);
        old
    }

    pub fn get_block(&mut self, pos: &BlockPos) -> i32 {
        let chunk_pos = pos.chunk_pos();
        let local_x = (pos.x.rem_euclid(16)) as usize;
        let local_z = (pos.z.rem_euclid(16)) as usize;
        self.ensure_chunk(chunk_pos);
        self.chunks.get(&chunk_pos).unwrap().get_block(local_x, pos.y, local_z)
    }

    pub fn get_block_entity(&self, pos: &BlockPos) -> Option<&BlockEntity> {
        self.block_entities.get(pos)
    }

    pub fn get_block_entity_mut(&mut self, pos: &BlockPos) -> Option<&mut BlockEntity> {
        self.block_entities.get_mut(pos)
    }

    pub fn set_block_entity(&mut self, pos: BlockPos, entity: BlockEntity) {
        self.block_entities.insert(pos, entity);
    }

    pub fn remove_block_entity(&mut self, pos: &BlockPos) -> Option<BlockEntity> {
        self.block_entities.remove(pos)
    }
}

/// The main game loop. Runs at 20 TPS on the main thread.
/// Owns the hecs World, the Lua ScriptRuntime, and all game state.
pub async fn run_tick_loop(
    config: Arc<ServerConfig>,
    scripting: ScriptRuntime,
    mut new_player_rx: mpsc::UnboundedReceiver<NewPlayer>,
    player_count: Arc<std::sync::atomic::AtomicUsize>,
    lua_commands: crate::bridge::LuaCommands,
    block_overrides: crate::bridge::BlockOverrides,
    next_eid: Arc<AtomicI32>,
    save_tx: mpsc::UnboundedSender<SaveOp>,
    region_storage: RegionStorage,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let adapter = V1_21Adapter::new();
    let mut world = World::new();
    let mut world_state = WorldState::new(region_storage, save_tx);

    // Load level.dat if it exists (restores world_age and time_of_day)
    let level_dat_path = PathBuf::from(&config.world_dir).join("level.dat");
    if let Some((world_age, time_of_day)) = load_level_dat(&level_dat_path) {
        world_state.world_age = world_age;
        world_state.time_of_day = time_of_day;
        info!("Loaded level.dat: world_age={}, time_of_day={}", world_age, time_of_day);
    }

    // Pre-generate spawn chunks so the first player join is instant
    let vd = config.view_distance as i32;
    let total = (2 * vd + 1) * (2 * vd + 1);
    info!("Preparing spawn area ({} chunks)...", total);
    for cx in -vd..=vd {
        for cz in -vd..=vd {
            world_state.ensure_chunk(ChunkPos::new(cx, cz));
        }
    }
    info!("Spawn area ready");

    // Collect inbound packet receivers from all active players
    // We store them separately since hecs components must be Send
    let mut inbound_receivers: HashMap<i32, mpsc::UnboundedReceiver<InboundPacket>> =
        HashMap::new();

    let tick_duration = Duration::from_millis(50); // 20 TPS
    let mut tick_count: u64 = 0;

    info!("Tick loop started (20 TPS)");

    loop {
        // Check for shutdown signal
        if *shutdown_rx.borrow() {
            info!("Shutting down...");
            // Save all players
            save_all_players(&world, &world_state.save_tx);
            // Save all chunks containing block entities
            save_block_entity_chunks(&world_state);
            // Save level.dat
            let level_data = serialize_level_dat(&world_state, &config);
            let _ = world_state.save_tx.send(SaveOp::LevelDat(level_data));
            // Signal saver to flush and stop
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            let _ = world_state.save_tx.send(SaveOp::Shutdown(done_tx));
            let _ = done_rx.await;
            info!("World saved. Goodbye!");
            return;
        }

        let tick_start = Instant::now();

        // 1. Accept new players
        while let Ok(new_player) = new_player_rx.try_recv() {
            handle_new_player(
                &config,
                &adapter,
                &mut world,
                &mut world_state,
                &mut inbound_receivers,
                new_player,
                &scripting,
                &lua_commands,
            );
        }

        // 2. Process inbound packets from all players
        let mut packets: Vec<InboundPacket> = Vec::new();
        let mut disconnected: Vec<i32> = Vec::new();

        for (&eid, rx) in inbound_receivers.iter_mut() {
            loop {
                match rx.try_recv() {
                    Ok(pkt) => packets.push(pkt),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        disconnected.push(eid);
                        break;
                    }
                }
            }
        }

        // 3. Handle disconnected players
        for eid in &disconnected {
            handle_disconnect(&mut world, &mut world_state, &mut inbound_receivers, *eid, &adapter, &scripting);
        }

        // Update player count
        let count = world.query::<&Profile>().iter().count();
        player_count.store(count, Ordering::Relaxed);

        // Update tick count in world state so it's available in process_packet
        world_state.tick_count = tick_count;

        // 4. Process packets
        for pkt in packets {
            process_packet(
                &config,
                &adapter,
                &mut world,
                &mut world_state,
                pkt,
                &scripting,
                &lua_commands,
                &block_overrides,
                &next_eid,
            );
        }

        // 5. Tick systems
        tick_keep_alive(&adapter, &mut world, tick_count);
        tick_attack_cooldown(&mut world);
        tick_void_damage(&mut world, &mut world_state, &scripting);
        tick_health_hunger(&mut world, &mut world_state, &scripting, tick_count);
        tick_eating(&mut world);
        tick_buttons(&mut world, &mut world_state);
        tick_item_physics(&mut world, &mut world_state, &scripting);
        if tick_count % 4 == 0 {
            tick_item_pickup(&mut world, &mut world_state, &scripting);
        }
        tick_furnaces(&world, &mut world_state);
        tick_entity_tracking(&mut world);
        tick_entity_movement_broadcast(&mut world);
        tick_world_time(&world, &mut world_state, tick_count);
        tick_block_breaking(&mut world, tick_count);

        // Periodic player/world data save (every 60 seconds = 1200 ticks)
        if tick_count % 1200 == 0 && tick_count > 0 {
            save_all_players(&world, &world_state.save_tx);
            save_block_entity_chunks(&world_state);
            let level_data = serialize_level_dat(&world_state, &config);
            let _ = world_state.save_tx.send(SaveOp::LevelDat(level_data));
        }

        tick_count += 1;

        // Sleep for remainder of tick
        let elapsed = tick_start.elapsed();
        if elapsed < tick_duration {
            tokio::time::sleep(tick_duration - elapsed).await;
        } else if tick_count % 100 == 0 {
            // Only warn occasionally to avoid log spam
            warn!(
                "Tick {} took {:?} (over 50ms budget)",
                tick_count, elapsed
            );
        }
    }
}

fn handle_new_player(
    config: &ServerConfig,
    _adapter: &V1_21Adapter,
    world: &mut World,
    world_state: &mut WorldState,
    inbound_receivers: &mut HashMap<i32, mpsc::UnboundedReceiver<InboundPacket>>,
    new_player: NewPlayer,
    scripting: &ScriptRuntime,
    lua_commands: &crate::bridge::LuaCommands,
) {
    let entity_id = new_player.entity_id;
    let profile = new_player.profile.clone();
    let sender = new_player.packet_tx;

    info!("{} entering play state (eid={})", profile.name, entity_id);

    let view_distance = config.view_distance as i32;

    // Try loading saved player data from disk
    let player_data_path = PathBuf::from(&config.world_dir)
        .join("playerdata")
        .join(format!("{}.dat", profile.uuid));
    let saved = if player_data_path.exists() {
        std::fs::read(&player_data_path)
            .ok()
            .and_then(|data| deserialize_player_data(&data))
    } else {
        None
    };

    // Determine values from saved data or defaults
    let spawn_pos = saved.as_ref().map(|s| s.position).unwrap_or(Vec3d::new(0.5, -59.0, 0.5));
    let player_yaw = saved.as_ref().map(|s| s.yaw).unwrap_or(0.0);
    let player_pitch = saved.as_ref().map(|s| s.pitch).unwrap_or(0.0);
    let player_game_mode = saved.as_ref().map(|s| s.game_mode).unwrap_or(GameMode::Survival);
    let player_health = saved.as_ref().map(|s| Health {
        current: s.health,
        max: 20.0,
        invulnerable_ticks: 60, // 3 seconds spawn invulnerability
    }).unwrap_or(Health {
        current: 20.0,
        max: 20.0,
        invulnerable_ticks: 60,
    });
    let player_food = saved.as_ref().map(|s| FoodData {
        food_level: s.food_level,
        saturation: s.saturation,
        exhaustion: s.exhaustion,
        tick_timer: 0,
    }).unwrap_or_default();
    let player_fall_distance = saved.as_ref().map(|s| s.fall_distance).unwrap_or(0.0);
    let player_held_slot = saved.as_ref().map(|s| s.held_slot).unwrap_or(0);
    let player_xp = saved.as_ref().map(|s| ExperienceData {
        level: s.xp_level,
        progress: s.xp_progress,
        total_xp: s.xp_total,
    }).unwrap_or_default();
    let player_inventory = saved.as_ref().map(|s| {
        let mut inv = Inventory::new();
        inv.slots = s.slots.clone();
        inv
    }).unwrap_or_else(Inventory::new);

    // Send Join Game
    let _ = sender.send(InternalPacket::JoinGame {
        entity_id,
        is_hardcore: false,
        dimension_names: vec!["minecraft:overworld".into()],
        max_players: config.max_players as i32,
        view_distance,
        simulation_distance: view_distance,
        reduced_debug_info: false,
        enable_respawn_screen: true,
        do_limited_crafting: false,
        dimension_type: 0,
        dimension_name: "minecraft:overworld".into(),
        hashed_seed: 0,
        game_mode: player_game_mode,
        previous_game_mode: -1,
        is_debug: false,
        is_flat: true,
        portal_cooldown: 0,
        enforces_secure_chat: false,
    });

    // Declare commands for tab completion (includes Lua-registered commands)
    let _ = sender.send(build_command_tree(lua_commands));

    // Send current world time
    let _ = sender.send(InternalPacket::UpdateTime {
        world_age: world_state.world_age,
        time_of_day: world_state.time_of_day,
    });

    let center_cx = (spawn_pos.x.floor() as i32) >> 4;
    let center_cz = (spawn_pos.z.floor() as i32) >> 4;

    // Set center chunk
    let _ = sender.send(InternalPacket::SetCenterChunk {
        chunk_x: center_cx,
        chunk_z: center_cz,
    });

    // Send chunks using batch protocol
    send_chunks_around(&sender, world_state, center_cx, center_cz, view_distance);

    // Teleport player to spawn
    let _ = sender.send(InternalPacket::SynchronizePlayerPosition {
        position: spawn_pos,
        yaw: player_yaw,
        pitch: player_pitch,
        flags: 0,
        teleport_id: 1,
    });

    // Start waiting for level chunks
    let _ = sender.send(InternalPacket::GameEvent {
        event: 13,
        value: 0.0,
    });

    // Default spawn position (must come after GameEvent)
    let _ = sender.send(InternalPacket::SetDefaultSpawnPosition {
        position: BlockPos::new(0, -60, 0),
        angle: 0.0,
    });

    // Send tab list: add this player to all existing players, and all existing to this player
    // First, send all existing players to the new player
    let mut existing_entries: Vec<PlayerInfoEntry> = Vec::new();
    for (_eid, (p, gm)) in world.query::<(&Profile, &PlayerGameMode)>().iter() {
        existing_entries.push(PlayerInfoEntry {
            uuid: p.0.uuid,
            name: Some(p.0.name.clone()),
            properties: p
                .0
                .properties
                .iter()
                .map(|pr| (pr.name.clone(), pr.value.clone(), pr.signature.clone()))
                .collect(),
            game_mode: Some(gm.0.id() as i32),
            listed: Some(true),
            ping: Some(0),
            display_name: None,
        });
    }

    // Add the new player's own entry
    let new_entry = PlayerInfoEntry {
        uuid: profile.uuid,
        name: Some(profile.name.clone()),
        properties: profile
            .properties
            .iter()
            .map(|pr| (pr.name.clone(), pr.value.clone(), pr.signature.clone()))
            .collect(),
        game_mode: Some(player_game_mode.id() as i32),
        listed: Some(true),
        ping: Some(0),
        display_name: None,
    };

    // Send all existing + self to new player
    let mut all_entries = existing_entries;
    all_entries.push(new_entry.clone());

    let actions = player_info_actions::ADD_PLAYER
        | player_info_actions::UPDATE_GAME_MODE
        | player_info_actions::UPDATE_LISTED
        | player_info_actions::UPDATE_LATENCY;

    let _ = sender.send(InternalPacket::PlayerInfoUpdate {
        actions,
        players: all_entries,
    });

    // Broadcast the new player to all existing players
    broadcast_to_all(
        world,
        &InternalPacket::PlayerInfoUpdate {
            actions,
            players: vec![new_entry],
        },
    );

    // Send inventory (loaded or empty)
    let _ = sender.send(InternalPacket::SetContainerContent {
        window_id: 0,
        state_id: player_inventory.state_id,
        slots: player_inventory.to_slot_vec(),
        carried_item: None,
    });

    // Send health/food bar
    let _ = sender.send(InternalPacket::SetHealth {
        health: player_health.current,
        food: player_food.food_level,
        saturation: player_food.saturation,
    });

    // Send XP bar
    let _ = sender.send(InternalPacket::SetExperience {
        progress: player_xp.progress,
        level: player_xp.level,
        total_xp: player_xp.total_xp,
    });

    // Spawn ECS entity (hecs supports up to 16-tuple, so we split)
    let player_entity = world.spawn((
        EntityId(entity_id),
        Profile(profile.clone()),
        Position(spawn_pos),
        Rotation {
            yaw: player_yaw,
            pitch: player_pitch,
        },
        OnGround(true),
        PlayerGameMode(player_game_mode),
        ConnectionSender(sender),
        ChunkPosition {
            chunk_x: center_cx,
            chunk_z: center_cz,
        },
        ViewDistance(view_distance),
        KeepAlive::new(),
        TrackedEntities::new(),
        PreviousPosition(spawn_pos),
        PreviousRotation {
            yaw: player_yaw,
            pitch: player_pitch,
        },
        player_inventory,
        HeldSlot(player_held_slot),
    ));
    // Additional components (exceeds 16-tuple limit)
    let _ = world.insert(player_entity, (
        player_health,
        player_food,
        FallDistance(player_fall_distance),
        MovementState { sprinting: false, sneaking: false },
        AttackCooldown::default(),
        player_xp,
    ));

    inbound_receivers.insert(entity_id, new_player.packet_rx);

    // Fire Lua event
    scripting.fire_event_in_context(
        "player_join",
        &[("name", &profile.name)],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );
}

fn handle_disconnect(
    world: &mut World,
    world_state: &mut WorldState,
    inbound_receivers: &mut HashMap<i32, mpsc::UnboundedReceiver<InboundPacket>>,
    entity_id: i32,
    _adapter: &V1_21Adapter,
    scripting: &ScriptRuntime,
) {
    inbound_receivers.remove(&entity_id);

    // Find and remove the entity
    let mut to_despawn = None;
    let mut player_uuid = None;
    let mut player_name = String::new();

    for (e, (eid, profile)) in world.query::<(&EntityId, &Profile)>().iter() {
        if eid.0 == entity_id {
            to_despawn = Some(e);
            player_uuid = Some(profile.0.uuid);
            player_name = profile.0.name.clone();
            break;
        }
    }

    if let Some(entity) = to_despawn {
        // Clean up open container (crafting grid items are lost on disconnect)
        let _ = world.remove_one::<OpenContainer>(entity);

        // Save player data BEFORE despawn (ECS components are needed)
        if let Some(uuid) = player_uuid {
            if let Some(data) = serialize_player_data(world, entity) {
                let _ = world_state.save_tx.send(SaveOp::Player(uuid, data));
            }
        }
        let _ = world.despawn(entity);
    }

    if let Some(uuid) = player_uuid {
        info!("{} disconnected", player_name);

        // Broadcast tab list removal
        broadcast_to_all(
            world,
            &InternalPacket::PlayerInfoRemove {
                uuids: vec![uuid],
            },
        );

        // Remove from all players' tracked entities and send despawn
        for (_e, (tracked, sender)) in world
            .query::<(&mut TrackedEntities, &ConnectionSender)>()
            .iter()
        {
            if tracked.visible.remove(&entity_id) {
                let _ = sender.0.send(InternalPacket::RemoveEntities {
                    entity_ids: vec![entity_id],
                });
            }
        }

        // Fire Lua event
        scripting.fire_event_in_context(
            "player_leave",
            &[("name", &player_name)],
            world as *mut _ as *mut (),
            world_state as *mut _ as *mut (),
        );
    }
}

fn process_packet(
    _config: &ServerConfig,
    _adapter: &V1_21Adapter,
    world: &mut World,
    world_state: &mut WorldState,
    pkt: InboundPacket,
    scripting: &ScriptRuntime,
    lua_commands: &crate::bridge::LuaCommands,
    block_overrides: &crate::bridge::BlockOverrides,
    next_eid: &Arc<AtomicI32>,
) {
    let entity_id = pkt.entity_id;

    // Find the hecs entity for this player
    let entity = {
        let mut found = None;
        for (e, eid) in world.query::<&EntityId>().iter() {
            if eid.0 == entity_id {
                found = Some(e);
                break;
            }
        }
        match found {
            Some(e) => e,
            None => return, // Player already disconnected
        }
    };

    match pkt.packet {
        InternalPacket::ConfirmTeleportation { teleport_id } => {
            debug!("Teleport confirmed: {}", teleport_id);
        }

        InternalPacket::PlayerPosition {
            x,
            y,
            z,
            on_ground,
        } => {
            handle_player_movement(world, world_state, entity, entity_id, x, y, z, None, on_ground, scripting);
        }

        InternalPacket::PlayerPositionAndRotation {
            x,
            y,
            z,
            yaw,
            pitch,
            on_ground,
        } => {
            if let Ok(mut rot) = world.get::<&mut Rotation>(entity) {
                rot.yaw = yaw;
                rot.pitch = pitch;
            }
            handle_player_movement(world, world_state, entity, entity_id, x, y, z, None, on_ground, scripting);
        }

        InternalPacket::PlayerRotation {
            yaw,
            pitch,
            on_ground,
        } => {
            if let Ok(mut rot) = world.get::<&mut Rotation>(entity) {
                rot.yaw = yaw;
                rot.pitch = pitch;
            }
            if let Ok(mut og) = world.get::<&mut OnGround>(entity) {
                og.0 = on_ground;
            }
        }

        InternalPacket::PlayerOnGround { on_ground } => {
            if let Ok(mut og) = world.get::<&mut OnGround>(entity) {
                og.0 = on_ground;
            }
        }

        InternalPacket::KeepAliveServerbound { id: ka_id } => {
            if let Ok(mut ka) = world.get::<&mut KeepAlive>(entity) {
                if ka.pending == Some(ka_id) {
                    ka.pending = None;
                    ka.last_response = Instant::now();
                }
            }
        }

        InternalPacket::BlockDig {
            status,
            position,
            sequence,
            ..
        } => {
            let game_mode = world
                .get::<&PlayerGameMode>(entity)
                .map(|gm| gm.0)
                .unwrap_or(GameMode::Survival);

            match status {
                // Started Digging
                0 => {
                    if game_mode == GameMode::Creative {
                        // Creative mode: instant break
                        complete_block_break(
                            world, world_state, entity, entity_id, &position, sequence,
                            scripting, block_overrides, next_eid,
                        );
                    } else {
                        // Survival mode: check block hardness
                        let block_state = world_state.get_block(&position);
                        let held_item_id = {
                            let slot =
                                world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                            world
                                .get::<&Inventory>(entity)
                                .ok()
                                .and_then(|inv| inv.held_item(slot).as_ref().map(|i| i.item_id))
                        };
                        match calculate_break_ticks(block_state, held_item_id, block_overrides) {
                            None => {
                                // Unbreakable block, just ack
                                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                    let _ = sender.0.send(
                                        InternalPacket::AcknowledgeBlockChange { sequence },
                                    );
                                }
                            }
                            Some(0) => {
                                // Instant break (hardness == 0)
                                complete_block_break(
                                    world, world_state, entity, entity_id, &position,
                                    sequence, scripting, block_overrides, next_eid,
                                );
                            }
                            Some(ticks) => {
                                // Timed break: insert BreakingBlock component
                                let _ = world.insert_one(
                                    entity,
                                    BreakingBlock {
                                        position,
                                        block_state,
                                        started_tick: world_state.tick_count,
                                        total_ticks: ticks,
                                        last_stage: -1,
                                    },
                                );
                                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                    let _ = sender.0.send(
                                        InternalPacket::AcknowledgeBlockChange { sequence },
                                    );
                                }
                            }
                        }
                    }
                }
                // Cancelled Digging
                1 => {
                    let _ = world.remove_one::<BreakingBlock>(entity);
                    // Clear destroy stage animation
                    broadcast_to_all(
                        world,
                        &InternalPacket::SetBlockDestroyStage {
                            entity_id,
                            position,
                            destroy_stage: -1,
                        },
                    );
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ =
                            sender
                                .0
                                .send(InternalPacket::AcknowledgeBlockChange { sequence });
                    }
                }
                // Finished Digging
                2 => {
                    let valid = if let Ok(breaking) = world.get::<&BreakingBlock>(entity) {
                        let elapsed = world_state
                            .tick_count
                            .saturating_sub(breaking.started_tick);
                        // Allow 2 tick tolerance
                        elapsed + 2 >= breaking.total_ticks
                    } else {
                        false
                    };

                    let _ = world.remove_one::<BreakingBlock>(entity);

                    if valid {
                        complete_block_break(
                            world, world_state, entity, entity_id, &position, sequence,
                            scripting, block_overrides, next_eid,
                        );
                    } else {
                        // Too fast or no breaking component — just ack without breaking
                        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                            let _ = sender
                                .0
                                .send(InternalPacket::AcknowledgeBlockChange { sequence });
                        }
                    }
                }
                // Drop Item (Q key — single item)
                3 => {
                    drop_held_item(world, world_state, entity, entity_id, false, next_eid, scripting);
                }
                // Drop Item Stack (Ctrl+Q — full stack)
                4 => {
                    drop_held_item(world, world_state, entity, entity_id, true, next_eid, scripting);
                }
                _ => {}
            }
        }

        InternalPacket::BlockPlace {
            position,
            face,
            sequence,
            ..
        } => {
            // Check if the target block is a container — open it instead of placing
            let target_block = world_state.get_block(&position);
            let target_name = pickaxe_data::block_state_to_name(target_block).unwrap_or("");
            let is_container = matches!(target_name, "chest" | "furnace" | "lit_furnace" | "crafting_table");
            let sneaking = world.get::<&MovementState>(entity).map(|m| m.sneaking).unwrap_or(false);

            if is_container && !sneaking {
                let name = world.get::<&Profile>(entity).map(|p| p.0.name.clone()).unwrap_or_default();
                let cancelled = scripting.fire_event_in_context(
                    "container_open",
                    &[
                        ("name", &name),
                        ("block_type", target_name),
                        ("x", &position.x.to_string()),
                        ("y", &position.y.to_string()),
                        ("z", &position.z.to_string()),
                    ],
                    world as *mut _ as *mut (),
                    world_state as *mut _ as *mut (),
                );

                if !cancelled {
                    open_container(world, world_state, entity, &position, target_name);
                }

                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                    let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                }
                return;
            }

            // Check if the target block is interactive (doors, trapdoors, fence gates, levers, buttons)
            if let Some(new_state) = pickaxe_data::toggle_interactive_block(target_block) {
                if !sneaking {
                    let name = world.get::<&Profile>(entity).map(|p| p.0.name.clone()).unwrap_or_default();
                    let cancelled = scripting.fire_event_in_context(
                        "block_interact",
                        &[
                            ("name", &name),
                            ("block_type", target_name),
                            ("x", &position.x.to_string()),
                            ("y", &position.y.to_string()),
                            ("z", &position.z.to_string()),
                        ],
                        world as *mut _ as *mut (),
                        world_state as *mut _ as *mut (),
                    );

                    if !cancelled {
                        // Toggle the block state
                        world_state.set_block(&position, new_state);
                        broadcast_to_all(world, &InternalPacket::BlockUpdate {
                            position,
                            block_id: new_state,
                        });

                        // For doors, also toggle the other half
                        if let Some(half_offset) = pickaxe_data::door_other_half_offset(target_block) {
                            let other_pos = BlockPos::new(
                                position.x,
                                position.y + half_offset,
                                position.z,
                            );
                            let other_state = world_state.get_block(&other_pos);
                            if let Some(other_new) = pickaxe_data::toggle_interactive_block(other_state) {
                                world_state.set_block(&other_pos, other_new);
                                broadcast_to_all(world, &InternalPacket::BlockUpdate {
                                    position: other_pos,
                                    block_id: other_new,
                                });
                            }
                        }

                        // For buttons, schedule auto-reset
                        if let Some(ticks) = pickaxe_data::button_reset_ticks(target_block) {
                            // Only schedule reset when activating (toggling to powered=true)
                            // Check if new state is powered by toggling again — if it gives original, new_state is powered
                            if pickaxe_data::toggle_interactive_block(new_state) == Some(target_block) {
                                let _ = world.spawn((
                                    ButtonTimer { position, remaining_ticks: ticks },
                                ));
                            }
                        }

                        // Play interaction sound
                        let is_opening = pickaxe_data::toggle_interactive_block(new_state) == Some(target_block);
                        let sound = if target_name.contains("iron_door") || target_name.contains("iron_trapdoor") {
                            if is_opening { "block.iron_door.open" } else { "block.iron_door.close" }
                        } else if target_name.contains("door") || target_name.contains("trapdoor") || target_name.contains("fence_gate") {
                            if is_opening { "block.wooden_door.open" } else { "block.wooden_door.close" }
                        } else if target_name == "lever" {
                            "block.lever.click"
                        } else if target_name.contains("stone_button") || target_name.contains("polished_blackstone_button") {
                            if is_opening { "block.stone_button.click_on" } else { "block.stone_button.click_off" }
                        } else if target_name.contains("button") {
                            if is_opening { "block.wooden_button.click_on" } else { "block.wooden_button.click_off" }
                        } else {
                            "block.wooden_door.open"
                        };
                        play_sound_at_block(world, &position, sound, SOUND_BLOCKS, 1.0, 1.0);

                        debug!("{} interacted with {} at {:?}", name, target_name, position);
                    }

                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                    }
                    return;
                }
            }

            // Look up the held item to determine what block to place
            let block_id = {
                let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                let inv = world.get::<&Inventory>(entity);
                match inv {
                    Ok(inv) => {
                        match inv.held_item(held_slot) {
                            Some(item) => {
                                pickaxe_data::item_id_to_block_state(item.item_id).unwrap_or(0)
                            }
                            None => 0,
                        }
                    }
                    Err(_) => 0,
                }
            };

            if block_id == 0 {
                // Nothing to place (empty hand or non-block item)
                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                    let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                }
                return;
            }

            let target = offset_by_face(&position, face);

            let name = world
                .get::<&Profile>(entity)
                .map(|p| p.0.name.clone())
                .unwrap_or_default();

            // Fire event BEFORE the place — handlers can cancel
            let cancelled = scripting.fire_event_in_context(
                "block_place",
                &[
                    ("name", &name),
                    ("x", &target.x.to_string()),
                    ("y", &target.y.to_string()),
                    ("z", &target.z.to_string()),
                    ("block_id", &block_id.to_string()),
                ],
                world as *mut _ as *mut (),
                world_state as *mut _ as *mut (),
            );

            if cancelled {
                // Just ack without placing
                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                    let _ = sender
                        .0
                        .send(InternalPacket::AcknowledgeBlockChange { sequence });
                }
                return;
            }

            world_state.set_block(&target, block_id);

            // Create block entity for container blocks
            let block_name = pickaxe_data::block_state_to_name(block_id).unwrap_or("");
            match block_name {
                "chest" => {
                    world_state.set_block_entity(target, BlockEntity::Chest {
                        inventory: std::array::from_fn(|_| None),
                    });
                }
                "furnace" => {
                    world_state.set_block_entity(target, BlockEntity::Furnace {
                        input: None, fuel: None, output: None,
                        burn_time: 0, burn_duration: 0, cook_progress: 0, cook_total: 200,
                    });
                }
                _ => {}
            }

            // Consume item from inventory (survival mode only)
            let game_mode = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
            if game_mode != GameMode::Creative {
                let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                let slot_index = 36 + held_slot as usize;
                let mut inv = world.get::<&mut Inventory>(entity).unwrap();
                let slot_data = inv.slots[slot_index].clone();
                if let Some(item) = slot_data {
                    if item.count > 1 {
                        inv.set_slot(slot_index, Some(ItemStack::new(item.item_id, item.count - 1)));
                    } else {
                        inv.set_slot(slot_index, None);
                    }
                }
            }

            // Send to placing player
            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                let _ = sender.0.send(InternalPacket::BlockUpdate {
                    position: target,
                    block_id,
                });
                let _ = sender
                    .0
                    .send(InternalPacket::AcknowledgeBlockChange { sequence });
            }
            // Broadcast to others
            broadcast_except(
                world,
                entity_id,
                &InternalPacket::BlockUpdate {
                    position: target,
                    block_id,
                },
            );

            // Play block place sound
            let sound_group = pickaxe_data::block_state_to_name(block_id)
                .map(|n| pickaxe_data::block_sound_group(n))
                .unwrap_or("stone");
            play_sound_at_block(world, &target, &format!("block.{}.place", sound_group), SOUND_BLOCKS, 1.0, 0.8);

            debug!("{} placed block at {:?}", name, target);
        }

        InternalPacket::ChatMessage { message, .. } => {
            let name = world
                .get::<&Profile>(entity)
                .map(|p| p.0.name.clone())
                .unwrap_or_default();
            info!("<{}> {}", name, message);

            // Fire Lua event
            let cancelled = scripting.fire_event_in_context(
                "player_chat",
                &[("name", &name), ("message", &message)],
                world as *mut _ as *mut (),
                world_state as *mut _ as *mut (),
            );

            if !cancelled {
                let chat_text = format!("<{}> {}", name, message);
                broadcast_to_all(
                    world,
                    &InternalPacket::SystemChatMessage {
                        content: TextComponent::plain(&chat_text),
                        overlay: false,
                    },
                );
            }
        }

        InternalPacket::ChatCommand { command } => {
            let name = world
                .get::<&Profile>(entity)
                .map(|p| p.0.name.clone())
                .unwrap_or_default();
            info!("{} issued command: /{}", name, command);

            scripting.fire_event_in_context(
                "player_command",
                &[("name", &name), ("command", &command)],
                world as *mut _ as *mut (),
                world_state as *mut _ as *mut (),
            );

            let parts: Vec<&str> = command.splitn(2, ' ').collect();
            let cmd_name = parts[0];
            let args = if parts.len() > 1 { parts[1] } else { "" };

            match cmd_name {
                "gamemode" | "gm" => cmd_gamemode(world, entity, args),
                "tp" | "teleport" => cmd_tp(world, entity, args),
                "give" => cmd_give(world, entity, args),
                "kill" => cmd_kill(world, world_state, entity, entity_id, scripting),
                "say" => cmd_say(world, args, &name),
                "help" => cmd_help(world, entity, lua_commands),
                "time" => cmd_time(world, entity, args, world_state),
                _ => {
                    // Check Lua-registered commands
                    let handled = if let Ok(cmds) = lua_commands.lock() {
                        if let Some(lua_cmd) = cmds.iter().find(|c| c.name == cmd_name) {
                            let lua = scripting.lua();
                            // Set game context so bridge APIs work inside command handlers
                            lua.set_app_data(pickaxe_scripting::bridge::LuaGameContext {
                                world_ptr: world as *mut _ as *mut (),
                                world_state_ptr: world_state as *mut _ as *mut (),
                            });
                            let func: mlua::Result<mlua::Function> =
                                lua.registry_value(&lua_cmd.handler_key);
                            let result = if let Ok(func) = func {
                                if let Err(e) = func.call::<()>((name.clone(), args.to_string())) {
                                    warn!("Lua command /{} error: {}", cmd_name, e);
                                    send_message(
                                        world,
                                        entity,
                                        &format!("Command error: {}", e),
                                    );
                                }
                                true
                            } else {
                                false
                            };
                            lua.remove_app_data::<pickaxe_scripting::bridge::LuaGameContext>();
                            result
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !handled {
                        send_message(
                            world,
                            entity,
                            &format!("Unknown command: /{}", cmd_name),
                        );
                    }
                }
            }
        }

        InternalPacket::HeldItemChange { slot } => {
            if (0..=8).contains(&slot) {
                if let Ok(mut held) = world.get::<&mut HeldSlot>(entity) {
                    held.0 = slot as u8;
                }
            }
        }

        InternalPacket::CreativeInventoryAction { slot, item } => {
            if slot >= 0 {
                if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                    inv.set_slot(slot as usize, item);
                }
            }
        }

        InternalPacket::PlayerCommand { action, .. } => {
            match action {
                0 => {
                    if let Ok(mut ms) = world.get::<&mut MovementState>(entity) {
                        ms.sneaking = true;
                    }
                }
                1 => {
                    if let Ok(mut ms) = world.get::<&mut MovementState>(entity) {
                        ms.sneaking = false;
                    }
                }
                3 => {
                    // MC: can't sprint if food < 6 (SPRINT_LEVEL)
                    let food_level = world.get::<&FoodData>(entity).map(|f| f.food_level).unwrap_or(20);
                    if food_level >= 6 {
                        if let Ok(mut ms) = world.get::<&mut MovementState>(entity) {
                            ms.sprinting = true;
                        }
                    }
                }
                4 => {
                    if let Ok(mut ms) = world.get::<&mut MovementState>(entity) {
                        ms.sprinting = false;
                    }
                }
                _ => {}
            }
        }

        InternalPacket::ClientCommand { action } => {
            if action == 0 {
                respawn_player(world, world_state, entity, entity_id, scripting);
            }
        }

        InternalPacket::ClientCloseContainer { container_id } => {
            close_container(world, world_state, entity, container_id, next_eid, scripting);
        }

        InternalPacket::ContainerClick { window_id, state_id, slot, button, mode, ref changed_slots, ref carried_item } => {
            handle_container_click(world, world_state, entity, window_id, state_id, slot, button, mode, changed_slots, carried_item);
        }

        InternalPacket::UseItem { hand, sequence } => {
            // Acknowledge the action
            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
            }

            // Get the item in the used hand
            let item_id = {
                let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                let inv = match world.get::<&Inventory>(entity) {
                    Ok(inv) => inv,
                    Err(_) => return,
                };
                let slot_idx = if hand == 1 { 45 } else { 36 + held_slot as usize };
                match &inv.slots[slot_idx] {
                    Some(item) => item.item_id,
                    None => return,
                }
            };

            // Check if the item is food
            if let Some(props) = pickaxe_data::food_properties(item_id) {
                // Check if player can eat (not full, or canAlwaysEat)
                let food_level = world.get::<&FoodData>(entity).map(|f| f.food_level).unwrap_or(20);
                if food_level >= 20 && !props.can_always_eat {
                    return;
                }

                // Start eating — add EatingState component
                let _ = world.insert_one(entity, EatingState {
                    remaining_ticks: props.eat_ticks,
                    hand,
                    item_id,
                    nutrition: props.nutrition,
                    saturation_modifier: props.saturation_modifier,
                });
            }
        }

        InternalPacket::InteractEntity { entity_id: target_eid, action_type, sneaking, .. } => {
            if action_type == 1 {
                // ATTACK action
                handle_attack(world, world_state, entity, entity_id, target_eid, scripting);
            }
            let _ = sneaking; // used for future interact mechanics
        }

        InternalPacket::Swing { hand } => {
            // Broadcast arm swing animation to other players
            let animation = if hand == 1 { 3 } else { 0 }; // 0=main, 3=off
            broadcast_except(world, entity_id, &InternalPacket::EntityAnimation {
                entity_id,
                animation,
            });
        }

        InternalPacket::Unknown { .. } => {}
        _ => {}
    }
}

// === Container system ===

fn open_container(
    world: &mut World,
    world_state: &WorldState,
    entity: hecs::Entity,
    pos: &BlockPos,
    block_name: &str,
) {
    let (menu_type, title, menu) = match block_name {
        "chest" => (2, "Chest", Menu::Chest { pos: *pos }),
        "furnace" | "lit_furnace" => (14, "Furnace", Menu::Furnace { pos: *pos }),
        "crafting_table" => (12, "Crafting", Menu::CraftingTable {
            grid: std::array::from_fn(|_| None),
            result: None,
        }),
        _ => return,
    };

    // Assign container ID (1-255, never 0)
    let container_id = {
        let old = world.get::<&OpenContainer>(entity).map(|c| c.container_id).unwrap_or(0);
        old.wrapping_add(1).max(1)
    };

    let slots = build_container_slots(world_state, world, entity, &menu);

    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::OpenScreen {
            container_id: container_id as i32,
            menu_type,
            title: TextComponent::plain(title),
        });
        let _ = sender.0.send(InternalPacket::SetContainerContent {
            window_id: container_id,
            state_id: 1,
            slots,
            carried_item: None,
        });

        // For furnaces, send current progress
        if block_name == "furnace" || block_name == "lit_furnace" {
            if let Some(BlockEntity::Furnace { burn_time, burn_duration, cook_progress, cook_total, .. }) = world_state.get_block_entity(pos) {
                let _ = sender.0.send(InternalPacket::SetContainerData { container_id, property: 0, value: *burn_time });
                let _ = sender.0.send(InternalPacket::SetContainerData { container_id, property: 1, value: *burn_duration });
                let _ = sender.0.send(InternalPacket::SetContainerData { container_id, property: 2, value: *cook_progress });
                let _ = sender.0.send(InternalPacket::SetContainerData { container_id, property: 3, value: *cook_total });
            }
        }
    }

    let _ = world.insert_one(entity, OpenContainer {
        container_id,
        menu,
        state_id: 1,
    });
}

fn build_container_slots(
    world_state: &WorldState,
    world: &World,
    entity: hecs::Entity,
    menu: &Menu,
) -> Vec<Option<ItemStack>> {
    let player_inv = world.get::<&Inventory>(entity).ok();

    match menu {
        Menu::Chest { pos } => {
            let mut slots = Vec::with_capacity(63);
            if let Some(BlockEntity::Chest { inventory }) = world_state.get_block_entity(pos) {
                slots.extend_from_slice(inventory);
            } else {
                slots.resize(27, None);
            }
            if let Some(inv) = &player_inv {
                for i in 9..36 { slots.push(inv.slots[i].clone()); }
                for i in 36..45 { slots.push(inv.slots[i].clone()); }
            } else {
                slots.resize(63, None);
            }
            slots
        }
        Menu::Furnace { pos } => {
            let mut slots = Vec::with_capacity(39);
            if let Some(BlockEntity::Furnace { input, fuel, output, .. }) = world_state.get_block_entity(pos) {
                slots.push(input.clone());
                slots.push(fuel.clone());
                slots.push(output.clone());
            } else {
                slots.resize(3, None);
            }
            if let Some(inv) = &player_inv {
                for i in 9..36 { slots.push(inv.slots[i].clone()); }
                for i in 36..45 { slots.push(inv.slots[i].clone()); }
            } else {
                slots.resize(39, None);
            }
            slots
        }
        Menu::CraftingTable { grid, result } => {
            let mut slots = Vec::with_capacity(46);
            slots.push(result.clone());
            for item in grid { slots.push(item.clone()); }
            if let Some(inv) = &player_inv {
                for i in 9..36 { slots.push(inv.slots[i].clone()); }
                for i in 36..45 { slots.push(inv.slots[i].clone()); }
            } else {
                slots.resize(46, None);
            }
            slots
        }
    }
}

fn close_container(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    container_id: u8,
    next_eid: &Arc<AtomicI32>,
    scripting: &ScriptRuntime,
) {
    let open = match world.remove_one::<OpenContainer>(entity) {
        Ok(oc) => oc,
        Err(_) => return,
    };

    if open.container_id != container_id {
        // Wrong container, put it back
        let _ = world.insert_one(entity, open);
        return;
    }

    let block_type = match &open.menu {
        Menu::Chest { .. } => "chest",
        Menu::Furnace { .. } => "furnace",
        Menu::CraftingTable { .. } => "crafting_table",
    };

    // Drop crafting grid items back to the player
    if let Menu::CraftingTable { grid, .. } = &open.menu {
        let pos = world.get::<&Position>(entity).map(|p| p.0).unwrap_or(Vec3d::new(0.0, 64.0, 0.0));
        for item in grid.iter().flatten() {
            spawn_item_entity(world, world_state, next_eid,
                pos.x, pos.y + 1.0, pos.z,
                item.clone(), 0, scripting);
        }
    }

    // Save chunk for block entity containers (chest/furnace)
    match &open.menu {
        Menu::Chest { pos } | Menu::Furnace { pos } => {
            world_state.queue_chunk_save(pos.chunk_pos());
        }
        _ => {}
    }

    let name = world.get::<&Profile>(entity).map(|p| p.0.name.clone()).unwrap_or_default();
    scripting.fire_event_in_context(
        "container_close",
        &[("name", &name), ("block_type", block_type)],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );
}

/// Slot target for container click mapping.
enum SlotTarget {
    Container(usize),
    PlayerInventory(usize),
    CraftResult,
    CraftGrid(usize),
}

fn map_slot(menu: &Menu, window_slot: i16) -> Option<SlotTarget> {
    let s = window_slot as usize;
    match menu {
        Menu::Chest { .. } => {
            if s < 27 { Some(SlotTarget::Container(s)) }
            else if s < 54 { Some(SlotTarget::PlayerInventory(s - 27 + 9)) }
            else if s < 63 { Some(SlotTarget::PlayerInventory(s - 54 + 36)) }
            else { None }
        }
        Menu::Furnace { .. } => {
            if s < 3 { Some(SlotTarget::Container(s)) }
            else if s < 30 { Some(SlotTarget::PlayerInventory(s - 3 + 9)) }
            else if s < 39 { Some(SlotTarget::PlayerInventory(s - 30 + 36)) }
            else { None }
        }
        Menu::CraftingTable { .. } => {
            if s == 0 { Some(SlotTarget::CraftResult) }
            else if s <= 9 { Some(SlotTarget::CraftGrid(s - 1)) }
            else if s < 37 { Some(SlotTarget::PlayerInventory(s - 10 + 9)) }
            else if s < 46 { Some(SlotTarget::PlayerInventory(s - 37 + 36)) }
            else { None }
        }
    }
}

fn set_container_slot(
    world_state: &mut WorldState,
    world: &mut World,
    entity: hecs::Entity,
    menu: &mut Menu,
    target: &SlotTarget,
    item: Option<ItemStack>,
) {
    match target {
        SlotTarget::Container(idx) => {
            match menu {
                Menu::Chest { pos } => {
                    if let Some(BlockEntity::Chest { ref mut inventory }) = world_state.get_block_entity_mut(pos) {
                        inventory[*idx] = item;
                    }
                }
                Menu::Furnace { pos } => {
                    if let Some(BlockEntity::Furnace { ref mut input, ref mut fuel, ref mut output, .. }) = world_state.get_block_entity_mut(pos) {
                        match idx { 0 => *input = item, 1 => *fuel = item, 2 => *output = item, _ => {} }
                    }
                }
                _ => {}
            }
        }
        SlotTarget::PlayerInventory(idx) => {
            if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                inv.set_slot(*idx, item);
            }
        }
        SlotTarget::CraftGrid(idx) => {
            if let Menu::CraftingTable { ref mut grid, .. } = menu {
                grid[*idx] = item;
            }
        }
        SlotTarget::CraftResult => {} // Read-only
    }
}

fn handle_container_click(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    window_id: u8,
    client_state_id: i32,
    slot: i16,
    _button: i8,
    mode: i32,
    changed_slots: &[(i16, Option<ItemStack>)],
    carried_item: &Option<ItemStack>,
) {
    let mut open = match world.remove_one::<OpenContainer>(entity) {
        Ok(oc) => oc,
        Err(_) => return,
    };

    if window_id != open.container_id {
        let _ = world.insert_one(entity, open);
        return;
    }

    // Apply the client's proposed slot changes (trust-based for now)
    match mode {
        0 | 1 | 2 | 4 => {
            for (changed_slot, changed_item) in changed_slots {
                if let Some(t) = map_slot(&open.menu, *changed_slot) {
                    set_container_slot(world_state, world, entity, &mut open.menu, &t, changed_item.clone());
                }
            }
            // Handle crafting result take
            if slot >= 0 {
                if let Some(SlotTarget::CraftResult) = map_slot(&open.menu, slot) {
                    if let Menu::CraftingTable { ref mut grid, ref mut result } = open.menu {
                        for grid_slot in grid.iter_mut() {
                            if let Some(ref mut item) = grid_slot {
                                item.count -= 1;
                                if item.count <= 0 { *grid_slot = None; }
                            }
                        }
                        *result = lookup_crafting_recipe(grid);
                    }
                }
            }
            // Recalculate crafting result if grid changed
            if slot >= 0 {
                if let Some(SlotTarget::CraftGrid(_)) = map_slot(&open.menu, slot) {
                    if let Menu::CraftingTable { ref grid, ref mut result } = open.menu {
                        *result = lookup_crafting_recipe(grid);
                    }
                }
            }
        }
        _ => {} // Modes 3, 5, 6 — resync below
    }

    let new_state_id = client_state_id.wrapping_add(1);
    open.state_id = new_state_id;

    // Resync full container content
    let slots = build_container_slots(world_state, world, entity, &open.menu);
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SetContainerContent {
            window_id: open.container_id,
            state_id: new_state_id,
            slots,
            carried_item: carried_item.clone(),
        });
    }

    let _ = world.insert_one(entity, open);
}

/// Look up a crafting recipe from a 3x3 grid. Returns the result item if a recipe matches.
fn lookup_crafting_recipe(grid: &[Option<ItemStack>; 9]) -> Option<ItemStack> {
    let grid_ids: [i32; 9] = std::array::from_fn(|i| {
        grid[i].as_ref().map(|item| item.item_id).unwrap_or(0)
    });

    // Find the bounding box of non-empty slots
    let mut min_x = 3usize;
    let mut max_x = 0usize;
    let mut min_y = 3usize;
    let mut max_y = 0usize;
    for y in 0..3 {
        for x in 0..3 {
            if grid_ids[y * 3 + x] != 0 {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if min_x > max_x { return None; }

    let w = max_x - min_x + 1;
    let h = max_y - min_y + 1;

    // Extract the compact pattern
    let mut compact = [0i32; 9];
    for y in 0..h {
        for x in 0..w {
            compact[y * 3 + x] = grid_ids[(min_y + y) * 3 + (min_x + x)];
        }
    }

    for recipe in pickaxe_data::crafting_recipes() {
        if recipe.width as usize != w || recipe.height as usize != h { continue; }

        // Normal match
        let mut matches = true;
        for y in 0..h {
            for x in 0..w {
                if compact[y * 3 + x] != recipe.pattern[y * 3 + x] {
                    matches = false;
                    break;
                }
            }
            if !matches { break; }
        }
        if matches {
            return Some(ItemStack::new(recipe.result_id, recipe.result_count));
        }

        // Mirrored match (flip X)
        let mut matches = true;
        for y in 0..h {
            for x in 0..w {
                if compact[y * 3 + (w - 1 - x)] != recipe.pattern[y * 3 + x] {
                    matches = false;
                    break;
                }
            }
            if !matches { break; }
        }
        if matches {
            return Some(ItemStack::new(recipe.result_id, recipe.result_count));
        }
    }

    None
}

/// Handle player position update: fall distance, sprint exhaustion, jump exhaustion.
fn handle_player_movement(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    entity_id: i32,
    x: f64,
    y: f64,
    z: f64,
    _rotation: Option<(f32, f32)>,
    on_ground: bool,
    scripting: &ScriptRuntime,
) {
    let old_pos = world.get::<&Position>(entity).map(|p| p.0).unwrap_or(Vec3d::new(x, y, z));
    let old_on_ground = world.get::<&OnGround>(entity).map(|og| og.0).unwrap_or(true);
    let dy = y - old_pos.y;

    // Update position and on_ground
    if let Ok(mut pos) = world.get::<&mut Position>(entity) {
        pos.0 = Vec3d::new(x, y, z);
    }
    if let Ok(mut og) = world.get::<&mut OnGround>(entity) {
        og.0 = on_ground;
    }

    // Cancel eating if player moved horizontally
    let dx = x - old_pos.x;
    let dz = z - old_pos.z;
    if dx * dx + dz * dz > 0.0001 {
        let _ = world.remove_one::<EatingState>(entity);
    }

    // Fall distance tracking and fall damage
    let fall_damage = {
        if let Ok(mut fd) = world.get::<&mut FallDistance>(entity) {
            if on_ground {
                let damage = if fd.0 > 3.0 {
                    Some((fd.0 - 3.0).ceil())
                } else {
                    None
                };
                fd.0 = 0.0;
                damage
            } else {
                if dy < 0.0 {
                    fd.0 -= dy as f32; // dy is negative when falling, so subtract to accumulate
                }
                None
            }
        } else {
            None
        }
    };
    if let Some(damage) = fall_damage {
        apply_damage(world, world_state, entity, entity_id, damage, "fall", scripting);
    }

    // Sprint exhaustion (MC: 0.1 per meter while sprinting)
    let dx = x - old_pos.x;
    let dz = z - old_pos.z;
    let horiz_dist = ((dx * dx + dz * dz) as f32).sqrt();
    if horiz_dist > 0.01 {
        let sprinting = world.get::<&MovementState>(entity).map(|m| m.sprinting).unwrap_or(false);
        if sprinting {
            if let Ok(mut food) = world.get::<&mut FoodData>(entity) {
                food.exhaustion = (food.exhaustion + horiz_dist * 0.1).min(40.0);
            }
        }
    }

    // Jump exhaustion: transition from on_ground to !on_ground while moving upward
    // MC: 0.05 normal jump, 0.2 sprint jump
    if !on_ground && old_on_ground && dy > 0.0 {
        let sprinting = world.get::<&MovementState>(entity).map(|m| m.sprinting).unwrap_or(false);
        if let Ok(mut food) = world.get::<&mut FoodData>(entity) {
            food.exhaustion = (food.exhaustion + if sprinting { 0.2 } else { 0.05 }).min(40.0);
        }
    }

    handle_chunk_updates(world, world_state, entity);
    fire_move_event(world, world_state, entity, x, y, z, scripting);
}

/// Handle an attack on a target entity (PvP or item entity destruction).
fn handle_attack(
    world: &mut World,
    world_state: &mut WorldState,
    attacker: hecs::Entity,
    _attacker_eid: i32,
    target_eid: i32,
    scripting: &ScriptRuntime,
) {
    // Check game mode — creative can attack freely, spectators can't attack
    let game_mode = world
        .get::<&PlayerGameMode>(attacker)
        .map(|gm| gm.0)
        .unwrap_or(GameMode::Survival);
    if game_mode == GameMode::Spectator {
        return;
    }

    // Find target entity by entity ID
    let target = {
        let mut found = None;
        for (e, eid) in world.query::<&EntityId>().iter() {
            if eid.0 == target_eid {
                found = Some(e);
                break;
            }
        }
        match found {
            Some(t) => t,
            None => return,
        }
    };

    // Compute attack strength (cooldown). MC: 0.2 + strength^2 * 0.8
    // Full strength at 10 ticks (vanilla uses getAttackStrengthScale(0.5f))
    let strength = {
        let cooldown = world
            .get::<&AttackCooldown>(attacker)
            .map(|c| c.ticks_since_last_attack)
            .unwrap_or(100);
        let f = (cooldown as f32 / 10.0).min(1.0);
        f
    };

    // Reset attack cooldown
    if let Ok(mut cd) = world.get::<&mut AttackCooldown>(attacker) {
        cd.ticks_since_last_attack = 0;
    }

    // Check if target is an item entity — destroy it
    if world.get::<&ItemEntity>(target).is_ok() {
        // Remove item entity
        let item_eid = world.get::<&EntityId>(target).map(|e| e.0).unwrap_or(target_eid);
        let _ = world.despawn(target);

        // Broadcast removal
        broadcast_to_all(world, &InternalPacket::RemoveEntities {
            entity_ids: vec![item_eid],
        });

        // Remove from tracked entities
        for (_, tracked) in world.query_mut::<&mut TrackedEntities>() {
            tracked.visible.remove(&item_eid);
        }
        return;
    }

    // PvP: target must be a player
    if world.get::<&Profile>(target).is_err() {
        return; // Not a player, skip for now (no mobs yet)
    }

    // Calculate damage: base 1.0 (fist), scaled by strength
    let base_damage = 1.0_f32;
    let damage_scale = 0.2 + strength * strength * 0.8;
    let mut damage = base_damage * damage_scale;

    // Critical hit: falling, not on ground, strength > 0.9
    let on_ground = world.get::<&OnGround>(attacker).map(|og| og.0).unwrap_or(true);
    let fall_distance = world.get::<&FallDistance>(attacker).map(|fd| fd.0).unwrap_or(0.0);
    let is_sprinting = world.get::<&MovementState>(attacker).map(|ms| ms.sprinting).unwrap_or(false);
    let is_critical = strength > 0.9 && fall_distance > 0.0 && !on_ground && !is_sprinting;

    if is_critical {
        damage *= 1.5;
    }

    // Apply damage to target player
    let target_eid_val = world.get::<&EntityId>(target).map(|e| e.0).unwrap_or(target_eid);
    apply_damage(world, world_state, target, target_eid_val, damage, "player", scripting);

    // Play attack sound at attacker position
    let attacker_pos = world.get::<&Position>(attacker).map(|p| p.0).unwrap_or(Vec3d { x: 0.0, y: 0.0, z: 0.0 });
    let attack_sound = if is_critical {
        "entity.player.attack.crit"
    } else if strength > 0.9 {
        "entity.player.attack.strong"
    } else {
        "entity.player.attack.weak"
    };
    play_sound_at_entity(world, attacker_pos.x, attacker_pos.y, attacker_pos.z, attack_sound, SOUND_PLAYERS, 1.0, 1.0);

    // Broadcast critical hit particle if applicable
    if is_critical {
        broadcast_to_all(world, &InternalPacket::EntityAnimation {
            entity_id: target_eid_val,
            animation: 4, // CRITICAL_HIT
        });
    }

    // Knockback
    let attacker_yaw = world.get::<&Rotation>(attacker).map(|r| r.yaw).unwrap_or(0.0);
    let kb_strength = if is_sprinting { 1.4_f32 } else { 0.4 };
    let sin_yaw = (attacker_yaw * std::f32::consts::PI / 180.0).sin();
    let cos_yaw = (attacker_yaw * std::f32::consts::PI / 180.0).cos();
    let kb_x = (-sin_yaw * kb_strength * 0.5) as f64;
    let kb_z = (cos_yaw * kb_strength * 0.5) as f64;
    let kb_y = 0.4_f64;

    // Send velocity to target
    if let Ok(sender) = world.get::<&ConnectionSender>(target) {
        let _ = sender.0.send(InternalPacket::SetEntityVelocity {
            entity_id: target_eid_val,
            velocity_x: (kb_x * 8000.0) as i16,
            velocity_y: (kb_y * 8000.0) as i16,
            velocity_z: (kb_z * 8000.0) as i16,
        });
    }

    // Attack exhaustion: MC adds 0.1 per attack
    if let Ok(mut food) = world.get::<&mut FoodData>(attacker) {
        food.exhaustion = (food.exhaustion + 0.1).min(40.0);
    }

    // If sprinting, stop sprint after knockback attack
    if is_sprinting {
        if let Ok(mut ms) = world.get::<&mut MovementState>(attacker) {
            ms.sprinting = false;
        }
    }
}

/// Apply damage to a player entity with invulnerability check and Lua event.
fn apply_damage(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    entity_id: i32,
    damage: f32,
    source: &str,
    scripting: &ScriptRuntime,
) {
    // Check game mode — creative/spectator players don't take damage (except void)
    let game_mode = world.get::<&PlayerGameMode>(entity).map(|gm| gm.0).unwrap_or(GameMode::Survival);
    if game_mode == GameMode::Creative && source != "void" {
        return;
    }

    // Check invulnerability — MC checks invulnerableTime > 10 (half the 20-tick cooldown)
    let invuln = world.get::<&Health>(entity).map(|h| h.invulnerable_ticks > 10).unwrap_or(false);
    if invuln {
        return;
    }

    // Fire cancellable Lua event
    let name = world.get::<&Profile>(entity).map(|p| p.0.name.clone()).unwrap_or_default();
    let cancelled = scripting.fire_event_in_context(
        "player_damage",
        &[
            ("name", &name),
            ("amount", &format!("{:.1}", damage)),
            ("source", source),
        ],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );
    if cancelled {
        return;
    }

    // Cancel eating on damage
    let _ = world.remove_one::<EatingState>(entity);

    // Apply damage
    let (new_health, is_dead) = {
        let mut health = match world.get::<&mut Health>(entity) {
            Ok(h) => h,
            Err(_) => return,
        };
        health.current = (health.current - damage).max(0.0);
        health.invulnerable_ticks = 20;
        (health.current, health.current <= 0.0)
    };

    // Damage exhaustion (MC: causeFoodExhaustion with DamageSource.getExhaustion, default 0.1)
    if let Ok(mut food) = world.get::<&mut FoodData>(entity) {
        food.exhaustion = (food.exhaustion + 0.1).min(40.0);
    }

    // Send health update to damaged player
    let (food, sat) = world
        .get::<&FoodData>(entity)
        .map(|f| (f.food_level, f.saturation))
        .unwrap_or((20, 5.0));
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SetHealth {
            health: new_health,
            food,
            saturation: sat,
        });
    }

    // Broadcast hurt animation to all players
    broadcast_to_all(world, &InternalPacket::HurtAnimation {
        entity_id,
        yaw: 0.0,
    });

    // Play hurt sound at player position
    let pos = world.get::<&Position>(entity).map(|p| p.0).unwrap_or(Vec3d { x: 0.0, y: 0.0, z: 0.0 });
    play_sound_at_entity(world, pos.x, pos.y, pos.z, "entity.player.hurt", SOUND_PLAYERS, 1.0, 1.0);

    if is_dead {
        handle_player_death(world, world_state, entity, entity_id, source, scripting);
    }
}

/// Handle player death: send death screen, broadcast death message.
fn handle_player_death(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    entity_id: i32,
    source: &str,
    scripting: &ScriptRuntime,
) {
    let name = world.get::<&Profile>(entity).map(|p| p.0.name.clone()).unwrap_or_default();

    // Fire Lua event
    scripting.fire_event_in_context(
        "player_death",
        &[("name", &name), ("source", source)],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );

    // Death message
    let death_msg = match source {
        "fall" => format!("{} hit the ground too hard", name),
        "void" => format!("{} fell out of the world", name),
        "starve" => format!("{} starved to death", name),
        "kill" => format!("{} was killed", name),
        _ => format!("{} died", name),
    };

    // Send combat kill to the dead player (shows death screen)
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::PlayerCombatKill {
            player_id: entity_id,
            message: TextComponent::plain(&death_msg),
        });
    }

    // Broadcast entity event 3 (death animation) to all observers
    broadcast_to_all(world, &InternalPacket::EntityEvent {
        entity_id,
        event_id: 3,
    });

    // Broadcast death message to all players
    broadcast_to_all(world, &InternalPacket::SystemChatMessage {
        content: TextComponent::plain(&death_msg),
        overlay: false,
    });

    // Reset XP on death (vanilla drops level * 7 XP orbs, we just reset for now)
    if let Ok(mut xp) = world.get::<&mut ExperienceData>(entity) {
        xp.level = 0;
        xp.progress = 0.0;
        xp.total_xp = 0;
    }
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SetExperience {
            progress: 0.0,
            level: 0,
            total_xp: 0,
        });
    }
}

/// Respawn a player after death: reset health/food, teleport to spawn, resend chunks.
fn respawn_player(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    _entity_id: i32,
    scripting: &ScriptRuntime,
) {
    let name = world.get::<&Profile>(entity).map(|p| p.0.name.clone()).unwrap_or_default();
    let game_mode = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);

    // Send Respawn packet
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::Respawn {
            dimension_type: 0,
            dimension_name: "minecraft:overworld".to_string(),
            hashed_seed: 0,
            game_mode: game_mode.id(),
            previous_game_mode: -1,
            is_debug: false,
            is_flat: true,
            data_to_keep: 0x00,
            last_death_x: None,
            last_death_y: None,
            last_death_z: None,
            last_death_dimension: None,
            portal_cooldown: 0,
        });
    }

    // Reset health and food
    if let Ok(mut h) = world.get::<&mut Health>(entity) {
        h.current = 20.0;
        h.invulnerable_ticks = 60; // 3 seconds spawn invulnerability
    }
    if let Ok(mut f) = world.get::<&mut FoodData>(entity) {
        *f = FoodData::default();
    }
    if let Ok(mut fd) = world.get::<&mut FallDistance>(entity) {
        fd.0 = 0.0;
    }

    // Teleport to spawn
    let spawn = Vec3d::new(0.5, -59.0, 0.5);
    if let Ok(mut pos) = world.get::<&mut Position>(entity) {
        pos.0 = spawn;
    }

    let spawn_cx = (spawn.x.floor() as i32) >> 4;
    let spawn_cz = (spawn.z.floor() as i32) >> 4;

    // Update chunk position
    if let Ok(mut cp) = world.get::<&mut ChunkPosition>(entity) {
        cp.chunk_x = spawn_cx;
        cp.chunk_z = spawn_cz;
    }

    // Send position, health, chunks
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SetCenterChunk {
            chunk_x: spawn_cx,
            chunk_z: spawn_cz,
        });

        let view_distance = world.get::<&ViewDistance>(entity).map(|vd| vd.0).unwrap_or(10);
        send_chunks_around(&sender.0, world_state, spawn_cx, spawn_cz, view_distance);

        let _ = sender.0.send(InternalPacket::SynchronizePlayerPosition {
            position: spawn,
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
            teleport_id: 100,
        });
        let _ = sender.0.send(InternalPacket::SetHealth {
            health: 20.0,
            food: 20,
            saturation: 5.0,
        });
    }

    // Fire Lua event
    scripting.fire_event_in_context(
        "player_respawn",
        &[("name", &name)],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );
}

/// Tick void damage for players below Y=-128.
/// Increment attack cooldown ticks for all players.
fn tick_attack_cooldown(world: &mut World) {
    for (_, cd) in world.query_mut::<&mut AttackCooldown>() {
        if cd.ticks_since_last_attack < 100 {
            cd.ticks_since_last_attack += 1;
        }
    }
}

fn tick_void_damage(world: &mut World, world_state: &mut WorldState, scripting: &ScriptRuntime) {
    let mut to_damage: Vec<(hecs::Entity, i32)> = Vec::new();
    for (entity, (eid, pos)) in world.query::<(&EntityId, &Position)>().iter() {
        if world.get::<&Profile>(entity).is_ok() && pos.0.y < -128.0 {
            to_damage.push((entity, eid.0));
        }
    }
    for (entity, eid) in to_damage {
        apply_damage(world, world_state, entity, eid, 4.0, "void", scripting);
    }
}

/// Tick hunger/saturation system: exhaustion drain, natural regen, starvation.
/// Based on MC source FoodData.tick() and FoodConstants.java.
fn tick_health_hunger(
    world: &mut World,
    world_state: &mut WorldState,
    scripting: &ScriptRuntime,
    tick_count: u64,
) {
    let mut starvation_damage: Vec<(hecs::Entity, i32)> = Vec::new();
    let mut health_updates: Vec<(hecs::Entity, f32, i32, f32)> = Vec::new();
    let mut sprint_stop: Vec<hecs::Entity> = Vec::new();

    for (entity, (eid, health, food, gm)) in
        world.query::<(&EntityId, &mut Health, &mut FoodData, &PlayerGameMode)>().iter()
    {
        if health.current <= 0.0 {
            continue; // dead
        }

        // Skip hunger in creative/spectator
        if gm.0 == GameMode::Creative || gm.0 == GameMode::Spectator {
            continue;
        }

        // Tick invulnerability
        if health.invulnerable_ticks > 0 {
            health.invulnerable_ticks -= 1;
        }

        // Cap exhaustion at 40.0 (MC: exhaustionLevel capped at 40.0F)
        food.exhaustion = food.exhaustion.min(40.0);

        // Exhaustion drain at 4.0 threshold
        if food.exhaustion >= 4.0 {
            food.exhaustion -= 4.0;
            if food.saturation > 0.0 {
                food.saturation = (food.saturation - 1.0).max(0.0);
            } else {
                food.food_level = (food.food_level - 1).max(0);
            }
        }

        // MC: can't sprint if food < 6 (SPRINT_LEVEL)
        if food.food_level < 6 {
            sprint_stop.push(entity);
        }

        let is_hurt = health.current < health.max;

        // Saturated regen: food=20 and saturation>0 and hurt → heal every 10 ticks
        if food.food_level >= 20 && food.saturation > 0.0 && is_hurt {
            food.tick_timer += 1;
            if food.tick_timer >= 10 {
                let heal_amount = food.saturation.min(6.0) / 6.0;
                health.current = (health.current + heal_amount).min(health.max);
                food.exhaustion = (food.exhaustion + food.saturation.min(6.0)).min(40.0);
                food.tick_timer = 0;
            }
        }
        // Normal regen: food>=18, hurt → heal every 80 ticks
        else if food.food_level >= 18 && is_hurt {
            food.tick_timer += 1;
            if food.tick_timer >= 80 {
                health.current = (health.current + 1.0).min(health.max);
                food.exhaustion = (food.exhaustion + 6.0).min(40.0);
                food.tick_timer = 0;
            }
        }
        // Starvation: food==0 → damage every 80 ticks
        // MC: EASY caps at 5.0HP, NORMAL caps at 1.0HP, HARD no cap
        // We implement Normal difficulty behavior
        else if food.food_level == 0 {
            food.tick_timer += 1;
            if food.tick_timer >= 80 {
                if health.current > 1.0 {
                    starvation_damage.push((entity, eid.0));
                }
                food.tick_timer = 0;
            }
        } else {
            food.tick_timer = 0;
        }

        // Collect health updates for periodic sending
        health_updates.push((entity, health.current, food.food_level, food.saturation));
    }

    // Apply starvation damage (outside borrow)
    for (entity, eid) in starvation_damage {
        apply_damage(world, world_state, entity, eid, 1.0, "starve", scripting);
    }

    // Force stop sprinting for players with food < 6 (MC: SPRINT_LEVEL)
    for entity in sprint_stop {
        if let Ok(mut ms) = world.get::<&mut MovementState>(entity) {
            ms.sprinting = false;
        }
    }

    // Send SetHealth every 20 ticks (1 second) to keep client in sync
    if tick_count % 20 == 0 {
        for (entity, health, food, sat) in health_updates {
            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                let _ = sender.0.send(InternalPacket::SetHealth {
                    health,
                    food,
                    saturation: sat,
                });
            }
        }
    }
}

/// Tick eating progress: decrement timer, consume food when done.
fn tick_eating(world: &mut World) {
    let mut finished: Vec<(hecs::Entity, i32, i32, f32, i32)> = Vec::new();

    for (entity, eating) in world.query::<&mut EatingState>().iter() {
        eating.remaining_ticks -= 1;
        if eating.remaining_ticks <= 0 {
            finished.push((
                entity,
                eating.hand,
                eating.nutrition,
                eating.saturation_modifier,
                eating.item_id,
            ));
        }
    }

    for (entity, hand, nutrition, sat_mod, item_id) in finished {
        // Remove the EatingState component
        let _ = world.remove_one::<EatingState>(entity);

        // Apply food restoration
        if let Ok(mut food) = world.get::<&mut FoodData>(entity) {
            food.food_level = (food.food_level + nutrition).min(20);
            let sat_gain = nutrition as f32 * sat_mod * 2.0;
            food.saturation = (food.saturation + sat_gain).min(food.food_level as f32);
        }

        // Consume the item from the hand slot
        let held_slot = world
            .get::<&HeldSlot>(entity)
            .map(|h| h.0)
            .unwrap_or(0);
        let slot_idx = if hand == 1 { 45 } else { 36 + held_slot as usize };
        let new_slot_item = {
            let mut inv = match world.get::<&mut Inventory>(entity) {
                Ok(inv) => inv,
                Err(_) => continue,
            };
            if let Some(ref mut item) = inv.slots[slot_idx] {
                if item.item_id == item_id {
                    item.count -= 1;
                    if item.count <= 0 {
                        inv.slots[slot_idx] = None;
                    }
                }
            }
            inv.state_id = inv.state_id.wrapping_add(1);
            (inv.slots[slot_idx].clone(), inv.state_id)
        };

        // Send slot update to client
        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
            let _ = sender.0.send(InternalPacket::SetContainerSlot {
                window_id: 0,
                state_id: new_slot_item.1,
                slot: slot_idx as i16,
                item: new_slot_item.0,
            });
        }

        // Send updated health/food/saturation
        let (health, food_level, saturation) = {
            let h = world
                .get::<&Health>(entity)
                .map(|h| h.current)
                .unwrap_or(20.0);
            let f = world
                .get::<&FoodData>(entity)
                .map(|f| (f.food_level, f.saturation))
                .unwrap_or((20, 5.0));
            (h, f.0, f.1)
        };
        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
            let _ = sender.0.send(InternalPacket::SetHealth {
                health,
                food: food_level,
                saturation,
            });
        }
    }
}

/// Tick button auto-reset timers: decrement, and when expired, toggle the button back to unpowered.
fn tick_buttons(world: &mut World, world_state: &mut WorldState) {
    let mut expired: Vec<(hecs::Entity, BlockPos)> = Vec::new();

    for (entity, timer) in world.query::<&mut ButtonTimer>().iter() {
        timer.remaining_ticks = timer.remaining_ticks.saturating_sub(1);
        if timer.remaining_ticks == 0 {
            expired.push((entity, timer.position));
        }
    }

    for (entity, position) in expired {
        let _ = world.despawn(entity);
        let current_state = world_state.get_block(&position);
        if let Some(new_state) = pickaxe_data::toggle_interactive_block(current_state) {
            world_state.set_block(&position, new_state);
            broadcast_to_all(world, &InternalPacket::BlockUpdate {
                position,
                block_id: new_state,
            });
            // Play button click-off sound
            let block_name = pickaxe_data::block_state_to_name(current_state).unwrap_or("");
            let sound = if block_name.contains("stone") || block_name.contains("polished_blackstone") {
                "block.stone_button.click_off"
            } else {
                "block.wooden_button.click_off"
            };
            play_sound_at_block(world, &position, sound, SOUND_BLOCKS, 1.0, 1.0);
        }
    }
}

fn fire_move_event(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    x: f64,
    y: f64,
    z: f64,
    scripting: &ScriptRuntime,
) {
    let name = world
        .get::<&Profile>(entity)
        .map(|p| p.0.name.clone())
        .unwrap_or_default();
    scripting.fire_event_in_context(
        "player_move",
        &[
            ("name", &name),
            ("x", &format!("{:.1}", x)),
            ("y", &format!("{:.1}", y)),
            ("z", &format!("{:.1}", z)),
        ],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );
}

fn tick_keep_alive(_adapter: &V1_21Adapter, world: &mut World, tick_count: u64) {
    // Send keep-alive every 300 ticks (15 seconds)
    if tick_count % 300 != 0 {
        return;
    }

    let now = Instant::now();
    let mut to_kick: Vec<i32> = Vec::new();

    for (_e, (eid, ka, sender)) in world
        .query::<(&EntityId, &mut KeepAlive, &ConnectionSender)>()
        .iter()
    {
        // Kick if no response for 30 seconds
        if ka.pending.is_some() && now.duration_since(ka.last_response).as_secs() >= 30 {
            let _ = sender.0.send(InternalPacket::Disconnect {
                reason: TextComponent::plain("Timed out"),
            });
            to_kick.push(eid.0);
            continue;
        }

        // Send new keep-alive
        let ka_id = now.elapsed().as_millis() as i64;
        let _ = sender
            .0
            .send(InternalPacket::KeepAliveClientbound { id: ka_id });
        ka.pending = Some(ka_id);
        ka.last_sent = now;
    }
}

fn tick_entity_tracking(world: &mut World) {
    use std::collections::HashSet;

    // Collect all player data (observers)
    let mut player_data: Vec<(hecs::Entity, i32, Vec3d, f32, f32, bool, Uuid, i32, i32)> =
        Vec::new();
    for (e, (eid, pos, rot, og, profile, cp, _vd)) in world
        .query::<(
            &EntityId,
            &Position,
            &Rotation,
            &OnGround,
            &Profile,
            &ChunkPosition,
            &ViewDistance,
        )>()
        .iter()
    {
        player_data.push((
            e,
            eid.0,
            pos.0,
            rot.yaw,
            rot.pitch,
            og.0,
            profile.0.uuid,
            cp.chunk_x,
            cp.chunk_z,
        ));
    }

    // Collect all item entities
    struct ItemData {
        eid: i32,
        uuid: Uuid,
        pos: Vec3d,
        vel: Vec3d,
        item: ItemStack,
    }
    let mut item_data: Vec<ItemData> = Vec::new();
    for (_e, (eid, euuid, pos, vel, item_ent)) in world
        .query::<(&EntityId, &EntityUuid, &Position, &Velocity, &ItemEntity)>()
        .iter()
    {
        item_data.push(ItemData {
            eid: eid.0,
            uuid: euuid.0,
            pos: pos.0,
            vel: vel.0,
            item: item_ent.item.clone(),
        });
    }

    for i in 0..player_data.len() {
        let (observer_entity, _observer_eid, _, _, _, _, _, obs_cx, obs_cz) = player_data[i];

        let obs_vd = match world.get::<&ViewDistance>(observer_entity) {
            Ok(vd) => vd.0,
            Err(_) => continue,
        };

        let mut should_see: HashSet<i32> = HashSet::new();

        // Other players in view distance
        for j in 0..player_data.len() {
            if i == j {
                continue;
            }
            let (_, target_eid, _, _, _, _, _, tgt_cx, tgt_cz) = player_data[j];
            if (tgt_cx - obs_cx).abs() <= obs_vd && (tgt_cz - obs_cz).abs() <= obs_vd {
                should_see.insert(target_eid);
            }
        }

        // Item entities in view distance
        for item in &item_data {
            let item_cx = (item.pos.x.floor() as i32) >> 4;
            let item_cz = (item.pos.z.floor() as i32) >> 4;
            if (item_cx - obs_cx).abs() <= obs_vd && (item_cz - obs_cz).abs() <= obs_vd {
                should_see.insert(item.eid);
            }
        }

        let currently_tracked: HashSet<i32> = match world.get::<&TrackedEntities>(observer_entity) {
            Ok(te) => te.visible.clone(),
            Err(_) => continue,
        };

        let observer_sender = match world.get::<&ConnectionSender>(observer_entity) {
            Ok(s) => s.0.clone(),
            Err(_) => continue,
        };

        // Spawn new entities
        for &eid in should_see.difference(&currently_tracked) {
            // Check if it's a player
            if let Some(&(_, _, pos, yaw, pitch, _, uuid, _, _)) =
                player_data.iter().find(|d| d.1 == eid)
            {
                let _ = observer_sender.send(InternalPacket::SpawnEntity {
                    entity_id: eid,
                    entity_uuid: uuid,
                    entity_type: 128, // player
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                    pitch: degrees_to_angle(pitch),
                    yaw: degrees_to_angle(yaw),
                    head_yaw: degrees_to_angle(yaw),
                    data: 0,
                    velocity_x: 0,
                    velocity_y: 0,
                    velocity_z: 0,
                });
                let _ = observer_sender.send(InternalPacket::SetHeadRotation {
                    entity_id: eid,
                    head_yaw: degrees_to_angle(yaw),
                });
            } else if let Some(item) = item_data.iter().find(|d| d.eid == eid) {
                // Item entity
                let vx = (item.vel.x * 8000.0) as i16;
                let vy = (item.vel.y * 8000.0) as i16;
                let vz = (item.vel.z * 8000.0) as i16;
                let _ = observer_sender.send(InternalPacket::SpawnEntity {
                    entity_id: eid,
                    entity_uuid: item.uuid,
                    entity_type: 58, // item entity type
                    x: item.pos.x,
                    y: item.pos.y,
                    z: item.pos.z,
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 1, // required for item rendering
                    velocity_x: vx,
                    velocity_y: vy,
                    velocity_z: vz,
                });
                // Send metadata with item slot
                let metadata = build_item_metadata(&item.item);
                let _ = observer_sender.send(InternalPacket::SetEntityMetadata {
                    entity_id: eid,
                    metadata,
                });
            }
        }

        // Despawn removed entities
        let to_remove: Vec<i32> = currently_tracked.difference(&should_see).copied().collect();
        if !to_remove.is_empty() {
            let _ = observer_sender.send(InternalPacket::RemoveEntities {
                entity_ids: to_remove,
            });
        }

        // Update tracked set
        if let Ok(mut te) = world.get::<&mut TrackedEntities>(observer_entity) {
            te.visible = should_see;
        }
    }
}

fn tick_entity_movement_broadcast(world: &mut World) {
    // Collect player entities that moved or rotated (have PreviousRotation)
    let mut player_movers: Vec<(i32, Vec3d, Vec3d, f32, f32, f32, f32, bool)> = Vec::new();

    for (_e, (eid, pos, prev_pos, rot, prev_rot, og, _profile)) in world
        .query::<(
            &EntityId,
            &Position,
            &PreviousPosition,
            &Rotation,
            &PreviousRotation,
            &OnGround,
            &Profile,
        )>()
        .iter()
    {
        let pos_changed =
            pos.0.x != prev_pos.0.x || pos.0.y != prev_pos.0.y || pos.0.z != prev_pos.0.z;
        let rot_changed = rot.yaw != prev_rot.yaw || rot.pitch != prev_rot.pitch;
        if pos_changed || rot_changed {
            player_movers.push((
                eid.0,
                pos.0,
                prev_pos.0,
                rot.yaw,
                rot.pitch,
                prev_rot.yaw,
                prev_rot.pitch,
                og.0,
            ));
        }
    }

    // Collect item entities that moved (no rotation tracking needed)
    let mut item_movers: Vec<(i32, Vec3d, Vec3d, bool)> = Vec::new();
    for (_e, (eid, pos, prev_pos, og, _item)) in world
        .query::<(&EntityId, &Position, &PreviousPosition, &OnGround, &ItemEntity)>()
        .iter()
    {
        let pos_changed =
            pos.0.x != prev_pos.0.x || pos.0.y != prev_pos.0.y || pos.0.z != prev_pos.0.z;
        if pos_changed {
            item_movers.push((eid.0, pos.0, prev_pos.0, og.0));
        }
    }

    // For each player mover, send packets to all observers tracking them
    for &(mover_eid, new_pos, old_pos, yaw, pitch, _old_yaw, _old_pitch, on_ground) in &player_movers {
        let dx = ((new_pos.x - old_pos.x) * 4096.0) as i16;
        let dy = ((new_pos.y - old_pos.y) * 4096.0) as i16;
        let dz = ((new_pos.z - old_pos.z) * 4096.0) as i16;

        let pos_changed = dx != 0 || dy != 0 || dz != 0;

        let needs_teleport = (new_pos.x - old_pos.x).abs() > 8.0
            || (new_pos.y - old_pos.y).abs() > 8.0
            || (new_pos.z - old_pos.z).abs() > 8.0;

        for (_e, (eid, tracked, sender)) in world
            .query::<(&EntityId, &TrackedEntities, &ConnectionSender)>()
            .iter()
        {
            if eid.0 == mover_eid {
                continue;
            }
            if !tracked.visible.contains(&mover_eid) {
                continue;
            }

            if needs_teleport {
                let _ = sender.0.send(InternalPacket::TeleportEntity {
                    entity_id: mover_eid,
                    x: new_pos.x,
                    y: new_pos.y,
                    z: new_pos.z,
                    yaw: degrees_to_angle(yaw),
                    pitch: degrees_to_angle(pitch),
                    on_ground,
                });
            } else if pos_changed {
                let _ = sender.0.send(InternalPacket::UpdateEntityPositionAndRotation {
                    entity_id: mover_eid,
                    delta_x: dx,
                    delta_y: dy,
                    delta_z: dz,
                    yaw: degrees_to_angle(yaw),
                    pitch: degrees_to_angle(pitch),
                    on_ground,
                });
            } else {
                let _ = sender.0.send(InternalPacket::UpdateEntityRotation {
                    entity_id: mover_eid,
                    yaw: degrees_to_angle(yaw),
                    pitch: degrees_to_angle(pitch),
                    on_ground,
                });
            }

            // Always send head rotation
            let _ = sender.0.send(InternalPacket::SetHeadRotation {
                entity_id: mover_eid,
                head_yaw: degrees_to_angle(yaw),
            });
        }
    }

    // For each item mover, send position-only updates
    for &(mover_eid, new_pos, old_pos, on_ground) in &item_movers {
        let dx = ((new_pos.x - old_pos.x) * 4096.0) as i16;
        let dy = ((new_pos.y - old_pos.y) * 4096.0) as i16;
        let dz = ((new_pos.z - old_pos.z) * 4096.0) as i16;

        let needs_teleport = (new_pos.x - old_pos.x).abs() > 8.0
            || (new_pos.y - old_pos.y).abs() > 8.0
            || (new_pos.z - old_pos.z).abs() > 8.0;

        for (_e, (eid, tracked, sender)) in world
            .query::<(&EntityId, &TrackedEntities, &ConnectionSender)>()
            .iter()
        {
            if eid.0 == mover_eid {
                continue;
            }
            if !tracked.visible.contains(&mover_eid) {
                continue;
            }

            if needs_teleport {
                let _ = sender.0.send(InternalPacket::TeleportEntity {
                    entity_id: mover_eid,
                    x: new_pos.x,
                    y: new_pos.y,
                    z: new_pos.z,
                    yaw: 0,
                    pitch: 0,
                    on_ground,
                });
            } else {
                let _ = sender.0.send(InternalPacket::UpdateEntityPosition {
                    entity_id: mover_eid,
                    delta_x: dx,
                    delta_y: dy,
                    delta_z: dz,
                    on_ground,
                });
            }
        }
    }

    // Update previous positions and rotations for all entities that have them
    for (_e, (pos, prev_pos)) in world
        .query::<(&Position, &mut PreviousPosition)>()
        .iter()
    {
        prev_pos.0 = pos.0;
    }

    for (_e, (rot, prev_rot)) in world
        .query::<(&Rotation, &mut PreviousRotation)>()
        .iter()
    {
        prev_rot.yaw = rot.yaw;
        prev_rot.pitch = rot.pitch;
    }
}

/// Advance world time each tick. Broadcast UpdateTime every 20 ticks (1 second).
fn tick_world_time(world: &World, world_state: &mut WorldState, tick_count: u64) {
    world_state.world_age += 1;
    world_state.time_of_day = (world_state.time_of_day + 1) % 24000;

    // Broadcast time update every 20 ticks (once per second)
    if tick_count % 20 == 0 {
        broadcast_to_all(world, &InternalPacket::UpdateTime {
            world_age: world_state.world_age,
            time_of_day: world_state.time_of_day,
        });
    }
}

/// Calculate how many ticks it takes to break a block in survival mode.
/// Returns None if the block is unbreakable, Some(0) for instant break, Some(ticks) otherwise.
/// Consults Lua block overrides before falling back to codegen data.
fn calculate_break_ticks(
    block_state: i32,
    held_item_id: Option<i32>,
    block_overrides: &crate::bridge::BlockOverrides,
) -> Option<u64> {
    let block_name = pickaxe_data::block_state_to_name(block_state);

    // Get hardness: override first, then codegen
    let (hardness, diggable) = {
        let override_hardness = block_name.and_then(|name| {
            block_overrides
                .lock()
                .ok()
                .and_then(|map| map.get(name).and_then(|o| o.hardness))
        });
        match override_hardness {
            Some(h) => (h, h >= 0.0),
            None => pickaxe_data::block_state_to_hardness(block_state)?,
        }
    };

    if !diggable || hardness < 0.0 {
        return None;
    }
    if hardness == 0.0 {
        return Some(0);
    }

    // Get harvest tools: override first, then codegen
    let has_correct_tool = {
        let override_tools = block_name.and_then(|name| {
            block_overrides
                .lock()
                .ok()
                .and_then(|map| map.get(name).and_then(|o| o.harvest_tools.clone()))
        });
        match override_tools {
            Some(ref tools) => held_item_id
                .map(|id| tools.contains(&id))
                .unwrap_or(false),
            None => {
                if let Some(required) = pickaxe_data::block_state_to_harvest_tools(block_state) {
                    held_item_id
                        .map(|id| required.contains(&id))
                        .unwrap_or(false)
                } else {
                    true
                }
            }
        }
    };

    let seconds = if has_correct_tool {
        hardness * 1.5
    } else {
        hardness * 5.0
    };
    Some((seconds * 20.0).ceil() as u64)
}

/// Complete a block break: fire pre-event, set to air, send updates, handle drops.
/// If the Lua event is cancelled, sends block correction to prevent desync.
fn complete_block_break(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    entity_id: i32,
    position: &BlockPos,
    sequence: i32,
    scripting: &ScriptRuntime,
    block_overrides: &crate::bridge::BlockOverrides,
    next_eid: &Arc<AtomicI32>,
) {
    let old_block = world_state.get_block(position);

    let name = world
        .get::<&Profile>(entity)
        .map(|p| p.0.name.clone())
        .unwrap_or_default();

    // Fire event BEFORE the break — handlers can cancel
    let cancelled = scripting.fire_event_in_context(
        "block_break",
        &[
            ("name", &name),
            ("x", &position.x.to_string()),
            ("y", &position.y.to_string()),
            ("z", &position.z.to_string()),
            ("block_id", &old_block.to_string()),
        ],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );

    if cancelled {
        // Send block correction (restore original) + ack to prevent desync
        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
            let _ = sender.0.send(InternalPacket::BlockUpdate {
                position: *position,
                block_id: old_block,
            });
            let _ = sender
                .0
                .send(InternalPacket::AcknowledgeBlockChange { sequence });
        }
        // Clear destroy stage animation
        broadcast_to_all(
            world,
            &InternalPacket::SetBlockDestroyStage {
                entity_id,
                position: *position,
                destroy_stage: -1,
            },
        );
        return;
    }

    // Proceed with the break
    world_state.set_block(position, 0);

    // Mining exhaustion (MC: 0.005 per block broken)
    if let Ok(mut food) = world.get::<&mut FoodData>(entity) {
        food.exhaustion = (food.exhaustion + 0.005).min(40.0);
    }

    // Send block update + ack to the breaking player
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::BlockUpdate {
            position: *position,
            block_id: 0,
        });
        let _ = sender
            .0
            .send(InternalPacket::AcknowledgeBlockChange { sequence });
    }

    // Broadcast block update to other players
    broadcast_except(
        world,
        entity_id,
        &InternalPacket::BlockUpdate {
            position: *position,
            block_id: 0,
        },
    );

    // Clear destroy stage animation for all players
    broadcast_to_all(
        world,
        &InternalPacket::SetBlockDestroyStage {
            entity_id,
            position: *position,
            destroy_stage: -1,
        },
    );

    // Play block break sound
    let sound_group = pickaxe_data::block_state_to_name(old_block)
        .map(|n| pickaxe_data::block_sound_group(n))
        .unwrap_or("stone");
    play_sound_at_block(world, position, &format!("block.{}.break", sound_group), SOUND_BLOCKS, 1.0, 0.8);

    // Award XP for ore mining (survival only)
    let xp_amount = block_xp_drop(old_block);
    if xp_amount > 0 {
        let gm = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
        if gm == GameMode::Survival {
            award_xp(world, entity, xp_amount);
        }
    }

    // Handle drops in survival mode
    let game_mode = world
        .get::<&PlayerGameMode>(entity)
        .map(|gm| gm.0)
        .unwrap_or(GameMode::Survival);

    if game_mode == GameMode::Survival {
        let block_name = pickaxe_data::block_state_to_name(old_block);

        // Check if player has the correct tool for drops (override first, then codegen)
        let has_correct_tool = {
            let override_tools = block_name.and_then(|name| {
                block_overrides
                    .lock()
                    .ok()
                    .and_then(|map| map.get(name).and_then(|o| o.harvest_tools.clone()))
            });
            match override_tools {
                Some(ref tools) => {
                    let held_item_id = {
                        let slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                        world
                            .get::<&Inventory>(entity)
                            .ok()
                            .and_then(|inv| inv.held_item(slot).as_ref().map(|i| i.item_id))
                    };
                    held_item_id
                        .map(|id| tools.contains(&id))
                        .unwrap_or(false)
                }
                None => {
                    if let Some(required) =
                        pickaxe_data::block_state_to_harvest_tools(old_block)
                    {
                        let held_item_id = {
                            let slot =
                                world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                            world
                                .get::<&Inventory>(entity)
                                .ok()
                                .and_then(|inv| inv.held_item(slot).as_ref().map(|i| i.item_id))
                        };
                        held_item_id
                            .map(|id| required.contains(&id))
                            .unwrap_or(false)
                    } else {
                        true // No tool requirement
                    }
                }
            }
        };

        if has_correct_tool {
            // Get drops: override first, then codegen
            let override_drops = block_name.and_then(|name| {
                block_overrides
                    .lock()
                    .ok()
                    .and_then(|map| map.get(name).and_then(|o| o.drops.clone()))
            });
            let drop_ids: Vec<i32> = match override_drops {
                Some(ids) => ids,
                None => pickaxe_data::block_state_to_drops(old_block).to_vec(),
            };
            for &drop_item_id in &drop_ids {
                spawn_item_entity(
                    world,
                    world_state,
                    next_eid,
                    position.x as f64 + 0.5,
                    position.y as f64 + 0.25,
                    position.z as f64 + 0.5,
                    ItemStack::new(drop_item_id, 1),
                    10, // pickup delay ticks
                    scripting,
                );
            }
        }
    }

    // Remove block entity and drop contents
    if let Some(block_entity) = world_state.remove_block_entity(position) {
        let items: Vec<ItemStack> = match block_entity {
            BlockEntity::Chest { inventory } => {
                inventory.into_iter().flatten().collect()
            }
            BlockEntity::Furnace { input, fuel, output, .. } => {
                [input, fuel, output].into_iter().flatten().collect()
            }
        };
        for item in items {
            spawn_item_entity(
                world, world_state, next_eid,
                position.x as f64 + 0.5, position.y as f64 + 0.5, position.z as f64 + 0.5,
                item, 10, scripting,
            );
        }
    }

    debug!("{} broke block at {:?} (was {})", name, position, old_block);
}

/// Handle a player dropping an item from their held slot.
fn drop_held_item(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    _entity_id: i32,
    drop_stack: bool,
    next_eid: &Arc<AtomicI32>,
    scripting: &ScriptRuntime,
) {
    let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
    let slot_index = 36 + held_slot as usize;

    // Get the item from the slot
    let item = {
        let inv = match world.get::<&Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return,
        };
        match &inv.slots[slot_index] {
            Some(item) => item.clone(),
            None => return, // Nothing to drop
        }
    };

    let drop_count = if drop_stack { item.count } else { 1 };
    let remaining = item.count - drop_count;

    // Update inventory
    {
        let mut inv = match world.get::<&mut Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return,
        };
        if remaining > 0 {
            inv.set_slot(slot_index, Some(ItemStack::new(item.item_id, remaining)));
        } else {
            inv.set_slot(slot_index, None);
        }
    }

    // Send slot update to client
    let state_id = world
        .get::<&Inventory>(entity)
        .map(|inv| inv.state_id)
        .unwrap_or(1);
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let slot_item = if remaining > 0 {
            Some(ItemStack::new(item.item_id, remaining))
        } else {
            None
        };
        let _ = sender.0.send(InternalPacket::SetContainerSlot {
            window_id: 0,
            state_id,
            slot: slot_index as i16,
            item: slot_item,
        });
    }

    // Get player position and look direction for throw velocity
    let (pos, yaw, pitch) = {
        let p = world.get::<&Position>(entity).map(|p| p.0).unwrap_or(Vec3d::new(0.0, 0.0, 0.0));
        let r = world.get::<&Rotation>(entity).map(|r| (r.yaw, r.pitch)).unwrap_or((0.0, 0.0));
        (p, r.0, r.1)
    };

    // Spawn in front of player at eye height
    let yaw_rad = (yaw as f64).to_radians();
    let pitch_rad = (pitch as f64).to_radians();
    let spawn_x = pos.x - yaw_rad.sin() * 0.5;
    let spawn_y = pos.y + 1.3; // eye height minus a bit
    let spawn_z = pos.z + yaw_rad.cos() * 0.5;

    // Throw velocity in look direction
    let speed = 0.3;
    let vx = -yaw_rad.sin() * pitch_rad.cos() * speed;
    let vy = -pitch_rad.sin() * speed + 0.1;
    let vz = yaw_rad.cos() * pitch_rad.cos() * speed;

    let eid = next_eid.fetch_add(1, Ordering::Relaxed);
    let uuid = Uuid::new_v4();

    let drop_item = ItemStack::new(item.item_id, drop_count);
    let item_id = drop_item.item_id;
    let item_count = drop_item.count;

    world.spawn((
        EntityId(eid),
        EntityUuid(uuid),
        Position(Vec3d::new(spawn_x, spawn_y, spawn_z)),
        PreviousPosition(Vec3d::new(spawn_x, spawn_y, spawn_z)),
        Velocity(Vec3d::new(vx, vy, vz)),
        OnGround(false),
        ItemEntity {
            item: drop_item,
            pickup_delay: 40, // 2 second delay so player doesn't immediately pick it up
            age: 0,
        },
        Rotation { yaw: 0.0, pitch: 0.0 },
    ));

    scripting.fire_event_in_context(
        "entity_spawn",
        &[
            ("entity_id", &eid.to_string()),
            ("entity_type", "item"),
            ("x", &format!("{:.2}", spawn_x)),
            ("y", &format!("{:.2}", spawn_y)),
            ("z", &format!("{:.2}", spawn_z)),
            ("item_id", &item_id.to_string()),
            ("item_count", &item_count.to_string()),
        ],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );
}

/// Spawn a dropped item entity in the world.
pub(crate) fn spawn_item_entity(
    world: &mut World,
    world_state: &mut WorldState,
    next_eid: &Arc<AtomicI32>,
    x: f64,
    y: f64,
    z: f64,
    item: ItemStack,
    pickup_delay: u32,
    scripting: &ScriptRuntime,
) -> i32 {
    let eid = next_eid.fetch_add(1, Ordering::Relaxed);
    let uuid = Uuid::new_v4();

    let mut rng = rand::thread_rng();
    let vx = rng.gen_range(-0.1..0.1);
    let vy = 0.2;
    let vz = rng.gen_range(-0.1..0.1);

    let item_id = item.item_id;
    let item_count = item.count;

    world.spawn((
        EntityId(eid),
        EntityUuid(uuid),
        Position(Vec3d::new(x, y, z)),
        PreviousPosition(Vec3d::new(x, y, z)),
        Velocity(Vec3d::new(vx, vy, vz)),
        OnGround(false),
        ItemEntity {
            item,
            pickup_delay,
            age: 0,
        },
        Rotation { yaw: 0.0, pitch: 0.0 },
    ));

    // Fire entity_spawn event
    scripting.fire_event_in_context(
        "entity_spawn",
        &[
            ("entity_id", &eid.to_string()),
            ("entity_type", "item"),
            ("x", &format!("{:.2}", x)),
            ("y", &format!("{:.2}", y)),
            ("z", &format!("{:.2}", z)),
            ("item_id", &item_id.to_string()),
            ("item_count", &item_count.to_string()),
        ],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );

    eid
}

/// Apply physics to item entities: gravity, velocity, ground collision, despawn.
fn tick_item_physics(world: &mut World, world_state: &mut WorldState, scripting: &ScriptRuntime) {
    // Collect entities to despawn (age >= 6000)
    let mut to_despawn: Vec<(hecs::Entity, i32)> = Vec::new();

    // Apply physics
    for (e, (eid, pos, vel, og, item_ent)) in world
        .query::<(&EntityId, &mut Position, &mut Velocity, &mut OnGround, &mut ItemEntity)>()
        .iter()
    {
        item_ent.age += 1;
        if item_ent.age >= 6000 {
            to_despawn.push((e, eid.0));
            continue;
        }

        if item_ent.pickup_delay > 0 {
            item_ent.pickup_delay -= 1;
        }

        // Skip physics when resting on ground with negligible velocity
        if og.0
            && vel.0.y.abs() < 0.001
            && vel.0.x.abs() < 0.001
            && vel.0.z.abs() < 0.001
        {
            vel.0 = Vec3d::new(0.0, 0.0, 0.0);
            continue;
        }

        // Apply gravity
        vel.0.y -= 0.04;

        // Apply velocity
        pos.0.x += vel.0.x;
        pos.0.y += vel.0.y;
        pos.0.z += vel.0.z;

        // Ground collision check
        let check_pos = BlockPos::new(
            pos.0.x.floor() as i32,
            (pos.0.y - 0.01).floor() as i32,
            pos.0.z.floor() as i32,
        );
        let block_below = world_state.get_block(&check_pos);
        if block_below != 0 {
            let ground_y = check_pos.y as f64 + 1.0;
            if pos.0.y < ground_y + 0.25 {
                pos.0.y = ground_y + 0.25;
                vel.0.y = 0.0;
                og.0 = true;
            }
        } else {
            og.0 = false;
        }

        // Friction
        vel.0.x *= 0.98;
        vel.0.y *= 0.98;
        vel.0.z *= 0.98;
        if og.0 {
            vel.0.x *= 0.5;
            vel.0.z *= 0.5;
        }
    }

    // Despawn aged-out items
    for (entity, eid) in &to_despawn {
        // Broadcast removal
        broadcast_to_all(world, &InternalPacket::RemoveEntities {
            entity_ids: vec![*eid],
        });

        // Remove from all players' tracked entities
        for (_e, tracked) in world.query::<&mut TrackedEntities>().iter() {
            tracked.visible.remove(eid);
        }

        // Fire event
        scripting.fire_event_in_context(
            "entity_despawn",
            &[
                ("entity_id", &eid.to_string()),
                ("reason", "timeout"),
            ],
            world as *mut _ as *mut (),
            world_state as *mut _ as *mut (),
        );

        let _ = world.despawn(*entity);
    }
}

/// Check for item pickup by nearby players. Runs every 4 ticks.
fn tick_item_pickup(world: &mut World, world_state: &mut WorldState, scripting: &ScriptRuntime) {
    // Collect all pickable items
    let mut items: Vec<(hecs::Entity, i32, Vec3d, i32, i8)> = Vec::new();
    for (e, (eid, pos, item_ent)) in world
        .query::<(&EntityId, &Position, &ItemEntity)>()
        .iter()
    {
        if item_ent.pickup_delay == 0 {
            items.push((e, eid.0, pos.0, item_ent.item.item_id, item_ent.item.count));
        }
    }

    // Collect all players
    let mut players: Vec<(hecs::Entity, i32, Vec3d, String)> = Vec::new();
    for (e, (eid, pos, profile)) in world
        .query::<(&EntityId, &Position, &Profile)>()
        .iter()
    {
        players.push((e, eid.0, pos.0, profile.0.name.clone()));
    }

    let mut picked_up: Vec<(hecs::Entity, i32, i32, i8)> = Vec::new(); // (entity, item_eid, collector_eid, count)

    for &(item_entity, item_eid, item_pos, item_id, item_count) in &items {
        for &(player_entity, player_eid, player_pos, ref name) in &players {
            let dx = item_pos.x - player_pos.x;
            let dy = item_pos.y - player_pos.y;
            let dz = item_pos.z - player_pos.z;
            let dist_sq = dx * dx + dy * dy + dz * dz;

            if dist_sq < 1.5 * 1.5 {
                let item_name = pickaxe_data::item_id_to_name(item_id)
                    .unwrap_or("unknown")
                    .to_string();

                // Fire cancellable event
                let cancelled = scripting.fire_event_in_context(
                    "item_pickup",
                    &[
                        ("name", name),
                        ("item_id", &item_id.to_string()),
                        ("item_name", &item_name),
                        ("item_count", &item_count.to_string()),
                        ("entity_id", &item_eid.to_string()),
                    ],
                    world as *mut _ as *mut (),
                    world_state as *mut _ as *mut (),
                );

                if cancelled {
                    continue;
                }

                // Try to give item to player
                if give_item_to_player(world, player_entity, item_id, item_count) {
                    picked_up.push((item_entity, item_eid, player_eid, item_count));
                    break; // Item is picked up, move to next item
                }
            }
        }
    }

    // Despawn picked up items
    for &(entity, eid, collector_eid, count) in &picked_up {
        // Send pickup animation
        broadcast_to_all(world, &InternalPacket::TakeItemEntity {
            collected_entity_id: eid,
            collector_entity_id: collector_eid,
            item_count: count as i32,
        });

        // Play item pickup sound at collector position
        if let Ok(pos) = world.get::<&Position>(entity) {
            play_sound_at_entity(world, pos.0.x, pos.0.y, pos.0.z, "entity.item.pickup", SOUND_PLAYERS, 0.2, ((rand::random::<f32>() - 0.5) * 1.4 + 1.0));
        }

        broadcast_to_all(world, &InternalPacket::RemoveEntities {
            entity_ids: vec![eid],
        });

        for (_e, tracked) in world.query::<&mut TrackedEntities>().iter() {
            tracked.visible.remove(&eid);
        }

        scripting.fire_event_in_context(
            "entity_despawn",
            &[
                ("entity_id", &eid.to_string()),
                ("reason", "pickup"),
            ],
            world as *mut _ as *mut (),
            world_state as *mut _ as *mut (),
        );

        let _ = world.despawn(entity);
    }
}

/// Give an item to a player entity, returning true on success.
fn give_item_to_player(world: &mut World, entity: hecs::Entity, item_id: i32, count: i8) -> bool {
    let max_stack = pickaxe_data::item_id_to_stack_size(item_id).unwrap_or(64);
    let slot_index = {
        let inv = match world.get::<&Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return false,
        };
        match inv.find_slot_for_item(item_id, max_stack) {
            Some(i) => i,
            None => return false,
        }
    };

    let (item, state_id) = {
        let mut inv = match world.get::<&mut Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return false,
        };
        let new_item = match &inv.slots[slot_index] {
            Some(existing) => {
                let space = (max_stack as i8).saturating_sub(existing.count);
                let to_add = count.min(space);
                ItemStack::new(item_id, existing.count.saturating_add(to_add))
            }
            None => ItemStack::new(item_id, count.min(max_stack as i8)),
        };
        inv.set_slot(slot_index, Some(new_item.clone()));
        (new_item, inv.state_id)
    };

    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SetContainerSlot {
            window_id: 0,
            state_id,
            slot: slot_index as i16,
            item: Some(item),
        });
    }

    true
}

/// Tick all furnace block entities: consume fuel, smelt items, send progress to viewers.
fn tick_furnaces(world: &World, world_state: &mut WorldState) {
    let mut updates: Vec<(BlockPos, i16, i16, i16, i16)> = Vec::new();

    for (pos, block_entity) in world_state.block_entities.iter_mut() {
        let BlockEntity::Furnace {
            ref mut input, ref mut fuel, ref mut output,
            ref mut burn_time, ref mut burn_duration,
            ref mut cook_progress, ref mut cook_total,
        } = block_entity else { continue };

        let was_lit = *burn_time > 0;

        let smelt_result = input.as_ref().and_then(|i| pickaxe_data::smelting_result(i.item_id));
        let can_smelt = smelt_result.is_some();

        let output_accepts = if let Some((result_id, _)) = smelt_result {
            match output {
                None => true,
                Some(ref o) => o.item_id == result_id && (o.count as i32) < pickaxe_data::item_id_to_stack_size(result_id).unwrap_or(64),
            }
        } else { false };

        // Consume fuel if needed
        if *burn_time <= 0 && can_smelt && output_accepts {
            if let Some(ref mut f) = fuel {
                if let Some(ticks) = pickaxe_data::fuel_burn_time(f.item_id) {
                    *burn_time = ticks;
                    *burn_duration = ticks;
                    f.count -= 1;
                    if f.count <= 0 { *fuel = None; }
                }
            }
        }

        // Burn
        if *burn_time > 0 {
            *burn_time -= 1;

            if can_smelt && output_accepts {
                if let Some((_, ct)) = smelt_result {
                    *cook_total = ct;
                }
                *cook_progress += 1;
                if *cook_progress >= *cook_total {
                    *cook_progress = 0;
                    if let Some((result_id, _)) = smelt_result {
                        match output {
                            None => *output = Some(ItemStack::new(result_id, 1)),
                            Some(ref mut o) => o.count += 1,
                        }
                        if let Some(ref mut i) = input {
                            i.count -= 1;
                            if i.count <= 0 { *input = None; }
                        }
                    }
                }
            } else {
                *cook_progress = 0;
            }
        } else {
            *cook_progress = 0;
        }

        let is_lit = *burn_time > 0;
        if was_lit != is_lit || *cook_progress > 0 || was_lit {
            updates.push((*pos, *burn_time, *burn_duration, *cook_progress, *cook_total));
        }
    }

    // Send progress updates to players who have this furnace open
    for (pos, bt, bd, cp, ct) in &updates {
        for (_e, (sender, open)) in world.query::<(&ConnectionSender, &OpenContainer)>().iter() {
            if let Menu::Furnace { pos: fpos } = &open.menu {
                if fpos == pos {
                    let _ = sender.0.send(InternalPacket::SetContainerData { container_id: open.container_id, property: 0, value: *bt });
                    let _ = sender.0.send(InternalPacket::SetContainerData { container_id: open.container_id, property: 1, value: *bd });
                    let _ = sender.0.send(InternalPacket::SetContainerData { container_id: open.container_id, property: 2, value: *cp });
                    let _ = sender.0.send(InternalPacket::SetContainerData { container_id: open.container_id, property: 3, value: *ct });
                }
            }
        }
    }
}

/// Update destroy stage animation for all players currently breaking blocks.
fn tick_block_breaking(world: &mut World, tick_count: u64) {
    let mut updates: Vec<(i32, BlockPos, i8)> = Vec::new();
    for (_entity, (eid, breaking)) in world.query::<(&EntityId, &BreakingBlock)>().iter() {
        let elapsed = tick_count.saturating_sub(breaking.started_tick);
        let progress = elapsed as f64 / breaking.total_ticks as f64;
        let stage = (progress * 10.0).min(9.0) as i8;
        if stage != breaking.last_stage {
            updates.push((eid.0, breaking.position, stage));
        }
    }

    for (eid, pos, stage) in &updates {
        broadcast_to_all(
            world,
            &InternalPacket::SetBlockDestroyStage {
                entity_id: *eid,
                position: *pos,
                destroy_stage: *stage,
            },
        );
    }

    for (_entity, breaking) in world.query::<&mut BreakingBlock>().iter() {
        let elapsed = tick_count.saturating_sub(breaking.started_tick);
        let progress = elapsed as f64 / breaking.total_ticks as f64;
        breaking.last_stage = (progress * 10.0).min(9.0) as i8;
    }
}

// ── Command handlers ──────────────────────────────────────────────────

fn cmd_gamemode(world: &mut World, entity: hecs::Entity, args: &str) {
    if !is_op(world, entity) {
        send_message(world, entity, "You don't have permission to use this command.");
        return;
    }

    let mode = match args.trim() {
        "survival" | "s" | "0" => GameMode::Survival,
        "creative" | "c" | "1" => GameMode::Creative,
        "adventure" | "a" | "2" => GameMode::Adventure,
        "spectator" | "sp" | "3" => GameMode::Spectator,
        _ => {
            send_message(
                world,
                entity,
                "Usage: /gamemode <survival|creative|adventure|spectator>",
            );
            return;
        }
    };

    if let Ok(mut gm) = world.get::<&mut PlayerGameMode>(entity) {
        gm.0 = mode;
    }

    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::GameEvent {
            event: 3,
            value: mode.id() as f32,
        });
    }

    let uuid = world
        .get::<&Profile>(entity)
        .map(|p| p.0.uuid)
        .unwrap_or(uuid::Uuid::nil());
    broadcast_to_all(
        world,
        &InternalPacket::PlayerInfoUpdate {
            actions: player_info_actions::UPDATE_GAME_MODE,
            players: vec![PlayerInfoEntry {
                uuid,
                name: None,
                properties: vec![],
                game_mode: Some(mode.id() as i32),
                listed: None,
                ping: None,
                display_name: None,
            }],
        },
    );

    let name = world
        .get::<&Profile>(entity)
        .map(|p| p.0.name.clone())
        .unwrap_or_default();
    send_message(world, entity, &format!("Game mode set to {:?}", mode));
    info!("{} changed game mode to {:?}", name, mode);
}

fn cmd_tp(world: &mut World, entity: hecs::Entity, args: &str) {
    let parts: Vec<&str> = args.split_whitespace().collect();

    let (x, y, z) = match parts.len() {
        3 => {
            let x: f64 = match parts[0].parse() {
                Ok(v) => v,
                Err(_) => {
                    send_message(world, entity, "Invalid x coordinate");
                    return;
                }
            };
            let y: f64 = match parts[1].parse() {
                Ok(v) => v,
                Err(_) => {
                    send_message(world, entity, "Invalid y coordinate");
                    return;
                }
            };
            let z: f64 = match parts[2].parse() {
                Ok(v) => v,
                Err(_) => {
                    send_message(world, entity, "Invalid z coordinate");
                    return;
                }
            };
            (x, y, z)
        }
        1 => {
            let target_name = parts[0];
            let mut found = None;
            for (_e, (profile, pos)) in world.query::<(&Profile, &Position)>().iter() {
                if profile.0.name.eq_ignore_ascii_case(target_name) {
                    found = Some(pos.0);
                    break;
                }
            }
            match found {
                Some(pos) => (pos.x, pos.y, pos.z),
                None => {
                    send_message(
                        world,
                        entity,
                        &format!("Player '{}' not found", target_name),
                    );
                    return;
                }
            }
        }
        _ => {
            send_message(world, entity, "Usage: /tp <x> <y> <z> or /tp <player>");
            return;
        }
    };

    if let Ok(mut pos) = world.get::<&mut Position>(entity) {
        pos.0 = Vec3d::new(x, y, z);
    }

    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SynchronizePlayerPosition {
            position: Vec3d::new(x, y, z),
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
            teleport_id: 2,
        });
    }

    send_message(
        world,
        entity,
        &format!("Teleported to {:.1}, {:.1}, {:.1}", x, y, z),
    );
}

fn cmd_give(world: &mut World, entity: hecs::Entity, args: &str) {
    if !is_op(world, entity) {
        send_message(world, entity, "You don't have permission to use this command.");
        return;
    }

    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() {
        send_message(world, entity, "Usage: /give <item_name> [count]");
        return;
    }

    let item_name = parts[0].strip_prefix("minecraft:").unwrap_or(parts[0]);
    let count = if parts.len() > 1 {
        parts[1].parse::<i8>().unwrap_or(1).max(1)
    } else {
        1
    };

    let item_id = match pickaxe_data::item_name_to_id(item_name) {
        Some(id) => id,
        None => {
            send_message(world, entity, &format!("Unknown item: {}", item_name));
            return;
        }
    };

    let max_stack = pickaxe_data::item_id_to_stack_size(item_id).unwrap_or(64);
    let slot_index = {
        let inv = match world.get::<&Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return,
        };
        match inv.find_slot_for_item(item_id, max_stack) {
            Some(i) => i,
            None => {
                send_message(world, entity, "Inventory is full!");
                return;
            }
        }
    };

    let (item, state_id) = {
        let mut inv = match world.get::<&mut Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return,
        };
        let new_item = match &inv.slots[slot_index] {
            Some(existing) => {
                let space = (max_stack as i8).saturating_sub(existing.count);
                let to_add = count.min(space);
                pickaxe_types::ItemStack::new(item_id, existing.count.saturating_add(to_add))
            }
            None => pickaxe_types::ItemStack::new(item_id, count.min(max_stack as i8)),
        };
        inv.set_slot(slot_index, Some(new_item.clone()));
        (new_item, inv.state_id)
    };

    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SetContainerSlot {
            window_id: 0,
            state_id,
            slot: slot_index as i16,
            item: Some(item),
        });
    }

    let display_name = pickaxe_data::item_id_to_name(item_id).unwrap_or("unknown");
    send_message(
        world,
        entity,
        &format!("Gave {} x{}", display_name, count),
    );
}

fn cmd_kill(world: &mut World, world_state: &mut WorldState, entity: hecs::Entity, entity_id: i32, scripting: &ScriptRuntime) {
    // Set health to 0 and trigger death
    if let Ok(mut h) = world.get::<&mut Health>(entity) {
        h.current = 0.0;
    }
    // Send health update
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SetHealth {
            health: 0.0,
            food: 20,
            saturation: 5.0,
        });
    }
    handle_player_death(world, world_state, entity, entity_id, "kill", scripting);
}

fn cmd_say(world: &World, message: &str, sender_name: &str) {
    if message.is_empty() {
        return;
    }
    let chat_text = format!("[{}] {}", sender_name, message);
    broadcast_to_all(
        world,
        &InternalPacket::SystemChatMessage {
            content: TextComponent::plain(&chat_text),
            overlay: false,
        },
    );
}

fn cmd_help(world: &World, entity: hecs::Entity, lua_commands: &crate::bridge::LuaCommands) {
    let help_text = [
        "=== Pickaxe Server Commands ===",
        "/gamemode <mode> - Change game mode (survival/creative/adventure/spectator)",
        "/tp <x> <y> <z> - Teleport to coordinates",
        "/tp <player> - Teleport to player",
        "/give <item> [count] - Give item to yourself",
        "/kill - Respawn at spawn point",
        "/say <message> - Broadcast a message",
        "/time set <day|night|noon|midnight|value> - Set time of day",
        "/time add <value> - Add to time of day",
        "/time query [daytime|gametime|day] - Query current time",
        "/help - Show this help",
    ];
    for line in &help_text {
        send_message(world, entity, line);
    }
    if let Ok(cmds) = lua_commands.lock() {
        if !cmds.is_empty() {
            send_message(world, entity, "=== Mod Commands ===");
            for cmd in cmds.iter() {
                send_message(world, entity, &format!("/{}", cmd.name));
            }
        }
    }
}

fn cmd_time(world: &World, entity: hecs::Entity, args: &str, world_state: &mut WorldState) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() {
        send_message(world, entity, "Usage: /time <set|add|query> [value]");
        return;
    }

    match parts[0] {
        "set" => {
            if !is_op(world, entity) {
                send_message(world, entity, "You don't have permission to use this command.");
                return;
            }
            if parts.len() < 2 {
                send_message(world, entity, "Usage: /time set <day|night|noon|midnight|value>");
                return;
            }
            let time = match parts[1] {
                "day" | "sunrise" => 0,
                "noon" => 6000,
                "sunset" => 12000,
                "night" | "midnight" => 18000,
                other => match other.parse::<i64>() {
                    Ok(v) => v.rem_euclid(24000),
                    Err(_) => {
                        send_message(world, entity, "Invalid time value.");
                        return;
                    }
                },
            };
            world_state.time_of_day = time;
            broadcast_to_all(world, &InternalPacket::UpdateTime {
                world_age: world_state.world_age,
                time_of_day: world_state.time_of_day,
            });
            send_message(world, entity, &format!("Set time to {}", time));
        }
        "add" => {
            if !is_op(world, entity) {
                send_message(world, entity, "You don't have permission to use this command.");
                return;
            }
            if parts.len() < 2 {
                send_message(world, entity, "Usage: /time add <value>");
                return;
            }
            let amount = match parts[1].parse::<i64>() {
                Ok(v) => v,
                Err(_) => {
                    send_message(world, entity, "Invalid time value.");
                    return;
                }
            };
            world_state.time_of_day = (world_state.time_of_day + amount).rem_euclid(24000);
            broadcast_to_all(world, &InternalPacket::UpdateTime {
                world_age: world_state.world_age,
                time_of_day: world_state.time_of_day,
            });
            send_message(world, entity, &format!("Added {} to time (now {})", amount, world_state.time_of_day));
        }
        "query" => {
            if parts.len() < 2 {
                send_message(world, entity, &format!("Time of day: {}, World age: {}", world_state.time_of_day, world_state.world_age));
                return;
            }
            match parts[1] {
                "daytime" => send_message(world, entity, &format!("Time of day: {}", world_state.time_of_day)),
                "gametime" => send_message(world, entity, &format!("World age: {}", world_state.world_age)),
                "day" => send_message(world, entity, &format!("Day: {}", world_state.world_age / 24000)),
                _ => send_message(world, entity, "Usage: /time query <daytime|gametime|day>"),
            }
        }
        _ => {
            send_message(world, entity, "Usage: /time <set|add|query> [value]");
        }
    }
}

fn handle_chunk_updates(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
) {
    // Get current position and chunk state
    let (pos, old_cx, old_cz, vd) = {
        let Ok(pos) = world.get::<&Position>(entity) else {
            return;
        };
        let Ok(cp) = world.get::<&ChunkPosition>(entity) else {
            return;
        };
        let Ok(vd) = world.get::<&ViewDistance>(entity) else {
            return;
        };
        (pos.0, cp.chunk_x, cp.chunk_z, vd.0)
    };

    let new_cx = (pos.x.floor() as i32) >> 4;
    let new_cz = (pos.z.floor() as i32) >> 4;

    if new_cx == old_cx && new_cz == old_cz {
        return;
    }

    // Update chunk position component
    if let Ok(mut cp) = world.get::<&mut ChunkPosition>(entity) {
        cp.chunk_x = new_cx;
        cp.chunk_z = new_cz;
    }

    let Ok(sender) = world.get::<&ConnectionSender>(entity) else {
        return;
    };
    let sender = &sender.0;

    // Send Set Center Chunk
    let _ = sender.send(InternalPacket::SetCenterChunk {
        chunk_x: new_cx,
        chunk_z: new_cz,
    });

    // Unload old chunks
    for cx in (old_cx - vd)..=(old_cx + vd) {
        for cz in (old_cz - vd)..=(old_cz + vd) {
            if (cx - new_cx).abs() > vd || (cz - new_cz).abs() > vd {
                let _ = sender.send(InternalPacket::UnloadChunk {
                    chunk_x: cx,
                    chunk_z: cz,
                });
            }
        }
    }

    // Send new chunks in batch
    send_new_chunks(sender, world_state, old_cx, old_cz, new_cx, new_cz, vd);
}

fn send_chunks_around(
    sender: &mpsc::UnboundedSender<InternalPacket>,
    world_state: &mut WorldState,
    center_cx: i32,
    center_cz: i32,
    view_distance: i32,
) {
    // We need to send chunk batch start/finished via raw packets.
    // Since we're going through the packet channel now, we use a special mechanism.
    // The writer task will handle the batch framing when it sees the ChunkBatchStart/Finished markers.
    let _ = sender.send(InternalPacket::ChunkBatchStart);

    let mut count = 0i32;
    for cx in (center_cx - view_distance)..=(center_cx + view_distance) {
        for cz in (center_cz - view_distance)..=(center_cz + view_distance) {
            let chunk_packet = world_state.get_chunk_packet(cx, cz);
            let _ = sender.send(chunk_packet);
            count += 1;
        }
    }

    let _ = sender.send(InternalPacket::ChunkBatchFinished { batch_size: count });
}

fn send_new_chunks(
    sender: &mpsc::UnboundedSender<InternalPacket>,
    world_state: &mut WorldState,
    old_cx: i32,
    old_cz: i32,
    new_cx: i32,
    new_cz: i32,
    vd: i32,
) {
    let _ = sender.send(InternalPacket::ChunkBatchStart);

    let mut count = 0i32;
    for cx in (new_cx - vd)..=(new_cx + vd) {
        for cz in (new_cz - vd)..=(new_cz + vd) {
            if (cx - old_cx).abs() > vd || (cz - old_cz).abs() > vd {
                let chunk_packet = world_state.get_chunk_packet(cx, cz);
                let _ = sender.send(chunk_packet);
                count += 1;
            }
        }
    }

    let _ = sender.send(InternalPacket::ChunkBatchFinished { batch_size: count });
}

/// Send a packet to all players.
fn broadcast_to_all(world: &World, packet: &InternalPacket) {
    for (_e, sender) in world.query::<&ConnectionSender>().iter() {
        let _ = sender.0.send(packet.clone());
    }
}

/// Send a packet to all players except the one with the given entity ID.
fn broadcast_except(world: &World, except_eid: i32, packet: &InternalPacket) {
    for (_e, (eid, sender)) in world.query::<(&EntityId, &ConnectionSender)>().iter() {
        if eid.0 != except_eid {
            let _ = sender.0.send(packet.clone());
        }
    }
}

/// SoundSource enum ordinal values matching MC SoundSource.
const SOUND_BLOCKS: u8 = 4;
const SOUND_PLAYERS: u8 = 7;
const SOUND_NEUTRAL: u8 = 6;

/// Play a sound at a block position, broadcast to all players.
fn play_sound_at_block(world: &World, pos: &BlockPos, sound: &str, source: u8, volume: f32, pitch: f32) {
    let packet = InternalPacket::SoundEffect {
        sound_name: format!("minecraft:{}", sound),
        source,
        x: pos.x as f64 + 0.5,
        y: pos.y as f64 + 0.5,
        z: pos.z as f64 + 0.5,
        volume,
        pitch,
        seed: rand::random(),
    };
    broadcast_to_all(world, &packet);
}

/// Play a sound at an entity's position, broadcast to all players.
fn play_sound_at_entity(world: &World, x: f64, y: f64, z: f64, sound: &str, source: u8, volume: f32, pitch: f32) {
    let packet = InternalPacket::SoundEffect {
        sound_name: format!("minecraft:{}", sound),
        source,
        x,
        y,
        z,
        volume,
        pitch,
        seed: rand::random(),
    };
    broadcast_to_all(world, &packet);
}

/// XP needed to advance from the given level (MC formula).
fn xp_needed_for_level(level: i32) -> i32 {
    if level < 15 {
        7 + level * 2
    } else if level < 30 {
        37 + (level - 15) * 5
    } else {
        112 + (level - 30) * 9
    }
}

/// Award XP to a player entity and send the updated XP bar.
fn award_xp(world: &mut World, entity: hecs::Entity, amount: i32) {
    let (level, progress, total_xp) = {
        let mut xp = match world.get::<&mut ExperienceData>(entity) {
            Ok(xp) => xp,
            Err(_) => return,
        };
        xp.total_xp += amount;
        let mut remaining = amount;
        while remaining > 0 {
            let needed = xp_needed_for_level(xp.level);
            let current_xp = (xp.progress * needed as f32) as i32;
            let new_xp = current_xp + remaining;
            if new_xp >= needed {
                remaining = new_xp - needed;
                xp.level += 1;
                xp.progress = 0.0;
            } else {
                xp.progress = new_xp as f32 / needed as f32;
                remaining = 0;
            }
        }
        (xp.level, xp.progress, xp.total_xp)
    };

    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SetExperience {
            progress,
            level,
            total_xp,
        });
    }
}

/// Returns XP dropped when mining a block (by state ID). Random range for ores.
fn block_xp_drop(state_id: i32) -> i32 {
    let name = match pickaxe_data::block_state_to_name(state_id) {
        Some(n) => n,
        None => return 0,
    };
    match name {
        "diamond_ore" | "deepslate_diamond_ore" | "emerald_ore" | "deepslate_emerald_ore" => {
            rand::random::<i32>().abs() % 5 + 3 // 3-7
        }
        "lapis_ore" | "deepslate_lapis_ore" => {
            rand::random::<i32>().abs() % 4 + 2 // 2-5
        }
        "redstone_ore" | "deepslate_redstone_ore" => {
            rand::random::<i32>().abs() % 5 + 1 // 1-5
        }
        "coal_ore" | "deepslate_coal_ore" => {
            rand::random::<i32>().abs() % 3 // 0-2
        }
        "nether_gold_ore" => {
            rand::random::<i32>().abs() % 2 // 0-1
        }
        _ => 0,
    }
}

/// Offset a block position by the given face direction.
fn offset_by_face(pos: &BlockPos, face: u8) -> BlockPos {
    match face {
        0 => BlockPos::new(pos.x, pos.y - 1, pos.z),
        1 => BlockPos::new(pos.x, pos.y + 1, pos.z),
        2 => BlockPos::new(pos.x, pos.y, pos.z - 1),
        3 => BlockPos::new(pos.x, pos.y, pos.z + 1),
        4 => BlockPos::new(pos.x - 1, pos.y, pos.z),
        5 => BlockPos::new(pos.x + 1, pos.y, pos.z),
        _ => *pos,
    }
}

/// Build the Declare Commands packet with the full command tree.
fn build_command_tree(lua_commands: &crate::bridge::LuaCommands) -> InternalPacket {
    let mut nodes: Vec<CommandNode> = Vec::new();

    // Helper: create a literal node (type=1)
    let lit = |name: &str, executable: bool, children: Vec<i32>| -> CommandNode {
        CommandNode {
            flags: 0x01 | if executable { 0x04 } else { 0 },
            children,
            name: Some(name.to_string()),
            parser: None,
            parser_properties: None,
        }
    };

    // Node 0: Root
    // Children will be filled in after we know all top-level command indices.
    nodes.push(CommandNode {
        flags: 0x00,
        children: vec![],
        name: None,
        parser: None,
        parser_properties: None,
    });

    // Simple commands: literal + executable, no subcommands
    let simple_cmds = ["gamemode", "gm", "tp", "teleport", "give", "kill", "say", "help"];
    let mut root_children: Vec<i32> = Vec::new();
    for cmd in &simple_cmds {
        let idx = nodes.len() as i32;
        root_children.push(idx);
        nodes.push(lit(cmd, true, vec![]));
    }

    // /time command with subcommands
    // /time set <day|night|noon|midnight|sunset|sunrise|value>
    // /time add <value>
    // /time query <daytime|gametime|day>

    // time set options
    let set_opts = ["day", "night", "noon", "midnight", "sunset", "sunrise"];
    let mut set_children: Vec<i32> = Vec::new();
    for opt in &set_opts {
        let idx = nodes.len() as i32;
        set_children.push(idx);
        nodes.push(lit(opt, true, vec![]));
    }
    let set_idx = nodes.len() as i32;
    nodes.push(lit("set", false, set_children));

    // time add (executable — takes a number typed by user)
    let add_idx = nodes.len() as i32;
    nodes.push(lit("add", true, vec![]));

    // time query options
    let query_opts = ["daytime", "gametime", "day"];
    let mut query_children: Vec<i32> = Vec::new();
    for opt in &query_opts {
        let idx = nodes.len() as i32;
        query_children.push(idx);
        nodes.push(lit(opt, true, vec![]));
    }
    let query_idx = nodes.len() as i32;
    nodes.push(lit("query", true, query_children));

    let time_idx = nodes.len() as i32;
    root_children.push(time_idx);
    nodes.push(lit("time", false, vec![set_idx, add_idx, query_idx]));

    // Add Lua-registered commands
    if let Ok(cmds) = lua_commands.lock() {
        for cmd in cmds.iter() {
            let idx = nodes.len() as i32;
            root_children.push(idx);
            nodes.push(lit(&cmd.name, true, vec![]));
        }
    }

    // Patch root children
    nodes[0].children = root_children;

    InternalPacket::DeclareCommands {
        nodes,
        root_index: 0,
    }
}

/// Send a system chat message to a specific player entity.
fn send_message(world: &World, entity: hecs::Entity, message: &str) {
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SystemChatMessage {
            content: TextComponent::plain(message),
            overlay: false,
        });
    }
}

/// Check if a player is an operator.
/// Re-reads config/ops.toml so changes take effect without a restart.
fn is_op(world: &World, entity: hecs::Entity) -> bool {
    let name = match world.get::<&Profile>(entity) {
        Ok(p) => p.0.name.clone(),
        Err(_) => return false,
    };
    let ops = crate::config::load_ops();
    ops.iter().any(|op| op.eq_ignore_ascii_case(&name))
}

/// Get the player count.
pub fn player_count(world: &World) -> usize {
    world.query::<&Profile>().iter().count()
}

/// Convert degrees to MC protocol angle (256ths of a turn).
fn degrees_to_angle(degrees: f32) -> u8 {
    ((degrees / 360.0) * 256.0) as i32 as u8
}
