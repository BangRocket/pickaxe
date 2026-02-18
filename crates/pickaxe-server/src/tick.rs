use crate::config::ServerConfig;
use crate::ecs::*;
use bytes::BytesMut;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use hecs::World;
use pickaxe_nbt::{nbt_compound, nbt_list, NbtValue};
use pickaxe_protocol_core::{player_info_actions, CommandNode, InternalPacket, PlayerInfoEntry};
use pickaxe_protocol_v1_21::{build_item_metadata, build_sleeping_metadata, build_tnt_metadata, build_wake_metadata, V1_21Adapter};
use pickaxe_region::RegionStorage;
use pickaxe_scripting::ScriptRuntime;
use pickaxe_types::{BlockPos, GameMode, GameProfile, ItemStack, TextComponent, Vec3d};
use pickaxe_world::{generate_flat_chunk_at, Chunk};
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
    spawn_point: Option<(BlockPos, f32)>, // bed position + yaw
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
        BlockEntity::Sign { front_text, back_text, color, has_glowing_text, is_waxed } => {
            let make_text_nbt = |lines: &[String; 4], col: &str, glowing: bool| -> NbtValue {
                let messages: Vec<NbtValue> = lines.iter().map(|line| {
                    if line.is_empty() {
                        NbtValue::String("{\"text\":\"\"}".into())
                    } else {
                        NbtValue::String(format!("{{\"text\":\"{}\"}}", line.replace('\\', "\\\\").replace('"', "\\\"")))
                    }
                }).collect();
                nbt_compound! {
                    "messages" => NbtValue::List(messages),
                    "color" => NbtValue::String(col.to_string()),
                    "has_glowing_text" => NbtValue::Byte(if glowing { 1 } else { 0 })
                }
            };
            nbt_compound! {
                "id" => NbtValue::String("minecraft:sign".into()),
                "x" => NbtValue::Int(pos.x),
                "y" => NbtValue::Int(pos.y),
                "z" => NbtValue::Int(pos.z),
                "front_text" => make_text_nbt(front_text, color, *has_glowing_text),
                "back_text" => make_text_nbt(back_text, color, false),
                "is_waxed" => NbtValue::Byte(if *is_waxed { 1 } else { 0 })
            }
        }
        BlockEntity::BrewingStand { bottles, ingredient, fuel, brew_time, fuel_uses } => {
            let mut items = Vec::new();
            for (i, slot) in bottles.iter().enumerate() {
                if let Some(item) = slot {
                    let name = pickaxe_data::item_id_to_name(item.item_id).unwrap_or("air");
                    if item.damage != 0 {
                        if let Some(potion_name) = pickaxe_data::potion_index_to_name(item.damage) {
                            items.push(nbt_compound! {
                                "Slot" => NbtValue::Byte(i as i8),
                                "id" => NbtValue::String(format!("minecraft:{}", name)),
                                "Count" => NbtValue::Byte(item.count),
                                "tag" => nbt_compound! {
                                    "Potion" => NbtValue::String(format!("minecraft:{}", potion_name))
                                }
                            });
                        } else {
                            items.push(nbt_compound! {
                                "Slot" => NbtValue::Byte(i as i8),
                                "id" => NbtValue::String(format!("minecraft:{}", name)),
                                "Count" => NbtValue::Byte(item.count)
                            });
                        }
                    } else {
                        items.push(nbt_compound! {
                            "Slot" => NbtValue::Byte(i as i8),
                            "id" => NbtValue::String(format!("minecraft:{}", name)),
                            "Count" => NbtValue::Byte(item.count)
                        });
                    }
                }
            }
            if let Some(item) = ingredient {
                let name = pickaxe_data::item_id_to_name(item.item_id).unwrap_or("air");
                items.push(nbt_compound! {
                    "Slot" => NbtValue::Byte(3),
                    "id" => NbtValue::String(format!("minecraft:{}", name)),
                    "Count" => NbtValue::Byte(item.count)
                });
            }
            if let Some(item) = fuel {
                let name = pickaxe_data::item_id_to_name(item.item_id).unwrap_or("air");
                items.push(nbt_compound! {
                    "Slot" => NbtValue::Byte(4),
                    "id" => NbtValue::String(format!("minecraft:{}", name)),
                    "Count" => NbtValue::Byte(item.count)
                });
            }
            nbt_compound! {
                "id" => NbtValue::String("minecraft:brewing_stand".into()),
                "x" => NbtValue::Int(pos.x),
                "y" => NbtValue::Int(pos.y),
                "z" => NbtValue::Int(pos.z),
                "Items" => NbtValue::List(items),
                "BrewTime" => NbtValue::Short(*brew_time),
                "Fuel" => NbtValue::Byte(*fuel_uses as i8)
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
                        inventory[slot] = Some(ItemStack::new(item_id, count));
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
                    let stack = ItemStack::new(item_id, count);
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
        "brewing_stand" => {
            let mut bottles: [Option<ItemStack>; 3] = [None, None, None];
            let mut ingredient = None;
            let mut fuel = None;
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
                    let mut stack = ItemStack::new(item_id, count);
                    // Restore potion type from tag.Potion
                    if let Some(tag) = item_nbt.get("tag") {
                        if let Some(potion_str) = tag.get("Potion").and_then(|v| v.as_str()) {
                            let potion_name = potion_str.strip_prefix("minecraft:").unwrap_or(potion_str);
                            if let Some(idx) = pickaxe_data::potion_name_to_index(potion_name) {
                                stack.damage = idx;
                            }
                        }
                    }
                    match slot {
                        0 => bottles[0] = Some(stack),
                        1 => bottles[1] = Some(stack),
                        2 => bottles[2] = Some(stack),
                        3 => ingredient = Some(stack),
                        4 => fuel = Some(stack),
                        _ => {}
                    }
                }
            }
            let brew_time = nbt.get("BrewTime").and_then(|v| v.as_short()).unwrap_or(0);
            let fuel_uses = nbt.get("Fuel").and_then(|v| v.as_byte()).unwrap_or(0) as i16;
            Some((pos, BlockEntity::BrewingStand {
                bottles, ingredient, fuel,
                brew_time, fuel_uses,
            }))
        }
        "sign" => {
            let parse_text_side = |nbt: &NbtValue, key: &str| -> ([String; 4], String, bool) {
                let mut lines = [String::new(), String::new(), String::new(), String::new()];
                let mut color = "black".to_string();
                let mut glowing = false;
                if let Some(text_compound) = nbt.get(key) {
                    if let Some(messages) = text_compound.get("messages").and_then(|v| v.as_list()) {
                        for (i, msg) in messages.iter().enumerate().take(4) {
                            if let Some(json_str) = msg.as_str() {
                                // Parse simple {"text":"..."} format
                                if let Some(start) = json_str.find("\"text\":\"") {
                                    let rest = &json_str[start + 8..];
                                    if let Some(end) = rest.find('"') {
                                        lines[i] = rest[..end].replace("\\\"", "\"").replace("\\\\", "\\");
                                    }
                                }
                            }
                        }
                    }
                    if let Some(c) = text_compound.get("color").and_then(|v| v.as_str()) {
                        color = c.to_string();
                    }
                    if let Some(g) = text_compound.get("has_glowing_text").and_then(|v| v.as_byte()) {
                        glowing = g != 0;
                    }
                }
                (lines, color, glowing)
            };
            let (front_text, color, has_glowing_text) = parse_text_side(nbt, "front_text");
            let (back_text, _, _) = parse_text_side(nbt, "back_text");
            let is_waxed = nbt.get("is_waxed").and_then(|v| v.as_byte()).unwrap_or(0) != 0;
            Some((pos, BlockEntity::Sign {
                front_text, back_text, color, has_glowing_text, is_waxed,
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
    let spawn_point = world.get::<&SpawnPoint>(entity).ok();

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
            let mut entries: Vec<(String, NbtValue)> = vec![
                ("Slot".into(), NbtValue::Byte(nbt_slot)),
                ("id".into(), NbtValue::String(item_name)),
                ("count".into(), NbtValue::Byte(stack.count)),
            ];
            if stack.max_damage > 0 {
                entries.push(("MaxDamage".into(), NbtValue::Int(stack.max_damage)));
                if stack.damage > 0 {
                    entries.push(("Damage".into(), NbtValue::Int(stack.damage)));
                }
            }
            if !stack.enchantments.is_empty() {
                let ench_list: Vec<NbtValue> = stack.enchantments.iter().map(|(id, lvl)| {
                    let ench_name = format!("minecraft:{}", pickaxe_data::enchantment_id_to_name(*id).unwrap_or("unknown"));
                    NbtValue::Compound(vec![
                        ("id".into(), NbtValue::String(ench_name)),
                        ("lvl".into(), NbtValue::Short(*lvl as i16)),
                    ])
                }).collect();
                entries.push(("Enchantments".into(), NbtValue::List(ench_list)));
            }
            inv_items.push(NbtValue::Compound(entries));
        }
    }

    let mut nbt = nbt_compound! {
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

    // Add bed spawn point if set (vanilla format)
    if let Some(sp) = spawn_point {
        if let NbtValue::Compound(ref mut entries) = nbt {
            entries.push(("SpawnX".into(), NbtValue::Int(sp.position.x)));
            entries.push(("SpawnY".into(), NbtValue::Int(sp.position.y)));
            entries.push(("SpawnZ".into(), NbtValue::Int(sp.position.z)));
            entries.push(("SpawnAngle".into(), NbtValue::Float(sp.yaw)));
            entries.push(("SpawnDimension".into(), NbtValue::String("minecraft:overworld".into())));
        }
    }

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
                    let max_damage = entry.get("MaxDamage").and_then(|v| v.as_int()).unwrap_or(0);
                    let damage = entry.get("Damage").and_then(|v| v.as_int()).unwrap_or(0);
                    let mut stack = ItemStack::new(item_id, count);
                    stack.max_damage = max_damage;
                    stack.damage = damage;
                    // Load enchantments
                    if let Some(ench_list) = entry.get("Enchantments").and_then(|v| v.as_list()) {
                        for ench_nbt in ench_list {
                            let ench_id_str = ench_nbt.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let ench_name = ench_id_str.strip_prefix("minecraft:").unwrap_or(ench_id_str);
                            let lvl = ench_nbt.get("lvl").and_then(|v| v.as_short()).unwrap_or(1) as i32;
                            if let Some(eid) = pickaxe_data::enchantment_name_to_id(ench_name) {
                                stack.enchantments.push((eid, lvl));
                            }
                        }
                    }
                    slots[ecs_slot] = Some(stack);
                }
            }
        }
    }

    let xp_level = nbt.get("XpLevel").and_then(|v| v.as_int()).unwrap_or(0);
    let xp_progress = nbt.get("XpP").and_then(|v| v.as_float()).unwrap_or(0.0);
    let xp_total = nbt.get("XpTotal").and_then(|v| v.as_int()).unwrap_or(0);

    // Read bed spawn point (vanilla format: SpawnX, SpawnY, SpawnZ, SpawnAngle)
    let spawn_point = nbt.get("SpawnX").and_then(|v| v.as_int()).and_then(|sx| {
        let sy = nbt.get("SpawnY")?.as_int()?;
        let sz = nbt.get("SpawnZ")?.as_int()?;
        let angle = nbt.get("SpawnAngle").and_then(|v| v.as_float()).unwrap_or(0.0);
        Some((BlockPos::new(sx, sy, sz), angle))
    });

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
        spawn_point,
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
            "raining" => NbtValue::Byte(world_state.raining as i8),
            "thundering" => NbtValue::Byte(world_state.thundering as i8),
            "rainTime" => NbtValue::Int(world_state.rain_time),
            "thunderTime" => NbtValue::Int(world_state.thunder_time),
            "clearWeatherTime" => NbtValue::Int(world_state.clear_weather_time),
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

/// Deserialized level.dat data.
struct LevelDatData {
    world_age: i64,
    time_of_day: i64,
    raining: bool,
    thundering: bool,
    rain_time: i32,
    thunder_time: i32,
    clear_weather_time: i32,
}

/// Load world state from a gzip-compressed level.dat file.
fn load_level_dat(path: &std::path::Path) -> Option<LevelDatData> {
    let data = std::fs::read(path).ok()?;
    let mut decoder = GzDecoder::new(&data[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed).ok()?;

    let (_, nbt) = NbtValue::read_root_named(&decompressed).ok()?;
    let data_nbt = nbt.get("Data")?;
    let world_age = data_nbt.get("Time")?.as_long()?;
    let time_of_day = data_nbt.get("DayTime")?.as_long()?;
    let raining = data_nbt.get("raining").and_then(|v| v.as_byte()).unwrap_or(0) != 0;
    let thundering = data_nbt.get("thundering").and_then(|v| v.as_byte()).unwrap_or(0) != 0;
    let rain_time = data_nbt.get("rainTime").and_then(|v| v.as_int()).unwrap_or(0);
    let thunder_time = data_nbt.get("thunderTime").and_then(|v| v.as_int()).unwrap_or(0);
    let clear_weather_time = data_nbt.get("clearWeatherTime").and_then(|v| v.as_int()).unwrap_or(0);
    Some(LevelDatData {
        world_age,
        time_of_day,
        raining,
        thundering,
        rain_time,
        thunder_time,
        clear_weather_time,
    })
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
    BrewingStand {
        /// Slots 0-2: potion bottles (input/output)
        bottles: [Option<ItemStack>; 3],
        /// Slot 3: ingredient/reagent
        ingredient: Option<ItemStack>,
        /// Slot 4: fuel (blaze powder)
        fuel: Option<ItemStack>,
        /// Ticks remaining in current brew (0 = not brewing, counts down from 400)
        brew_time: i16,
        /// Fuel uses remaining (0-20, each blaze powder = 20)
        fuel_uses: i16,
    },
    Sign {
        /// 4 lines of text for the front side
        front_text: [String; 4],
        /// 4 lines of text for the back side
        back_text: [String; 4],
        /// Text color (default "black")
        color: String,
        /// Whether the text glows
        has_glowing_text: bool,
        /// Whether the sign is waxed (prevents editing)
        is_waxed: bool,
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
    // Weather state
    pub raining: bool,
    pub thundering: bool,
    pub rain_time: i32,      // ticks until rain toggles
    pub thunder_time: i32,   // ticks until thunder toggles
    pub clear_weather_time: i32,
    pub rain_level: f32,     // 0.0-1.0, gradual transition
    pub thunder_level: f32,  // 0.0-1.0, gradual transition
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
            raining: false,
            thundering: false,
            rain_time: 12000 + rand::random::<i32>().unsigned_abs() as i32 % 168000,
            thunder_time: 12000 + rand::random::<i32>().unsigned_abs() as i32 % 168000,
            clear_weather_time: 0,
            rain_level: 0.0,
            thunder_level: 0.0,
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
            // Generate with ore distribution based on chunk coordinates
            let chunk = generate_flat_chunk_at(pos.x, pos.z);
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

    /// Returns a block state only if the chunk is already loaded (no disk I/O).
    pub fn get_block_if_loaded(&self, pos: &BlockPos) -> Option<i32> {
        let chunk_pos = pos.chunk_pos();
        let local_x = (pos.x.rem_euclid(16)) as usize;
        let local_z = (pos.z.rem_euclid(16)) as usize;
        self.chunks.get(&chunk_pos).map(|c| c.get_block(local_x, pos.y, local_z))
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

    /// Unload chunks that are not within any player's view distance.
    /// Saves chunks to disk before removing them from memory.
    /// Also removes block entities belonging to unloaded chunks.
    pub fn unload_distant_chunks(&mut self, player_chunks: &[(i32, i32, i32)]) {
        // player_chunks: &[(chunk_x, chunk_z, view_distance)]
        let chunks_to_unload: Vec<ChunkPos> = self.chunks.keys()
            .filter(|pos| {
                !player_chunks.iter().any(|&(pcx, pcz, vd)| {
                    (pos.x - pcx).abs() <= vd && (pos.z - pcz).abs() <= vd
                })
            })
            .copied()
            .collect();

        if chunks_to_unload.is_empty() {
            return;
        }

        let count = chunks_to_unload.len();
        for pos in &chunks_to_unload {
            // Save before unloading
            self.queue_chunk_save(*pos);
            self.chunks.remove(pos);

            // Remove block entities in this chunk
            let chunk_min_x = pos.x * 16;
            let chunk_min_z = pos.z * 16;
            self.block_entities.retain(|be_pos, _| {
                !(be_pos.x >= chunk_min_x && be_pos.x < chunk_min_x + 16
                    && be_pos.z >= chunk_min_z && be_pos.z < chunk_min_z + 16)
            });
        }
        info!("Unloaded {} distant chunks ({} remain)", count, self.chunks.len());
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

    // Load level.dat if it exists (restores world_age, time_of_day, weather)
    let level_dat_path = PathBuf::from(&config.world_dir).join("level.dat");
    if let Some(level_data) = load_level_dat(&level_dat_path) {
        world_state.world_age = level_data.world_age;
        world_state.time_of_day = level_data.time_of_day;
        world_state.raining = level_data.raining;
        world_state.thundering = level_data.thundering;
        world_state.rain_time = level_data.rain_time;
        world_state.thunder_time = level_data.thunder_time;
        world_state.clear_weather_time = level_data.clear_weather_time;
        if level_data.raining {
            world_state.rain_level = 1.0;
        }
        if level_data.thundering {
            world_state.thunder_level = 1.0;
        }
        info!("Loaded level.dat: world_age={}, time_of_day={}, raining={}, thundering={}",
            level_data.world_age, level_data.time_of_day, level_data.raining, level_data.thundering);
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
        tick_shield_cooldown(&mut world);
        tick_void_damage(&mut world, &mut world_state, &scripting);
        tick_drowning_and_lava(&mut world, &mut world_state, &scripting);
        tick_health_hunger(&mut world, &mut world_state, &scripting, tick_count);
        tick_effects(&mut world, &mut world_state, &scripting, tick_count);
        tick_eating(&mut world);
        tick_sleeping(&mut world, &mut world_state, &scripting);
        tick_buttons(&mut world, &mut world_state);
        tick_item_physics(&mut world, &mut world_state, &scripting);
        tick_arrow_physics(&mut world, &mut world_state, &next_eid, &scripting);
        tick_fishing_bobbers(&mut world, &mut world_state);
        tick_tnt_entities(&mut world, &mut world_state, &next_eid, &scripting);
        if tick_count % 4 == 0 {
            tick_item_pickup(&mut world, &mut world_state, &scripting);
        }
        // Crop growth + farmland moisture (every 68 ticks ≈ 3.4s, simulating random ticks)
        if tick_count % 68 == 0 {
            tick_farming(&world, &mut world_state);
        }
        // Fire tick (every 35 ticks ≈ 1.75s, simulating MC's 30-40 tick random delay)
        if tick_count % 35 == 0 {
            tick_fire(&mut world, &mut world_state, &next_eid, &scripting);
        }
        // Fluid tick: water every 5 ticks, lava every 30 ticks
        if tick_count % 5 == 0 {
            tick_fluids(&world, &mut world_state, true, tick_count % 30 == 0);
        }
        tick_furnaces(&world, &mut world_state);
        tick_brewing_stands(&world, &mut world_state);
        tick_mob_ai(&mut world, &mut world_state, &scripting, &next_eid);
        tick_mob_spawning(&mut world, &world_state, &next_eid, tick_count);
        if tick_count % 100 == 0 {
            tick_mob_despawn(&mut world);
        }
        tick_entity_tracking(&mut world);
        tick_entity_movement_broadcast(&mut world);
        tick_world_time(&world, &mut world_state, tick_count);
        tick_weather_cycle(&world, &mut world_state, &scripting);
        tick_lightning(&mut world, &mut world_state, &next_eid, &scripting);
        tick_block_breaking(&mut world, tick_count);

        // Periodic player/world data save (every 60 seconds = 1200 ticks)
        if tick_count % 1200 == 0 && tick_count > 0 {
            save_all_players(&world, &world_state.save_tx);
            save_block_entity_chunks(&world_state);
            let level_data = serialize_level_dat(&world_state, &config);
            let _ = world_state.save_tx.send(SaveOp::LevelDat(level_data));

            // Unload chunks not in any player's view distance
            let player_chunks: Vec<(i32, i32, i32)> = world
                .query::<(&ChunkPosition, &ViewDistance)>()
                .iter()
                .map(|(_, (cp, vd))| (cp.chunk_x, cp.chunk_z, vd.0))
                .collect();
            world_state.unload_distant_chunks(&player_chunks);
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
    let spawn_pos = saved.as_ref().map(|s| s.position).unwrap_or(Vec3d::new(0.5, -49.0, 0.5));
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
    let player_spawn_point = saved.as_ref().and_then(|s| s.spawn_point);

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
    let (spawn_block_pos, spawn_angle) = if let Some((pos, angle)) = player_spawn_point {
        (pos, angle)
    } else {
        (BlockPos::new(0, -50, 0), 0.0)
    };
    let _ = sender.send(InternalPacket::SetDefaultSpawnPosition {
        position: spawn_block_pos,
        angle: spawn_angle,
    });

    // Send current weather state to new player
    if world_state.raining {
        let _ = sender.send(InternalPacket::GameEvent {
            event: 1, // START_RAINING
            value: 0.0,
        });
        let _ = sender.send(InternalPacket::GameEvent {
            event: 7, // RAIN_LEVEL_CHANGE
            value: world_state.rain_level,
        });
        let _ = sender.send(InternalPacket::GameEvent {
            event: 8, // THUNDER_LEVEL_CHANGE
            value: world_state.thunder_level,
        });
    }

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
        AirSupply::default(),
        ActiveEffects::new(),
    ));
    if let Some((pos, yaw)) = player_spawn_point {
        let _ = world.insert_one(player_entity, SpawnPoint { position: pos, yaw });
    }

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
        // Clean up sleeping state (unset bed occupied)
        if let Ok(sleeping) = world.get::<&SleepingState>(entity) {
            let bed_pos = sleeping.bed_pos;
            let head_block = world_state.get_block(&bed_pos);
            if pickaxe_data::is_bed(head_block) {
                let new_state = pickaxe_data::bed_set_occupied(head_block, false);
                world_state.set_block(&bed_pos, new_state);
                let facing = pickaxe_data::bed_facing(head_block);
                let (dx, dz) = pickaxe_data::bed_head_offset(facing);
                let foot_pos = BlockPos::new(bed_pos.x - dx, bed_pos.y, bed_pos.z - dz);
                let foot_block = world_state.get_block(&foot_pos);
                if pickaxe_data::is_bed(foot_block) {
                    let new_foot = pickaxe_data::bed_set_occupied(foot_block, false);
                    world_state.set_block(&foot_pos, new_foot);
                }
            }
        }

        // Clean up open container (crafting grid items are lost on disconnect)
        let _ = world.remove_one::<OpenContainer>(entity);

        // Despawn any active fishing bobber
        let bobber_to_remove: Option<(hecs::Entity, i32)> = {
            let mut found = None;
            for (e, (eid, bobber)) in world.query::<(&EntityId, &FishingBobber)>().iter() {
                if bobber.owner == entity {
                    found = Some((e, eid.0));
                    break;
                }
            }
            found
        };
        if let Some((bobber_entity, bobber_eid)) = bobber_to_remove {
            let _ = world.despawn(bobber_entity);
            broadcast_to_all(world, &InternalPacket::RemoveEntities {
                entity_ids: vec![bobber_eid],
            });
            for (_e, tracked) in world.query::<&mut TrackedEntities>().iter() {
                tracked.visible.remove(&bobber_eid);
            }
        }

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
            if x.is_nan() || y.is_nan() || z.is_nan() || x.is_infinite() || y.is_infinite() || z.is_infinite() {
                return; // Reject invalid position
            }
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
            if x.is_nan() || y.is_nan() || z.is_nan() || x.is_infinite() || y.is_infinite() || z.is_infinite()
                || yaw.is_nan() || pitch.is_nan() || yaw.is_infinite() || pitch.is_infinite() {
                return; // Reject invalid position
            }
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
            // Range validation: reject digs > 6 blocks away
            let player_pos = world.get::<&Position>(entity).map(|p| p.0).unwrap_or(Vec3d::new(0.0, 0.0, 0.0));
            {
                let dx = player_pos.x - (position.x as f64 + 0.5);
                let dy = player_pos.y - (position.y as f64 + 0.5);
                let dz = player_pos.z - (position.z as f64 + 0.5);
                if dx * dx + dy * dy + dz * dz > 6.0 * 6.0 {
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                    }
                    return;
                }
            }

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
                        let (held_item_id, efficiency_level) = {
                            let slot =
                                world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                            if let Ok(inv) = world.get::<&Inventory>(entity) {
                                if let Some(ref item) = inv.held_item(slot) {
                                    (Some(item.item_id), item.enchantment_level(20))
                                } else {
                                    (None, 0)
                                }
                            } else {
                                (None, 0)
                            }
                        };
                        match calculate_break_ticks(block_state, held_item_id, efficiency_level, block_overrides) {
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
                    // Accept if player has a BreakingBlock component (they started digging)
                    let valid = world.get::<&BreakingBlock>(entity).is_ok();

                    let _ = world.remove_one::<BreakingBlock>(entity);

                    if valid {
                        complete_block_break(
                            world, world_state, entity, entity_id, &position, sequence,
                            scripting, block_overrides, next_eid,
                        );
                    } else {
                        // No breaking component — just ack without breaking
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
                // Release Use Item (status 5) — fires bow arrow or stops shield block
                5 => {
                    // Check if player was blocking with shield — stop blocking
                    if world.get::<&BlockingState>(entity).is_ok() {
                        let _ = world.remove_one::<BlockingState>(entity);
                        // Clear entity metadata for blocking
                        broadcast_to_all(world, &InternalPacket::SetEntityMetadata {
                            entity_id,
                            metadata: vec![pickaxe_protocol_core::EntityMetadataEntry {
                                index: 8,
                                type_id: 0,
                                data: vec![0],
                            }],
                        });
                        return;
                    }

                    // Check if player is drawing a bow
                    let bow_draw = match world.get::<&BowDrawState>(entity) {
                        Ok(draw) => (draw.start_tick, draw.hand),
                        Err(_) => return,
                    };
                    let (start_tick, _hand) = bow_draw;
                    let _ = world.remove_one::<BowDrawState>(entity);

                    // Calculate draw power (MC formula)
                    let draw_ticks = world_state.tick_count.saturating_sub(start_tick) as f32;
                    let mut power = draw_ticks / 20.0;
                    power = (power * power + power * 2.0) / 3.0;
                    if power < 0.1 {
                        return; // too short a draw, don't fire
                    }
                    if power > 1.0 {
                        power = 1.0;
                    }
                    let is_critical = power >= 1.0;

                    // Get player position and look direction
                    let (px, py, pz, yaw, pitch) = {
                        let pos = match world.get::<&Position>(entity) {
                            Ok(p) => p.0,
                            Err(_) => return,
                        };
                        let rot = match world.get::<&Rotation>(entity) {
                            Ok(r) => (r.yaw, r.pitch),
                            Err(_) => (0.0, 0.0),
                        };
                        (pos.x, pos.y, pos.z, rot.0, rot.1)
                    };

                    // Calculate velocity from look direction (MC: power * 3.0)
                    let speed = power as f64 * 3.0;
                    let yaw_rad = (yaw as f64).to_radians();
                    let pitch_rad = (pitch as f64).to_radians();
                    let vx = -yaw_rad.sin() * pitch_rad.cos() * speed;
                    let vy = -pitch_rad.sin() * speed;
                    let vz = yaw_rad.cos() * pitch_rad.cos() * speed;

                    // Spawn arrow entity at eye height
                    let eye_y = py + 1.62;
                    spawn_arrow(
                        world, next_eid,
                        px, eye_y, pz,
                        vx, vy, vz,
                        2.0, // base arrow damage
                        Some(entity),
                        is_critical,
                        true, // from_player
                    );

                    // Consume one arrow from inventory
                    let arrow_id = pickaxe_data::item_name_to_id("arrow").unwrap_or(802);
                    if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                        for i in 0..46 {
                            if let Some(ref slot) = inv.slots[i] {
                                if slot.item_id == arrow_id {
                                    if slot.count <= 1 {
                                        inv.slots[i] = None;
                                    } else {
                                        inv.slots[i].as_mut().unwrap().count -= 1;
                                    }
                                    // Send slot update
                                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                        let _ = sender.0.send(InternalPacket::SetContainerSlot {
                                            window_id: 0,
                                            state_id: inv.state_id,
                                            slot: i as i16,
                                            item: inv.slots[i].clone(),
                                        });
                                    }
                                    break;
                                }
                            }
                        }
                    }

                    // Apply bow durability damage
                    let bow_id = pickaxe_data::item_name_to_id("bow").unwrap_or(801);
                    let held_slot_idx = {
                        let hs = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                        36 + hs as usize
                    };
                    if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                        if let Some(ref mut bow_item) = inv.slots[held_slot_idx] {
                            if bow_item.item_id == bow_id {
                                let max_dur = bow_item.max_damage;
                                let new_dur = bow_item.damage + 1;
                                if max_dur > 0 && new_dur >= max_dur {
                                    // Bow breaks
                                    inv.slots[held_slot_idx] = None;
                                    play_sound_at_entity(world, px, py, pz, "entity.item.break", SOUND_PLAYERS, 1.0, 1.0);
                                } else {
                                    bow_item.damage = new_dur;
                                }
                                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                    let _ = sender.0.send(InternalPacket::SetContainerSlot {
                                        window_id: 0,
                                        state_id: inv.state_id,
                                        slot: held_slot_idx as i16,
                                        item: inv.slots[held_slot_idx].clone(),
                                    });
                                }
                            }
                        }
                    }

                    // Play bow shoot sound
                    play_sound_at_entity(world, px, py, pz, "entity.arrow.shoot", SOUND_PLAYERS, 1.0, 1.0);
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
            let is_container = matches!(target_name, "chest" | "furnace" | "lit_furnace" | "crafting_table" | "brewing_stand" | "anvil" | "chipped_anvil" | "damaged_anvil");
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

            // Check if the target block is a sign — open editor on right-click
            if pickaxe_data::is_sign_state(target_block) && !sneaking {
                // Check if sign is waxed
                let is_waxed = world_state.get_block_entity(&position)
                    .and_then(|be| if let BlockEntity::Sign { is_waxed, .. } = be { Some(*is_waxed) } else { None })
                    .unwrap_or(false);

                if !is_waxed {
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::OpenSignEditor {
                            position,
                            is_front_text: true,
                        });
                        let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                    }
                    return;
                }
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

                        // Update redstone neighbors when lever/button is toggled
                        if target_name == "lever" || target_name.contains("button") {
                            update_redstone_neighbors(world, world_state, &position);
                        }

                        debug!("{} interacted with {} at {:?}", name, target_name, position);
                    }

                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                    }
                    return;
                }
            }

            // Check if the target block is a bed — try to sleep
            if pickaxe_data::is_bed(target_block) && !sneaking {
                try_sleep_in_bed(world, world_state, entity, entity_id, &position, target_block, scripting);
                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                    let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                }
                return;
            }

            // Check for flint_and_steel on TNT block — ignite it
            if target_name == "tnt" {
                let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                let held_item_id = world.get::<&Inventory>(entity)
                    .ok()
                    .and_then(|inv| inv.held_item(held_slot).as_ref().map(|i| i.item_id));
                let held_name = held_item_id.and_then(pickaxe_data::item_id_to_name).unwrap_or("");

                if held_name == "flint_and_steel" {
                    // Remove TNT block
                    world_state.set_block(&position, 0);
                    broadcast_to_all(world, &InternalPacket::BlockUpdate {
                        position,
                        block_id: 0,
                    });

                    // Spawn primed TNT entity
                    spawn_tnt_entity(
                        world, world_state, &next_eid,
                        position.x as f64 + 0.5,
                        position.y as f64,
                        position.z as f64 + 0.5,
                        80, // default fuse
                        Some(entity),
                        scripting,
                    );

                    // Damage flint_and_steel durability in survival
                    let game_mode = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
                    if game_mode != GameMode::Creative {
                        let slot_index = 36 + held_slot as usize;
                        if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                            if let Some(ref mut tool) = inv.slots[slot_index] {
                                tool.damage += 1;
                                if tool.max_damage > 0 && tool.damage >= tool.max_damage {
                                    inv.slots[slot_index] = None;
                                }
                            }
                            let state_id = inv.state_id;
                            let slot_item = inv.slots[slot_index].clone();
                            drop(inv);
                            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                let _ = sender.0.send(InternalPacket::SetContainerSlot {
                                    window_id: 0, state_id, slot: slot_index as i16, item: slot_item,
                                });
                            }
                        }
                    }

                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                    }
                    return;
                }
            }

            // Check for flint_and_steel fire placement (non-TNT blocks)
            {
                let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                let held_item_id = world.get::<&Inventory>(entity)
                    .ok()
                    .and_then(|inv| inv.held_item(held_slot).as_ref().map(|i| i.item_id));
                let held_name = held_item_id.and_then(pickaxe_data::item_id_to_name).unwrap_or("");

                if held_name == "flint_and_steel" && target_name != "tnt" {
                    // Place fire on the adjacent face
                    let fire_pos = offset_by_face(&position, face);
                    let fire_block = world_state.get_block(&fire_pos);
                    if fire_block == 0 {
                        // Check that the block below fire_pos is solid, or adjacent block is flammable
                        let below = BlockPos::new(fire_pos.x, fire_pos.y - 1, fire_pos.z);
                        let below_block = world_state.get_block(&below);
                        let below_name = pickaxe_data::block_state_to_name(below_block).unwrap_or("");
                        let has_support = below_block != 0 && !pickaxe_data::is_fire(below_block);

                        let has_adjacent_flammable = {
                            let offsets = [(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)];
                            offsets.iter().any(|(dx, dy, dz)| {
                                let adj = BlockPos::new(fire_pos.x + dx, fire_pos.y + dy, fire_pos.z + dz);
                                let adj_block = world_state.get_block(&adj);
                                let adj_name = pickaxe_data::block_state_to_name(adj_block).unwrap_or("");
                                pickaxe_data::is_flammable(adj_name)
                            })
                        };

                        if has_support || has_adjacent_flammable {
                            // Check for soul fire: fire on soul_sand or soul_soil
                            let fire_state = if below_name == "soul_sand" || below_name == "soul_soil" {
                                pickaxe_data::SOUL_FIRE_STATE
                            } else {
                                pickaxe_data::fire_default_state()
                            };

                            let player_name = world.get::<&Profile>(entity).map(|p| p.0.name.clone()).unwrap_or_default();
                            let cancelled = scripting.fire_event_in_context(
                                "block_place",
                                &[
                                    ("name", &player_name),
                                    ("x", &fire_pos.x.to_string()),
                                    ("y", &fire_pos.y.to_string()),
                                    ("z", &fire_pos.z.to_string()),
                                    ("block_id", &fire_state.to_string()),
                                ],
                                world as *mut _ as *mut (),
                                world_state as *mut _ as *mut (),
                            );
                            if !cancelled {
                                world_state.set_block(&fire_pos, fire_state);
                                broadcast_to_all(world, &InternalPacket::BlockUpdate {
                                    position: fire_pos,
                                    block_id: fire_state,
                                });
                                play_sound_at_block(world, &fire_pos, "item.flintandsteel.use", SOUND_PLAYERS, 1.0, 1.0);
                            }

                            // Damage flint_and_steel durability in survival
                            let game_mode = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
                            if game_mode != GameMode::Creative {
                                let slot_index = 36 + held_slot as usize;
                                if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                                    if let Some(ref mut tool) = inv.slots[slot_index] {
                                        tool.damage += 1;
                                        if tool.max_damage > 0 && tool.damage >= tool.max_damage {
                                            inv.slots[slot_index] = None;
                                        }
                                    }
                                    let state_id = inv.state_id;
                                    let slot_item = inv.slots[slot_index].clone();
                                    drop(inv);
                                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                        let _ = sender.0.send(InternalPacket::SetContainerSlot {
                                            window_id: 0, state_id, slot: slot_index as i16, item: slot_item,
                                        });
                                    }
                                }
                            }

                            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                            }
                            return;
                        }
                    }
                }
            }

            // Check for bucket interactions (water/lava placement and pickup)
            {
                let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                let held_item_id = world.get::<&Inventory>(entity)
                    .ok()
                    .and_then(|inv| inv.held_item(held_slot).as_ref().map(|i| i.item_id));
                let held_name = held_item_id.and_then(pickaxe_data::item_id_to_name).unwrap_or("");

                match held_name {
                    "water_bucket" | "lava_bucket" => {
                        // Place water/lava source at target face
                        let place_pos = offset_by_face(&position, face);
                        let place_block = world_state.get_block(&place_pos);
                        let place_name = pickaxe_data::block_state_to_name(place_block).unwrap_or("");

                        if place_block == 0 || pickaxe_data::is_fluid_destructible(place_name)
                            || pickaxe_data::is_fluid(place_block) {
                            let source_state = if held_name == "water_bucket" {
                                pickaxe_data::WATER_SOURCE
                            } else {
                                pickaxe_data::LAVA_SOURCE
                            };

                            world_state.set_block(&place_pos, source_state);
                            broadcast_to_all(world, &InternalPacket::BlockUpdate {
                                position: place_pos,
                                block_id: source_state,
                            });

                            let sound = if held_name == "water_bucket" {
                                "item.bucket.empty"
                            } else {
                                "item.bucket.empty_lava"
                            };
                            play_sound_at_block(world, &place_pos, sound, SOUND_PLAYERS, 1.0, 1.0);

                            // Replace held bucket with empty bucket (survival)
                            let game_mode = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
                            if game_mode != GameMode::Creative {
                                let slot_index = 36 + held_slot as usize;
                                if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                                    inv.set_slot(slot_index, Some(ItemStack::new(908, 1))); // empty bucket
                                    let state_id = inv.state_id;
                                    let slot_item = inv.slots[slot_index].clone();
                                    drop(inv);
                                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                        let _ = sender.0.send(InternalPacket::SetContainerSlot {
                                            window_id: 0, state_id, slot: slot_index as i16, item: slot_item,
                                        });
                                    }
                                }
                            }

                            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                            }
                            return;
                        }
                    }
                    "bucket" => {
                        // Pick up water/lava source with empty bucket
                        // Check the block at cursor position (not offset)
                        let pickup_pos = offset_by_face(&position, face);
                        let pickup_block = world_state.get_block(&pickup_pos);

                        if pickaxe_data::is_fluid_source(pickup_block) {
                            let filled_id = if pickaxe_data::is_water(pickup_block) { 909 } else { 910 };
                            let sound = if pickaxe_data::is_water(pickup_block) {
                                "item.bucket.fill"
                            } else {
                                "item.bucket.fill_lava"
                            };

                            // Remove the source block
                            world_state.set_block(&pickup_pos, 0);
                            broadcast_to_all(world, &InternalPacket::BlockUpdate {
                                position: pickup_pos,
                                block_id: 0,
                            });

                            play_sound_at_block(world, &pickup_pos, sound, SOUND_PLAYERS, 1.0, 1.0);

                            // Replace empty bucket with filled bucket
                            let game_mode = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
                            let slot_index = 36 + held_slot as usize;
                            if game_mode != GameMode::Creative {
                                if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                                    let held = inv.slots[slot_index].clone();
                                    // Buckets don't stack, so just replace the one bucket
                                    inv.set_slot(slot_index, Some(ItemStack::new(filled_id, 1)));
                                    // Send full inventory update
                                    let state_id = inv.state_id;
                                    let slot_item = inv.slots[slot_index].clone();
                                    drop(inv);
                                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                        let _ = sender.0.send(InternalPacket::SetContainerSlot {
                                            window_id: 0, state_id, slot: slot_index as i16, item: slot_item,
                                        });
                                    }
                                }
                            }

                            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                            }
                            return;
                        }
                    }
                    _ => {}
                }
            }

            // Check for farming interactions (hoe, seeds, bone meal)
            {
                let held_item_info = {
                    let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                    match world.get::<&Inventory>(entity) {
                        Ok(inv) => inv.held_item(held_slot).as_ref().map(|item| (item.item_id, item.count)),
                        Err(_) => None,
                    }
                };

                if let Some((item_id, _item_count)) = held_item_info {
                    let item_name = pickaxe_data::item_id_to_name(item_id).unwrap_or("");

                    // Hoe: till dirt/grass into farmland (must click top face)
                    if pickaxe_data::is_hoe(item_name) && face == 1 {
                        let target_name = pickaxe_data::block_state_to_name(target_block).unwrap_or("");
                        if pickaxe_data::is_hoeable(target_name) {
                            // Check air above
                            let above_pos = BlockPos::new(position.x, position.y + 1, position.z);
                            let above_block = world_state.get_block(&above_pos);
                            if above_block == 0 {
                                // Convert to farmland (moisture=0)
                                let farmland = pickaxe_data::farmland_state(0);
                                world_state.set_block(&position, farmland);
                                broadcast_to_all(world, &InternalPacket::BlockUpdate {
                                    position,
                                    block_id: farmland,
                                });
                                play_sound_at_block(world, &position, "item.hoe.till", SOUND_BLOCKS, 1.0, 1.0);

                                // Hoe durability damage (survival mode)
                                let game_mode = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
                                if game_mode != GameMode::Creative {
                                    let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                                    let slot_index = 36 + held_slot as usize;
                                    if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                                        if let Some(ref mut hoe_item) = inv.slots[slot_index] {
                                            hoe_item.damage += 1;
                                            if hoe_item.max_damage > 0 && hoe_item.damage >= hoe_item.max_damage {
                                                inv.slots[slot_index] = None;
                                            }
                                        }
                                        let state_id = inv.state_id;
                                        let slot_item = inv.slots[slot_index].clone();
                                        drop(inv);
                                        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                            let _ = sender.0.send(InternalPacket::SetContainerSlot {
                                                window_id: 0, state_id, slot: slot_index as i16, item: slot_item,
                                            });
                                        }
                                    }
                                }

                                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                    let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                                }
                                return;
                            }
                        }
                    }

                    // Seeds: plant on farmland (must click top face)
                    if let Some(crop_state) = pickaxe_data::seed_to_crop(item_name) {
                        if face == 1 && pickaxe_data::is_farmland(target_block) {
                            let plant_pos = BlockPos::new(position.x, position.y + 1, position.z);
                            let above_block = world_state.get_block(&plant_pos);
                            if above_block == 0 {
                                world_state.set_block(&plant_pos, crop_state);
                                broadcast_to_all(world, &InternalPacket::BlockUpdate {
                                    position: plant_pos,
                                    block_id: crop_state,
                                });
                                play_sound_at_block(world, &plant_pos, "item.crop.plant", SOUND_BLOCKS, 1.0, 1.0);

                                // Consume seed (survival mode)
                                let game_mode = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
                                if game_mode != GameMode::Creative {
                                    let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                                    let slot_index = 36 + held_slot as usize;
                                    if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                                        if let Some(ref item) = inv.slots[slot_index] {
                                            if item.count > 1 {
                                                let mut new_item = item.clone();
                                                new_item.count -= 1;
                                                inv.set_slot(slot_index, Some(new_item));
                                            } else {
                                                inv.set_slot(slot_index, None);
                                            }
                                            let state_id = inv.state_id;
                                            let slot_item = inv.slots[slot_index].clone();
                                            drop(inv);
                                            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                                let _ = sender.0.send(InternalPacket::SetContainerSlot {
                                                    window_id: 0, state_id, slot: slot_index as i16, item: slot_item,
                                                });
                                            }
                                        }
                                    }
                                }

                                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                    let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                                }
                                return;
                            }
                        }
                    }

                    // Bone meal: accelerate crop growth
                    let bone_meal_id = pickaxe_data::item_name_to_id("bone_meal").unwrap_or(960);
                    if item_id == bone_meal_id && pickaxe_data::is_crop(target_block) {
                        let (age, max_age) = pickaxe_data::crop_age(target_block).unwrap_or((0, 7));
                        if age < max_age {
                            let mut rng = rand::thread_rng();
                            let stages = rng.gen_range(2..=5);
                            if let Some(new_state) = pickaxe_data::crop_grow(target_block, stages) {
                                world_state.set_block(&position, new_state);
                                broadcast_to_all(world, &InternalPacket::BlockUpdate {
                                    position,
                                    block_id: new_state,
                                });

                                // Green particle effect (block event level 15 = bone meal)
                                // Send WorldEvent 1505 for bone meal particles
                                broadcast_to_all(world, &InternalPacket::WorldEvent {
                                    event: 1505,
                                    position,
                                    data: 0,
                                    disable_relative: false,
                                });

                                play_sound_at_block(world, &position, "item.bone_meal.use", SOUND_BLOCKS, 1.0, 1.0);

                                // Consume bone meal (survival mode)
                                let game_mode = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
                                if game_mode != GameMode::Creative {
                                    let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                                    let slot_index = 36 + held_slot as usize;
                                    if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                                        if let Some(ref item) = inv.slots[slot_index] {
                                            if item.count > 1 {
                                                let mut new_item = item.clone();
                                                new_item.count -= 1;
                                                inv.set_slot(slot_index, Some(new_item));
                                            } else {
                                                inv.set_slot(slot_index, None);
                                            }
                                            let state_id = inv.state_id;
                                            let slot_item = inv.slots[slot_index].clone();
                                            drop(inv);
                                            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                                let _ = sender.0.send(InternalPacket::SetContainerSlot {
                                                    window_id: 0, state_id, slot: slot_index as i16, item: slot_item,
                                                });
                                            }
                                        }
                                    }
                                }

                                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                    let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                                }
                                return;
                            }
                        }
                    }
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

            // Range validation: reject placements > 6 blocks away (vanilla limit)
            let player_pos = world.get::<&Position>(entity).map(|p| p.0).unwrap_or(Vec3d::new(0.0, 0.0, 0.0));
            let dx = player_pos.x - (target.x as f64 + 0.5);
            let dy = player_pos.y - (target.y as f64 + 0.5);
            let dz = player_pos.z - (target.z as f64 + 0.5);
            if dx * dx + dy * dy + dz * dz > 6.0 * 6.0 {
                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                    let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                }
                return;
            }

            // Special handling for bed placement (2-block structure)
            if pickaxe_data::is_bed(block_id) {
                let yaw = world.get::<&Rotation>(entity).map(|r| r.yaw).unwrap_or(0.0);
                let facing = pickaxe_data::yaw_to_facing(yaw);
                let (dx, dz) = pickaxe_data::bed_head_offset(facing);
                let head_pos = BlockPos::new(target.x + dx, target.y, target.z + dz);

                // Check if head position is clear
                let head_block = world_state.get_block(&head_pos);
                if head_block != 0 {
                    // Can't place bed — head position blocked
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                    }
                    return;
                }

                // Find the bed's min state from the block_id
                let bed_min = block_id - ((block_id - 1688) % 16);
                let foot_state = pickaxe_data::bed_state(bed_min, facing, false, false);
                let head_state = pickaxe_data::bed_state(bed_min, facing, false, true);

                world_state.set_block(&target, foot_state);
                world_state.set_block(&head_pos, head_state);

                broadcast_to_all(world, &InternalPacket::BlockUpdate { position: target, block_id: foot_state });
                broadcast_to_all(world, &InternalPacket::BlockUpdate { position: head_pos, block_id: head_state });

                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                    let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                }

                // Play placement sound
                play_sound_at_block(world, &target, "block.wood.place", SOUND_BLOCKS, 1.0, 0.8);

                // Consume item (survival mode)
                let game_mode = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
                if game_mode != GameMode::Creative {
                    let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                    let slot_index = 36 + held_slot as usize;
                    if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                        if let Some(ref item) = inv.slots[slot_index] {
                            if item.count > 1 {
                                let mut new_item = item.clone();
                                new_item.count -= 1;
                                inv.set_slot(slot_index, Some(new_item));
                            } else {
                                inv.set_slot(slot_index, None);
                            }
                            let state_id = inv.state_id;
                            let slot_item = inv.slots[slot_index].clone();
                            drop(inv);
                            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                let _ = sender.0.send(InternalPacket::SetContainerSlot {
                                    window_id: 0, state_id, slot: slot_index as i16, item: slot_item,
                                });
                            }
                        }
                    }
                }
                return;
            }

            // Special handling for sign placement
            {
                let held_item_name = {
                    let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                    match world.get::<&Inventory>(entity) {
                        Ok(inv) => inv.held_item(held_slot).as_ref().and_then(|item| {
                            pickaxe_data::item_id_to_name(item.item_id).map(|n| n.to_string())
                        }),
                        Err(_) => None,
                    }
                };
                if let Some(ref item_name) = held_item_name {
                    if let Some((standing_min, wall_min)) = pickaxe_data::sign_state_ids(item_name) {
                        // Determine if wall sign or standing sign based on placement face
                        // Face 2-5 (horizontal) = wall sign on that face
                        // Face 0 (bottom) or 1 (top) = standing sign with rotation from yaw
                        let (sign_state, is_wall) = if face >= 2 && face <= 5 {
                            (pickaxe_data::wall_sign_state(wall_min, face), true)
                        } else {
                            let yaw = world.get::<&Rotation>(entity).map(|r| r.yaw).unwrap_or(0.0);
                            (pickaxe_data::standing_sign_state(standing_min, yaw), false)
                        };
                        let _ = is_wall;

                        let player_name = world.get::<&Profile>(entity).map(|p| p.0.name.clone()).unwrap_or_default();
                        let cancelled = scripting.fire_event_in_context(
                            "block_place",
                            &[
                                ("name", &player_name),
                                ("x", &target.x.to_string()),
                                ("y", &target.y.to_string()),
                                ("z", &target.z.to_string()),
                                ("block_id", &sign_state.to_string()),
                            ],
                            world as *mut _ as *mut (),
                            world_state as *mut _ as *mut (),
                        );
                        if cancelled {
                            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                            }
                            return;
                        }

                        world_state.set_block(&target, sign_state);
                        world_state.set_block_entity(target, BlockEntity::Sign {
                            front_text: [String::new(), String::new(), String::new(), String::new()],
                            back_text: [String::new(), String::new(), String::new(), String::new()],
                            color: "black".to_string(),
                            has_glowing_text: false,
                            is_waxed: false,
                        });

                        broadcast_to_all(world, &InternalPacket::BlockUpdate {
                            position: target,
                            block_id: sign_state,
                        });

                        // Send OpenSignEditor to the placing player
                        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                            let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
                            let _ = sender.0.send(InternalPacket::OpenSignEditor {
                                position: target,
                                is_front_text: true,
                            });
                        }

                        // Play placement sound
                        play_sound_at_block(world, &target, "block.wood.place", SOUND_BLOCKS, 1.0, 0.8);

                        // Consume item (survival mode)
                        let game_mode = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
                        if game_mode != GameMode::Creative {
                            let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                            let slot_index = 36 + held_slot as usize;
                            if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                                if let Some(ref item) = inv.slots[slot_index] {
                                    if item.count > 1 {
                                        let mut new_item = item.clone();
                                        new_item.count -= 1;
                                        inv.set_slot(slot_index, Some(new_item));
                                    } else {
                                        inv.set_slot(slot_index, None);
                                    }
                                    let state_id = inv.state_id;
                                    let slot_item = inv.slots[slot_index].clone();
                                    drop(inv);
                                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                        let _ = sender.0.send(InternalPacket::SetContainerSlot {
                                            window_id: 0, state_id, slot: slot_index as i16, item: slot_item,
                                        });
                                    }
                                }
                            }
                        }
                        return;
                    }
                }
            }

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

            // Special handling for directional redstone components
            let block_id = {
                let block_name = pickaxe_data::block_state_to_name(block_id).unwrap_or("");
                if block_name == "repeater" {
                    // Repeater faces the player's look direction (north=0, south=1, west=2, east=3)
                    let yaw = world.get::<&Rotation>(entity).map(|r| r.yaw).unwrap_or(0.0);
                    let angle = ((yaw % 360.0) + 360.0) % 360.0;
                    // MC yaw: 0=south, 90=west, 180=north, 270=east
                    let facing = if angle >= 315.0 || angle < 45.0 { 1 }       // south (yaw ~0)
                        else if angle >= 45.0 && angle < 135.0 { 2 }           // west (yaw ~90)
                        else if angle >= 135.0 && angle < 225.0 { 0 }          // north (yaw ~180)
                        else { 3 };                                             // east (yaw ~270)
                    pickaxe_data::repeater_state(1, facing, false, false)
                } else if block_name == "redstone_torch" {
                    // Wall torch when placed on side of a block (face 2-5)
                    if face >= 2 && face <= 5 {
                        // face 2=north, 3=south, 4=west, 5=east
                        // Wall torch facing order: north=0, south=1, west=2, east=3
                        // State = 5740 + facing*2 + lit_offset (0=lit, 1=unlit)
                        let wall_facing = match face {
                            2 => 0, // north
                            3 => 1, // south
                            4 => 2, // west
                            5 => 3, // east
                            _ => 0,
                        };
                        5740 + wall_facing * 2 // lit=true (offset 0)
                    } else {
                        // Standing torch on top of block — default is already lit (5738)
                        5738
                    }
                } else if block_name == "redstone_lamp" {
                    // Redstone lamp should default to unlit when placed
                    pickaxe_data::redstone_lamp_set_lit(false)
                } else if block_name == "piston" || block_name == "sticky_piston" {
                    // Piston faces opposite to player's look direction
                    let yaw = world.get::<&Rotation>(entity).map(|r| r.yaw).unwrap_or(0.0);
                    let pitch = world.get::<&Rotation>(entity).map(|r| r.pitch).unwrap_or(0.0);
                    let facing6 = pickaxe_data::yaw_pitch_to_facing6(yaw, pitch);
                    pickaxe_data::piston_state(facing6, false, block_name == "sticky_piston")
                } else {
                    block_id
                }
            };

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
                "brewing_stand" => {
                    world_state.set_block_entity(target, BlockEntity::BrewingStand {
                        bottles: [None, None, None],
                        ingredient: None,
                        fuel: None,
                        brew_time: 0,
                        fuel_uses: 0,
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

            // Update redstone neighbors when a block is placed
            update_redstone_neighbors(world, world_state, &target);

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
                "effect" => cmd_effect(world, entity, args),
                "potion" => cmd_potion(world, entity, args),
                "enchant" => cmd_enchant(world, entity, args),
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
                // Cancel bow draw / shield block if switching slots
                let _ = world.remove_one::<BowDrawState>(entity);
                if world.remove_one::<BlockingState>(entity).is_ok() {
                    broadcast_to_all(world, &InternalPacket::SetEntityMetadata {
                        entity_id,
                        metadata: vec![pickaxe_protocol_core::EntityMetadataEntry {
                            index: 8,
                            type_id: 0,
                            data: vec![0],
                        }],
                    });
                }
                // Broadcast mainhand equipment change
                send_equipment_update(world, entity, entity_id);
            }
        }

        InternalPacket::CreativeInventoryAction { slot, item } => {
            if slot >= 0 {
                if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                    inv.set_slot(slot as usize, item);
                }
                // Broadcast equipment if armor or held item changed
                let held = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                let affected_slot = slot as usize;
                if (5..=8).contains(&affected_slot) || affected_slot == 45 || affected_slot == 36 + held as usize {
                    send_equipment_update(world, entity, entity_id);
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
                2 => {
                    // STOP_SLEEPING — player clicked "Leave Bed"
                    wake_player(world, world_state, entity, entity_id);
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
            // Broadcast equipment if armor/held slots may have changed
            send_equipment_update(world, entity, entity_id);
        }

        InternalPacket::RenameItem { ref name } => {
            handle_anvil_rename(world, entity, name);
        }

        InternalPacket::SignUpdate { position, is_front_text, ref lines } => {
            // Update the sign block entity with the text from the client
            if let Some(be) = world_state.get_block_entity_mut(&position) {
                if let BlockEntity::Sign { ref mut front_text, ref mut back_text, .. } = be {
                    if is_front_text {
                        for (i, line) in lines.iter().enumerate().take(4) {
                            front_text[i] = line.clone();
                        }
                    } else {
                        for (i, line) in lines.iter().enumerate().take(4) {
                            back_text[i] = line.clone();
                        }
                    }
                }
            }

            // Broadcast BlockEntityData to all players so they see the text
            if let Some(be) = world_state.get_block_entity(&position) {
                if matches!(be, BlockEntity::Sign { .. }) {
                    let nbt = build_sign_update_nbt(be);
                    broadcast_to_all(world, &InternalPacket::BlockEntityData {
                        position,
                        block_entity_type: 7, // sign
                        nbt,
                    });
                }
            }

            // Save the chunk containing this sign
            let chunk_pos = position.chunk_pos();
            world_state.queue_chunk_save(chunk_pos);

            let player_name = world.get::<&Profile>(entity).map(|p| p.0.name.clone()).unwrap_or_default();
            debug!("{} updated sign at {:?}", player_name, position);
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

            // Check if item is a shield
            let shield_id = pickaxe_data::item_name_to_id("shield").unwrap_or(1162);
            if item_id == shield_id {
                // Check for shield cooldown
                let on_cooldown = world.get::<&ShieldCooldown>(entity).is_ok();
                if !on_cooldown {
                    let _ = world.insert_one(entity, BlockingState {
                        start_tick: world_state.tick_count,
                        hand,
                    });
                    // Send entity metadata: living entity flags bit 0 (using item) + bit 1 if offhand
                    let flags: u8 = if hand == 1 { 0x03 } else { 0x01 };
                    broadcast_to_all(world, &InternalPacket::SetEntityMetadata {
                        entity_id,
                        metadata: vec![pickaxe_protocol_core::EntityMetadataEntry {
                            index: 8, // LivingEntity hand states (byte)
                            type_id: 0, // byte type
                            data: vec![flags],
                        }],
                    });
                }
                return;
            }

            // Check if item is a fishing rod
            let fishing_rod_id = pickaxe_data::item_name_to_id("fishing_rod").unwrap_or(931);
            if item_id == fishing_rod_id {
                // Check if player already has a bobber out — if so, retract it
                let existing_bobber = {
                    let mut found = None;
                    for (e, (eid, bobber)) in world.query::<(&EntityId, &FishingBobber)>().iter() {
                        if bobber.owner == entity {
                            found = Some((e, eid.0, bobber.state, bobber.nibble));
                            break;
                        }
                    }
                    found
                };

                if let Some((bobber_entity, bobber_eid, bobber_state, nibble)) = existing_bobber {
                    // Retract bobber — check if fish is biting
                    let mut rod_damage = 0i32;
                    if nibble > 0 && bobber_state == FishingBobberState::Bobbing {
                        // Fish caught! Spawn loot item
                        let bobber_pos = world.get::<&Position>(bobber_entity).ok().map(|p| p.0);
                        if let Some(bpos) = bobber_pos {
                            let player_pos = world.get::<&Position>(entity).ok().map(|p| p.0);
                            if let Some(_ppos) = player_pos {
                                let roll: f64 = rand::random();
                                let (loot_name, loot_count) = pickaxe_data::fishing_loot(roll);
                                if let Some(loot_id) = pickaxe_data::item_name_to_id(loot_name) {
                                    // Spawn item entity at bobber position
                                    let item = ItemStack::new(loot_id, loot_count as i8);
                                    spawn_item_entity(
                                        world, world_state, next_eid,
                                        bpos.x, bpos.y + 0.5, bpos.z,
                                        item, 10, scripting,
                                    );

                                    // Award XP (1-6) directly to fishing player
                                    let xp_amount = {
                                        let mut rng = rand::thread_rng();
                                        rng.gen_range(1..=6)
                                    };
                                    award_xp(world, entity, xp_amount);

                                    // Play splash sound
                                    play_sound_at_entity(world, bpos.x, bpos.y, bpos.z, "entity.fishing_bobber.splash", SOUND_PLAYERS, 1.0, 1.0);

                                    rod_damage = 1; // fish catch = 1 durability

                                    // Fire fishing_catch event
                                    let player_name = world.get::<&Profile>(entity).ok().map(|p| p.0.name.clone()).unwrap_or_default();
                                    let count_str = loot_count.to_string();
                                    let _ = scripting.fire_event_in_context(
                                        "fishing_catch",
                                        &[("name", &player_name), ("item_name", loot_name), ("item_count", &count_str)],
                                        world as *mut _ as *mut (),
                                        world_state as *mut _ as *mut (),
                                    );
                                }
                            }
                        }
                    }

                    // Despawn bobber
                    let _ = world.despawn(bobber_entity);
                    broadcast_to_all(world, &InternalPacket::RemoveEntities {
                        entity_ids: vec![bobber_eid],
                    });
                    // Remove from tracked entities
                    for (_e, tracked) in world.query::<&mut TrackedEntities>().iter() {
                        tracked.visible.remove(&bobber_eid);
                    }

                    // Play retrieve sound
                    if let Ok(pos) = world.get::<&Position>(entity) {
                        play_sound_at_entity(world, pos.0.x, pos.0.y, pos.0.z, "entity.fishing_bobber.retrieve", SOUND_PLAYERS, 1.0, 1.0);
                    }

                    // Apply rod durability damage
                    if rod_damage > 0 {
                        let held_slot_idx = {
                            let hs = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                            if hand == 1 { 45 } else { 36 + hs as usize }
                        };
                        if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                            if let Some(ref mut rod_item) = inv.slots[held_slot_idx] {
                                rod_item.damage += rod_damage;
                                if rod_item.max_damage > 0 && rod_item.damage >= rod_item.max_damage {
                                    inv.slots[held_slot_idx] = None;
                                    // Play break sound
                                    if let Ok(pos) = world.get::<&Position>(entity) {
                                        play_sound_at_entity(world, pos.0.x, pos.0.y, pos.0.z, "entity.item.break", SOUND_PLAYERS, 1.0, 1.0);
                                    }
                                }
                            }
                            let state_id = inv.state_id;
                            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                                let _ = sender.0.send(InternalPacket::SetContainerSlot {
                                    window_id: 0,
                                    state_id,
                                    slot: held_slot_idx as i16,
                                    item: inv.slots[held_slot_idx].clone(),
                                });
                            }
                        }
                    }
                } else {
                    // Cast bobber — spawn fishing hook entity
                    let (px, py, pz, yaw, pitch) = {
                        let pos = match world.get::<&Position>(entity) {
                            Ok(p) => p.0,
                            Err(_) => return,
                        };
                        let rot = match world.get::<&Rotation>(entity) {
                            Ok(r) => (r.yaw, r.pitch),
                            Err(_) => (0.0, 0.0),
                        };
                        (pos.x, pos.y, pos.z, rot.0, rot.1)
                    };

                    // Calculate velocity from look direction (MC: speed ~0.6 at 1 block distance)
                    let yaw_rad = (yaw as f64).to_radians();
                    let pitch_rad = (pitch as f64).to_radians();
                    let speed = 1.5;
                    let vx = -yaw_rad.sin() * pitch_rad.cos() * speed;
                    let vy = -pitch_rad.sin() * speed;
                    let vz = yaw_rad.cos() * pitch_rad.cos() * speed;

                    // Spawn at eye height with slight offset toward look direction
                    let eye_y = py + 1.62;
                    let offset = 0.3;
                    let sx = px - yaw_rad.sin() * offset;
                    let sz = pz + yaw_rad.cos() * offset;

                    spawn_fishing_bobber(world, next_eid, entity, entity_id, sx, eye_y, sz, vx, vy, vz);

                    // Play throw sound
                    play_sound_at_entity(world, px, py, pz, "entity.fishing_bobber.throw", SOUND_PLAYERS, 0.5, 1.0);
                }
                return;
            }

            // Check if item is a bow
            let bow_id = pickaxe_data::item_name_to_id("bow").unwrap_or(801);
            if item_id == bow_id {
                // Check player has arrows in inventory
                let arrow_id = pickaxe_data::item_name_to_id("arrow").unwrap_or(802);
                let has_arrows = {
                    let inv = match world.get::<&Inventory>(entity) {
                        Ok(inv) => inv,
                        Err(_) => return,
                    };
                    inv.slots.iter().any(|s| s.as_ref().is_some_and(|i| i.item_id == arrow_id))
                };
                if has_arrows {
                    let _ = world.insert_one(entity, BowDrawState {
                        start_tick: world_state.tick_count,
                        hand,
                    });
                }
                return;
            }

            // Check if the item is a drinkable potion
            if pickaxe_data::is_potion(item_id) {
                // Potions always drinkable, use 32-tick drink time
                // Store potion type index in nutrition field (repurposed)
                // and use saturation_modifier = -1.0 as a marker that this is a potion
                let potion_index = {
                    let inv = world.get::<&Inventory>(entity).ok();
                    let held = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                    let slot_idx = if hand == 1 { 45 } else { 36 + held as usize };
                    inv.and_then(|inv| inv.slots[slot_idx].as_ref().map(|i| i.damage))
                        .unwrap_or(0)
                };
                let _ = world.insert_one(entity, EatingState {
                    remaining_ticks: 32,
                    hand,
                    item_id,
                    nutrition: potion_index, // repurposed: potion type index
                    saturation_modifier: -1.0, // marker: this is a potion, not food
                });
                return;
            }

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
                handle_attack(world, world_state, entity, entity_id, target_eid, scripting, next_eid);
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
        "brewing_stand" => (11, "Brewing Stand", Menu::BrewingStand { pos: *pos }),
        "crafting_table" => (12, "Crafting", Menu::CraftingTable {
            grid: std::array::from_fn(|_| None),
            result: None,
        }),
        "anvil" | "chipped_anvil" | "damaged_anvil" => (8, "Repair & Name", Menu::Anvil {
            pos: *pos,
            input: None,
            sacrifice: None,
            result: None,
            rename: None,
            repair_cost: 0,
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
        // For brewing stands, send current brew time and fuel
        if block_name == "brewing_stand" {
            if let Some(BlockEntity::BrewingStand { brew_time, fuel_uses, .. }) = world_state.get_block_entity(pos) {
                let _ = sender.0.send(InternalPacket::SetContainerData { container_id, property: 0, value: *brew_time });
                let _ = sender.0.send(InternalPacket::SetContainerData { container_id, property: 1, value: *fuel_uses });
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
        Menu::BrewingStand { pos } => {
            // Slots: 0-2=bottles, 3=ingredient, 4=fuel, 5-31=player inv, 32-40=hotbar
            let mut slots = Vec::with_capacity(41);
            if let Some(BlockEntity::BrewingStand { bottles, ingredient, fuel, .. }) = world_state.get_block_entity(pos) {
                for b in bottles { slots.push(b.clone()); }
                slots.push(ingredient.clone());
                slots.push(fuel.clone());
            } else {
                slots.resize(5, None);
            }
            if let Some(inv) = &player_inv {
                for i in 9..36 { slots.push(inv.slots[i].clone()); }
                for i in 36..45 { slots.push(inv.slots[i].clone()); }
            } else {
                slots.resize(41, None);
            }
            slots
        }
        Menu::Anvil { input, sacrifice, result, .. } => {
            // Slots: 0=input, 1=sacrifice, 2=result, 3-29=player inv, 30-38=hotbar
            let mut slots = Vec::with_capacity(39);
            slots.push(input.clone());
            slots.push(sacrifice.clone());
            slots.push(result.clone());
            if let Some(inv) = &player_inv {
                for i in 9..36 { slots.push(inv.slots[i].clone()); }
                for i in 36..45 { slots.push(inv.slots[i].clone()); }
            } else {
                slots.resize(39, None);
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
        Menu::BrewingStand { .. } => "brewing_stand",
        Menu::Anvil { .. } => "anvil",
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

    // Drop anvil input/sacrifice items back to the player
    if let Menu::Anvil { input, sacrifice, .. } = &open.menu {
        let pos = world.get::<&Position>(entity).map(|p| p.0).unwrap_or(Vec3d::new(0.0, 64.0, 0.0));
        if let Some(item) = input {
            spawn_item_entity(world, world_state, next_eid,
                pos.x, pos.y + 1.0, pos.z,
                item.clone(), 0, scripting);
        }
        if let Some(item) = sacrifice {
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
        Menu::BrewingStand { .. } => {
            // 0-4=container, 5-31=player inv (9-35), 32-40=hotbar (36-44)
            if s < 5 { Some(SlotTarget::Container(s)) }
            else if s < 32 { Some(SlotTarget::PlayerInventory(s - 5 + 9)) }
            else if s < 41 { Some(SlotTarget::PlayerInventory(s - 32 + 36)) }
            else { None }
        }
        Menu::Anvil { .. } => {
            // 0=input, 1=sacrifice, 2=result, 3-29=player inv (9-35), 30-38=hotbar (36-44)
            if s == 2 { Some(SlotTarget::CraftResult) }
            else if s < 2 { Some(SlotTarget::Container(s)) }
            else if s < 30 { Some(SlotTarget::PlayerInventory(s - 3 + 9)) }
            else if s < 39 { Some(SlotTarget::PlayerInventory(s - 30 + 36)) }
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
                Menu::BrewingStand { pos } => {
                    if let Some(BlockEntity::BrewingStand { ref mut bottles, ref mut ingredient, ref mut fuel, .. }) = world_state.get_block_entity_mut(pos) {
                        match idx {
                            0..=2 => bottles[*idx] = item,
                            3 => *ingredient = item,
                            4 => *fuel = item,
                            _ => {}
                        }
                    }
                }
                Menu::Anvil { ref mut input, ref mut sacrifice, .. } => {
                    match idx {
                        0 => *input = item,
                        1 => *sacrifice = item,
                        _ => {}
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
        0 | 1 | 2 | 3 | 4 | 5 | 6 => {
            for (changed_slot, changed_item) in changed_slots {
                if let Some(t) = map_slot(&open.menu, *changed_slot) {
                    set_container_slot(world_state, world, entity, &mut open.menu, &t, changed_item.clone());
                }
            }
            // Handle crafting/anvil result take
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
                    handle_anvil_result_take(world, world_state, entity, &mut open.menu);
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
            // Recalculate anvil result when input or sacrifice changes
            if matches!(&open.menu, Menu::Anvil { .. }) {
                calculate_anvil_result(&mut open.menu);
                if let Menu::Anvil { repair_cost, .. } = &open.menu {
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::SetContainerData {
                            container_id: open.container_id,
                            property: 0,
                            value: *repair_cost as i16,
                        });
                    }
                }
            }
        }
        _ => {} // Unknown modes — resync below
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
            return Some(make_crafted_item(recipe.result_id, recipe.result_count));
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
            return Some(make_crafted_item(recipe.result_id, recipe.result_count));
        }
    }

    None
}

/// Create an item with proper durability set for tools/armor.
fn make_crafted_item(item_id: i32, count: i8) -> ItemStack {
    let name = pickaxe_data::item_id_to_name(item_id).unwrap_or("");
    let max_durability = pickaxe_data::item_max_durability(name);
    if max_durability > 0 {
        ItemStack::with_durability(item_id, count, max_durability)
    } else {
        ItemStack::new(item_id, count)
    }
}

/// Calculate the anvil result and repair cost from current inputs.
fn calculate_anvil_result(menu: &mut Menu) {
    let (input, sacrifice, result, repair_cost, rename) = match menu {
        Menu::Anvil { ref input, ref sacrifice, ref mut result, ref mut repair_cost, ref rename, .. } => {
            (input.clone(), sacrifice.clone(), result, repair_cost, rename.clone())
        }
        _ => return,
    };

    *result = None;
    *repair_cost = 0;

    let left = match &input {
        Some(item) => item.clone(),
        None => return,
    };

    let mut cost = 0i32;
    let mut output = left.clone();

    if let Some(ref right) = sacrifice {
        // Check if right item is a repair material for left item
        let left_name = pickaxe_data::item_id_to_name(left.item_id).unwrap_or("");
        let right_name = pickaxe_data::item_id_to_name(right.item_id).unwrap_or("");
        let is_same_item = left.item_id == right.item_id;

        if is_same_item && left.max_damage > 0 {
            // Combining two damaged items: repair = sum of durabilities + 12% bonus
            let left_durability = left.max_damage - left.damage;
            let right_durability = right.max_damage - right.damage;
            let bonus = left.max_damage * 12 / 100;
            let combined = left_durability + right_durability + bonus;
            let new_damage = (left.max_damage - combined).max(0);
            output.damage = new_damage;
            cost += 2;

            // Merge enchantments from right into left
            for &(ench_id, sac_level) in &right.enchantments {
                let target_level = output.enchantment_level(ench_id);
                let new_level = if target_level == sac_level {
                    (sac_level + 1).min(pickaxe_data::enchantment_max_level(ench_id))
                } else {
                    target_level.max(sac_level)
                };
                if let Some(entry) = output.enchantments.iter_mut().find(|(id, _)| *id == ench_id) {
                    entry.1 = new_level;
                } else {
                    output.enchantments.push((ench_id, new_level));
                }
                let anvil_cost = pickaxe_data::enchantment_anvil_cost(ench_id);
                cost += anvil_cost * new_level;
            }
        } else if left.max_damage > 0 && is_repair_material(left_name, right_name) {
            // Material repair: each item repairs 25% of max durability
            let mut damage = left.damage;
            let mut materials_used = 0;
            for _ in 0..right.count {
                let repair_amount = (left.max_damage / 4).max(1);
                if damage <= 0 { break; }
                damage = (damage - repair_amount).max(0);
                materials_used += 1;
                cost += 1;
            }
            if materials_used == 0 && rename.is_none() { return; }
            output.damage = damage;
        } else if right_name == "enchanted_book" && !right.enchantments.is_empty() {
            // Enchanted book: merge enchantments, half anvil cost
            for &(ench_id, sac_level) in &right.enchantments {
                let target_level = output.enchantment_level(ench_id);
                let new_level = if target_level == sac_level {
                    (sac_level + 1).min(pickaxe_data::enchantment_max_level(ench_id))
                } else {
                    target_level.max(sac_level)
                };
                if let Some(entry) = output.enchantments.iter_mut().find(|(id, _)| *id == ench_id) {
                    entry.1 = new_level;
                } else {
                    output.enchantments.push((ench_id, new_level));
                }
                let anvil_cost = (pickaxe_data::enchantment_anvil_cost(ench_id) / 2).max(1);
                cost += anvil_cost * new_level;
            }
        } else if !is_same_item && rename.is_none() {
            // Incompatible items, no rename — no result
            return;
        }
    } else if rename.is_none() {
        // No sacrifice and no rename — nothing to do
        return;
    }

    // Apply rename cost
    if let Some(ref _new_name) = rename {
        cost += 1;
    }

    // Minimum cost of 1
    if cost < 1 { cost = 1; }

    // Too expensive cap (39 for display, but 40+ blocks in survival)
    if cost >= 40 {
        cost = 39;
    }

    *repair_cost = cost;
    *result = Some(output);
}

/// Check if right_name is a valid repair material for left_name.
fn is_repair_material(tool_name: &str, material_name: &str) -> bool {
    match material_name {
        "iron_ingot" => tool_name.starts_with("iron_") || tool_name == "chainmail_helmet" || tool_name == "chainmail_chestplate" || tool_name == "chainmail_leggings" || tool_name == "chainmail_boots",
        "gold_ingot" => tool_name.starts_with("golden_"),
        "diamond" => tool_name.starts_with("diamond_"),
        "netherite_ingot" => tool_name.starts_with("netherite_"),
        "leather" => tool_name.starts_with("leather_"),
        "oak_planks" | "spruce_planks" | "birch_planks" | "jungle_planks"
        | "acacia_planks" | "dark_oak_planks" | "mangrove_planks" | "cherry_planks"
        | "bamboo_planks" | "crimson_planks" | "warped_planks" => {
            tool_name.starts_with("wooden_") || tool_name == "shield"
        }
        "cobblestone" | "cobbled_deepslate" | "blackstone" => tool_name.starts_with("stone_"),
        "string" => tool_name == "bow" || tool_name == "crossbow",
        "phantom_membrane" => tool_name == "elytra",
        _ => false,
    }
}

/// Handle anvil result slot take: deduct XP, consume inputs, chance to damage anvil.
fn handle_anvil_result_take(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    menu: &mut Menu,
) {
    let (input, sacrifice, result, repair_cost, rename, pos) = match menu {
        Menu::Anvil { ref mut input, ref mut sacrifice, ref mut result, ref mut repair_cost, ref mut rename, pos } => {
            (input, sacrifice, result, repair_cost, rename, *pos)
        }
        _ => return,
    };

    if result.is_none() || *repair_cost <= 0 { return; }

    // Check XP in survival
    let gm = world.get::<&PlayerGameMode>(entity).map(|g| g.0).unwrap_or(GameMode::Survival);
    if gm != GameMode::Creative {
        let has_levels = world.get::<&ExperienceData>(entity)
            .map(|xp| xp.level >= *repair_cost)
            .unwrap_or(false);
        if !has_levels {
            *result = None;
            return;
        }
        // Deduct levels
        if let Ok(mut xp) = world.get::<&mut ExperienceData>(entity) {
            xp.level -= *repair_cost;
            xp.progress = 0.0;
            // Recalculate total (approximate)
            let mut total = 0;
            for l in 0..xp.level {
                total += xp_needed_for_level(l);
            }
            xp.total_xp = total;
        }
    }

    // Consume inputs
    *input = None;
    *sacrifice = None;
    *result = None;
    *repair_cost = 0;
    *rename = None;

    // Send XP update
    if let Ok(xp) = world.get::<&ExperienceData>(entity) {
        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
            let _ = sender.0.send(InternalPacket::SetExperience {
                progress: xp.progress,
                level: xp.level,
                total_xp: xp.total_xp,
            });
        }
    }

    // 12% chance to damage the anvil
    if rand::random::<f32>() < 0.12 {
        let current = world_state.get_block(&pos);
        let block_name = pickaxe_data::block_state_to_name(current).unwrap_or("");
        let new_block = match block_name {
            "anvil" => pickaxe_data::block_name_to_default_state("chipped_anvil"),
            "chipped_anvil" => pickaxe_data::block_name_to_default_state("damaged_anvil"),
            "damaged_anvil" => Some(0), // air — anvil breaks
            _ => None,
        };
        if let Some(new_state) = new_block {
            world_state.set_block(&pos, new_state);
            broadcast_to_all(world, &InternalPacket::BlockUpdate {
                position: pos,
                block_id: new_state,
            });
        }
    }
}

/// Handle the RenameItem packet for anvil.
fn handle_anvil_rename(world: &mut World, entity: hecs::Entity, name: &str) {
    let mut open = match world.remove_one::<OpenContainer>(entity) {
        Ok(oc) => oc,
        Err(_) => return,
    };

    if let Menu::Anvil { ref mut rename, .. } = open.menu {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            *rename = None;
        } else {
            *rename = Some(trimmed.chars().take(50).collect());
        }
        calculate_anvil_result(&mut open.menu);

        // Send updated result and cost
        let container_id = open.container_id;
        if let Menu::Anvil { ref result, repair_cost, .. } = &open.menu {
            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                let _ = sender.0.send(InternalPacket::SetContainerData {
                    container_id,
                    property: 0,
                    value: *repair_cost as i16,
                });
                // Send result slot update
                let _ = sender.0.send(InternalPacket::SetContainerSlot {
                    window_id: container_id as i8,
                    state_id: open.state_id,
                    slot: 2,
                    item: result.clone(),
                });
            }
        }
    }

    let _ = world.insert_one(entity, open);
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
    // Ignore movement while sleeping
    if world.get::<&SleepingState>(entity).is_ok() {
        return;
    }

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
    // Check if player is in water (resets fall distance)
    let in_water = {
        let feet_block = world_state.get_block(&BlockPos::new(x.floor() as i32, y.floor() as i32, z.floor() as i32));
        pickaxe_data::is_fluid(feet_block)
    };
    let fall_damage = {
        if let Ok(mut fd) = world.get::<&mut FallDistance>(entity) {
            if on_ground || in_water {
                let damage = if on_ground && fd.0 > 3.0 && !in_water {
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
    next_eid: &Arc<AtomicI32>,
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

    // Calculate damage based on held weapon
    let held_slot = world.get::<&HeldSlot>(attacker).map(|h| h.0).unwrap_or(0);
    let base_damage = {
        let inv = world.get::<&Inventory>(attacker);
        if let Ok(inv) = inv {
            if let Some(ref item) = inv.slots[36 + held_slot as usize] {
                let name = pickaxe_data::item_id_to_name(item.item_id).unwrap_or("");
                pickaxe_data::item_attack_damage(name)
            } else {
                1.0
            }
        } else {
            1.0
        }
    };
    let damage_scale = 0.2 + strength * strength * 0.8;
    let mut damage = base_damage * damage_scale;

    // Strength effect: +3 per level
    if let Ok(effects) = world.get::<&ActiveEffects>(attacker) {
        if let Some(inst) = effects.effects.get(&4) { // strength
            damage += 3.0 * (inst.amplifier as f32 + 1.0);
        }
        // Weakness effect: -4 per level
        if let Some(inst) = effects.effects.get(&17) { // weakness
            damage = (damage - 4.0 * (inst.amplifier as f32 + 1.0)).max(0.0);
        }
    }

    // Sharpness/knockback enchantments
    let mut knockback_bonus = 0.0_f32;
    if let Ok(inv) = world.get::<&Inventory>(attacker) {
        if let Some(ref item) = inv.slots[36 + held_slot as usize] {
            let sharpness = item.enchantment_level(13); // sharpness
            if sharpness > 0 {
                damage += 0.5 + 0.5 * sharpness as f32;
            }
            let knockback_level = item.enchantment_level(16); // knockback
            let fire_aspect = item.enchantment_level(17); // fire_aspect
            // Fire aspect: set target on fire (4 seconds per level)
            if fire_aspect > 0 {
                if let Ok(sender) = world.get::<&ConnectionSender>(attacker) {
                    // EntityEvent for fire is handled by metadata; for now just add damage
                    let _ = sender; // fire aspect visual is TODO
                }
            }
            knockback_bonus = knockback_level as f32;
        }
    }

    // Critical hit: falling, not on ground, strength > 0.9
    let on_ground = world.get::<&OnGround>(attacker).map(|og| og.0).unwrap_or(true);
    let fall_distance = world.get::<&FallDistance>(attacker).map(|fd| fd.0).unwrap_or(0.0);
    let is_sprinting = world.get::<&MovementState>(attacker).map(|ms| ms.sprinting).unwrap_or(false);
    let is_critical = strength > 0.9 && fall_distance > 0.0 && !on_ground && !is_sprinting;

    if is_critical {
        damage *= 1.5;
    }

    let target_eid_val = world.get::<&EntityId>(target).map(|e| e.0).unwrap_or(target_eid);

    // Check if target is a mob
    let is_mob = world.get::<&MobEntity>(target).is_ok();
    let is_player = world.get::<&Profile>(target).is_ok();

    if !is_mob && !is_player {
        return;
    }

    if is_mob {
        attack_mob(world, world_state, attacker, _attacker_eid, target, target_eid_val, damage, is_critical, scripting, next_eid);
    } else {
        // Check if target is blocking and attacker has an axe — disable shield
        let attacker_has_axe = {
            let inv = world.get::<&Inventory>(attacker);
            if let Ok(inv) = inv {
                if let Some(ref item) = inv.slots[36 + held_slot as usize] {
                    let name = pickaxe_data::item_id_to_name(item.item_id).unwrap_or("");
                    pickaxe_data::is_axe(name)
                } else { false }
            } else { false }
        };
        let target_is_blocking = world.get::<&BlockingState>(target).is_ok();

        // PvP: Apply damage to target player
        apply_damage(world, world_state, target, target_eid_val, damage, "player", scripting);

        // If target was blocking and attacker used axe, disable their shield
        if attacker_has_axe && target_is_blocking {
            let _ = world.remove_one::<BlockingState>(target);
            let _ = world.insert_one(target, ShieldCooldown { remaining_ticks: 100 });
            // Clear blocking metadata
            broadcast_to_all(world, &InternalPacket::SetEntityMetadata {
                entity_id: target_eid_val,
                metadata: vec![pickaxe_protocol_core::EntityMetadataEntry {
                    index: 8,
                    type_id: 0,
                    data: vec![0],
                }],
            });
            // Broadcast shield disable event (entity event 30)
            broadcast_to_all(world, &InternalPacket::EntityEvent {
                entity_id: target_eid_val,
                event_id: 30, // shield disable
            });
            let target_pos = world.get::<&Position>(target).map(|p| p.0).unwrap_or(Vec3d::new(0.0, 0.0, 0.0));
            play_sound_at_entity(world, target_pos.x, target_pos.y, target_pos.z, "item.shield.break", SOUND_PLAYERS, 1.0, 1.0);
        }
    }

    // Tool durability loss on attack (2 per hit, survival only)
    if game_mode == GameMode::Survival {
        damage_held_item(world, attacker, _attacker_eid, 2);
    }

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

    // Knockback (vanilla formula from LivingEntity.knockback + Player.attack)
    // Base knockback = attack_knockback_attribute (0 for players) + knockback_enchantment
    // Sprint bonus: +1.0 if sprinting
    let kb_raw = knockback_bonus + if is_sprinting { 1.0 } else { 0.0 };
    // Vanilla multiplies by 0.5 when calling knockback()
    let kb_strength = kb_raw * 0.5;

    if kb_strength > 0.0 || knockback_bonus == 0.0 {
        // Even without enchantment, base knockback of 0.4 is applied
        let effective_kb = if kb_strength > 0.0 { kb_strength } else { 0.4 };

        let attacker_yaw = world.get::<&Rotation>(attacker).map(|r| r.yaw).unwrap_or(0.0);
        let sin_yaw = (attacker_yaw * std::f32::consts::PI / 180.0).sin() as f64;
        let cos_yaw = (attacker_yaw * std::f32::consts::PI / 180.0).cos() as f64;

        // Normalize direction (sin, -cos) and scale by strength
        let dir_len = (sin_yaw * sin_yaw + cos_yaw * cos_yaw).sqrt();
        let dir_x = sin_yaw / dir_len;
        let dir_z = -cos_yaw / dir_len;
        let kb_vec_x = dir_x * effective_kb as f64;
        let kb_vec_z = dir_z * effective_kb as f64;

        // Get target's current velocity
        let (old_vx, old_vy, old_vz) = if is_mob {
            world.get::<&Velocity>(target).map(|v| (v.0.x, v.0.y, v.0.z)).unwrap_or((0.0, 0.0, 0.0))
        } else {
            (0.0, 0.0, 0.0) // Players: server doesn't track their velocity
        };

        let target_on_ground = world.get::<&OnGround>(target).map(|og| og.0).unwrap_or(true);

        // Vanilla formula: halve existing velocity, subtract knockback vector
        // Y: if on ground, min(0.4, old_y/2 + strength), else keep old_y
        let new_vx = old_vx / 2.0 - kb_vec_x;
        let new_vy = if target_on_ground {
            (old_vy / 2.0 + effective_kb as f64).min(0.4)
        } else {
            old_vy
        };
        let new_vz = old_vz / 2.0 - kb_vec_z;

        // Send velocity packet to target
        let vel_packet = InternalPacket::SetEntityVelocity {
            entity_id: target_eid_val,
            velocity_x: (new_vx.clamp(-3.9, 3.9) * 8000.0) as i16,
            velocity_y: (new_vy.clamp(-3.9, 3.9) * 8000.0) as i16,
            velocity_z: (new_vz.clamp(-3.9, 3.9) * 8000.0) as i16,
        };

        if is_player {
            if let Ok(sender) = world.get::<&ConnectionSender>(target) {
                let _ = sender.0.send(vel_packet.clone());
            }
        }
        if is_mob {
            if let Ok(mut vel) = world.get::<&mut Velocity>(target) {
                vel.0.x = new_vx;
                vel.0.y = new_vy;
                vel.0.z = new_vz;
            }
            broadcast_to_all(world, &vel_packet);
        }
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

    // Shield blocking check — shields block everything except void, starvation, drowning, lava, lightning, fall
    let blockable = !matches!(source, "void" | "starve" | "starvation" | "drowning" | "lava" | "lightning" | "fall");
    let shield_blocked = if blockable {
        // Extract blocking info first to avoid borrow conflicts
        let blocking_info = world.get::<&BlockingState>(entity)
            .ok()
            .map(|b| (b.start_tick, b.hand));
        if let Some((start_tick, _shield_hand)) = blocking_info {
            world_state.tick_count.saturating_sub(start_tick) >= 5
        } else {
            false
        }
    } else {
        false
    };

    if shield_blocked {
        let shield_hand = world.get::<&BlockingState>(entity).map(|b| b.hand).unwrap_or(0);
        // Apply shield durability damage (minimum 3)
        let dur_damage = (damage.floor() as i32).max(3);
        let shield_slot = if shield_hand == 1 { 45 } else {
            let hs = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
            36 + hs as usize
        };
        let mut shield_broke = false;
        if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
            if let Some(ref mut shield_item) = inv.slots[shield_slot] {
                let new_damage = shield_item.damage + dur_damage;
                if shield_item.max_damage > 0 && new_damage >= shield_item.max_damage {
                    inv.slots[shield_slot] = None;
                    shield_broke = true;
                } else {
                    shield_item.damage = new_damage;
                }
                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                    let _ = sender.0.send(InternalPacket::SetContainerSlot {
                        window_id: 0,
                        state_id: inv.state_id,
                        slot: shield_slot as i16,
                        item: inv.slots[shield_slot].clone(),
                    });
                }
            }
        }
        if shield_broke {
            let pos = world.get::<&Position>(entity).map(|p| p.0).unwrap_or(Vec3d::new(0.0, 0.0, 0.0));
            play_sound_at_entity(world, pos.x, pos.y, pos.z, "item.shield.break", SOUND_PLAYERS, 1.0, 1.0);
            // Remove blocking state since shield broke
            let _ = world.remove_one::<BlockingState>(entity);
            // Clear blocking metadata
            broadcast_to_all(world, &InternalPacket::SetEntityMetadata {
                entity_id,
                metadata: vec![pickaxe_protocol_core::EntityMetadataEntry {
                    index: 8,
                    type_id: 0,
                    data: vec![0],
                }],
            });
        } else {
            let pos = world.get::<&Position>(entity).map(|p| p.0).unwrap_or(Vec3d::new(0.0, 0.0, 0.0));
            play_sound_at_entity(world, pos.x, pos.y, pos.z, "item.shield.block", SOUND_PLAYERS, 1.0, 1.0);
        }
        // Damage fully blocked — return
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

    // Cancel eating and blocking on damage
    let _ = world.remove_one::<EatingState>(entity);
    if world.remove_one::<BlockingState>(entity).is_ok() {
        broadcast_to_all(world, &InternalPacket::SetEntityMetadata {
            entity_id,
            metadata: vec![pickaxe_protocol_core::EntityMetadataEntry {
                index: 8,
                type_id: 0,
                data: vec![0],
            }],
        });
    }

    // Wake up if sleeping
    if world.get::<&SleepingState>(entity).is_ok() {
        wake_player(world, world_state, entity, entity_id);
    }

    // Fire resistance: immune to fire/lava damage
    if source == "lava" || source == "fire" || source == "lightning" {
        if let Ok(effects) = world.get::<&ActiveEffects>(entity) {
            if effects.effects.contains_key(&11) { // fire_resistance
                return;
            }
        }
    }

    // Resistance effect: reduce damage by 20% per level (max 80%)
    let damage = if source != "void" && source != "starvation" {
        if let Ok(effects) = world.get::<&ActiveEffects>(entity) {
            if let Some(inst) = effects.effects.get(&10) { // resistance
                let reduction = ((inst.amplifier + 1) * 5).min(20) as f32 / 25.0;
                damage * (1.0 - reduction)
            } else {
                damage
            }
        } else {
            damage
        }
    } else {
        damage
    };

    // Apply armor damage reduction (not for void/starvation)
    let final_damage = if source != "void" && source != "starvation" {
        // Sum armor defense, toughness, and protection enchant levels from equipped armor
        let (total_armor, total_toughness, total_protection) = if let Ok(inv) = world.get::<&Inventory>(entity) {
            let mut armor = 0i32;
            let mut toughness = 0.0f32;
            let mut prot = 0i32;
            for slot_idx in 5..=8 {
                if let Some(ref item) = inv.slots[slot_idx] {
                    if let Some(name) = pickaxe_data::item_id_to_name(item.item_id) {
                        if let Some((def, tough)) = pickaxe_data::armor_defense(name) {
                            armor += def;
                            toughness += tough;
                        }
                    }
                    // Protection enchantment (id 0): each level = 4% reduction
                    prot += item.enchantment_level(0);
                    // Fire protection (1), blast protection (3), projectile protection (4)
                    // count as general protection too for simplicity
                    prot += item.enchantment_level(1);
                    prot += item.enchantment_level(3);
                    prot += item.enchantment_level(4);
                }
            }
            (armor, toughness, prot)
        } else {
            (0, 0.0, 0)
        };

        let after_armor = if total_armor > 0 {
            // Vanilla damage reduction formula (CombatRules.java)
            let toughness_factor = 2.0 + total_toughness / 4.0;
            let effective_armor = (total_armor as f32 - damage / toughness_factor)
                .clamp(total_armor as f32 * 0.2, 20.0);
            let reduction = effective_armor / 25.0;
            damage * (1.0 - reduction)
        } else {
            damage
        };

        // Protection enchantment: 4% per level, capped at 80%
        let prot_reduction = (total_protection as f32 * 4.0).min(80.0) / 100.0;
        let reduced = after_armor * (1.0 - prot_reduction);

        // Damage armor pieces: durabilityLoss = max(1, floor(damage / 4))
        if total_armor > 0 {
            let armor_damage = (damage / 4.0).floor().max(1.0) as i32;
            if let Ok(mut inv) = world.get::<&mut Inventory>(entity) {
                let mut broken_slots = Vec::new();
                for slot_idx in 5..=8 {
                    if let Some(ref mut item) = inv.slots[slot_idx] {
                        if item.max_damage > 0 {
                            // Unbreaking enchantment: chance to not consume durability
                            let unbreaking = item.enchantment_level(22);
                            if unbreaking > 0 {
                                // Armor: 60% + 40% / (unbreaking + 1) chance to damage
                                let chance = 0.6 + 0.4 / (unbreaking as f32 + 1.0);
                                if rand::random::<f32>() > chance {
                                    continue;
                                }
                            }
                            item.damage += armor_damage;
                            if item.damage >= item.max_damage {
                                broken_slots.push(slot_idx);
                            }
                        }
                    }
                }
                for slot_idx in broken_slots {
                    inv.set_slot(slot_idx, None);
                }
            }

            // Send updated equipment to all observers
            send_equipment_update(world, entity, entity_id);
        }

        reduced
    } else {
        damage
    };

    // Apply damage
    let (new_health, is_dead) = {
        let mut health = match world.get::<&mut Health>(entity) {
            Ok(h) => h,
            Err(_) => return,
        };
        health.current = (health.current - final_damage).max(0.0);
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

/// Try to make a player sleep in a bed.
fn try_sleep_in_bed(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    entity_id: i32,
    clicked_pos: &BlockPos,
    bed_state: i32,
    scripting: &ScriptRuntime,
) {
    let name = world.get::<&Profile>(entity).map(|p| p.0.name.clone()).unwrap_or_default();

    // Already sleeping?
    if world.get::<&SleepingState>(entity).is_ok() {
        return;
    }

    // Dead?
    let health = world.get::<&Health>(entity).map(|h| h.current).unwrap_or(20.0);
    if health <= 0.0 {
        return;
    }

    // Determine the head block position (sleep always targets head)
    let head_pos = if pickaxe_data::bed_is_head(bed_state) {
        *clicked_pos
    } else {
        let facing = pickaxe_data::bed_facing(bed_state);
        let (dx, dz) = pickaxe_data::bed_head_offset(facing);
        BlockPos::new(clicked_pos.x + dx, clicked_pos.y, clicked_pos.z + dz)
    };

    // Check bed not occupied
    let head_block = world_state.get_block(&head_pos);
    if pickaxe_data::is_bed(head_block) {
        // Check occupied property
        let state_offset = (head_block - 1688) % 16;
        let occupied = (state_offset / 2) % 2 == 1;
        if occupied {
            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                let _ = sender.0.send(InternalPacket::SystemChatMessage {
                    content: TextComponent::plain("This bed is occupied"),
                    overlay: true,
                });
            }
            return;
        }
    }

    // Check nighttime: time_of_day 12542..=23459 is valid sleep time (MC source)
    let time = world_state.time_of_day % 24000;
    let is_night = time >= 12542 || time < 0; // thunderstorms also allow, but we don't have weather
    if !is_night {
        // Set spawn point even if can't sleep (MC behavior)
        let yaw = world.get::<&Rotation>(entity).map(|r| r.yaw).unwrap_or(0.0);
        let _ = world.insert_one(entity, SpawnPoint { position: head_pos, yaw });
        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
            let _ = sender.0.send(InternalPacket::SystemChatMessage {
                content: TextComponent::plain("You can only sleep at night"),
                overlay: true,
            });
        }
        return;
    }

    // Set spawn point
    let yaw = world.get::<&Rotation>(entity).map(|r| r.yaw).unwrap_or(0.0);
    let _ = world.insert_one(entity, SpawnPoint { position: head_pos, yaw });

    // Send "set spawn" message and update client spawn position
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SetDefaultSpawnPosition {
            position: head_pos,
            angle: yaw,
        });
    }

    // Set bed block occupied
    let new_head_state = pickaxe_data::bed_set_occupied(head_block, true);
    world_state.set_block(&head_pos, new_head_state);
    broadcast_to_all(world, &InternalPacket::BlockUpdate {
        position: head_pos,
        block_id: new_head_state,
    });

    // Also set foot block occupied
    let facing = pickaxe_data::bed_facing(head_block);
    let (dx, dz) = pickaxe_data::bed_head_offset(facing);
    let foot_pos = BlockPos::new(head_pos.x - dx, head_pos.y, head_pos.z - dz);
    let foot_block = world_state.get_block(&foot_pos);
    if pickaxe_data::is_bed(foot_block) {
        let new_foot_state = pickaxe_data::bed_set_occupied(foot_block, true);
        world_state.set_block(&foot_pos, new_foot_state);
        broadcast_to_all(world, &InternalPacket::BlockUpdate {
            position: foot_pos,
            block_id: new_foot_state,
        });
    }

    // Move player to bed position (bed Y + 0.6875)
    let bed_x = head_pos.x as f64 + 0.5;
    let bed_y = head_pos.y as f64 + 0.6875;
    let bed_z = head_pos.z as f64 + 0.5;
    if let Ok(mut pos) = world.get::<&mut Position>(entity) {
        pos.0 = Vec3d::new(bed_x, bed_y, bed_z);
    }

    // Set sleeping pose via entity metadata (broadcast to all players)
    let metadata = build_sleeping_metadata(&head_pos);
    broadcast_to_all(world, &InternalPacket::SetEntityMetadata {
        entity_id,
        metadata,
    });

    // Add sleeping state component
    let _ = world.insert_one(entity, SleepingState {
        bed_pos: head_pos,
        sleep_timer: 0,
    });

    // Fire Lua event
    scripting.fire_event_in_context(
        "player_sleep",
        &[
            ("name", &name),
            ("x", &head_pos.x.to_string()),
            ("y", &head_pos.y.to_string()),
            ("z", &head_pos.z.to_string()),
        ],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );

    debug!("{} is now sleeping at {:?}", name, head_pos);
}

/// Wake a player up from sleeping.
fn wake_player(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    entity_id: i32,
) {
    let sleeping = match world.get::<&SleepingState>(entity) {
        Ok(s) => s.bed_pos,
        Err(_) => return,
    };

    // Broadcast wake-up animation (action 2)
    broadcast_to_all(world, &InternalPacket::EntityAnimation {
        entity_id,
        animation: 2, // WAKE_UP
    });

    // Clear sleeping pose metadata
    let metadata = build_wake_metadata();
    broadcast_to_all(world, &InternalPacket::SetEntityMetadata {
        entity_id,
        metadata,
    });

    // Set bed block unoccupied
    let head_block = world_state.get_block(&sleeping);
    if pickaxe_data::is_bed(head_block) {
        let new_state = pickaxe_data::bed_set_occupied(head_block, false);
        world_state.set_block(&sleeping, new_state);
        broadcast_to_all(world, &InternalPacket::BlockUpdate {
            position: sleeping,
            block_id: new_state,
        });

        // Also unset foot block
        let facing = pickaxe_data::bed_facing(head_block);
        let (dx, dz) = pickaxe_data::bed_head_offset(facing);
        let foot_pos = BlockPos::new(sleeping.x - dx, sleeping.y, sleeping.z - dz);
        let foot_block = world_state.get_block(&foot_pos);
        if pickaxe_data::is_bed(foot_block) {
            let new_foot = pickaxe_data::bed_set_occupied(foot_block, false);
            world_state.set_block(&foot_pos, new_foot);
            broadcast_to_all(world, &InternalPacket::BlockUpdate {
                position: foot_pos,
                block_id: new_foot,
            });
        }
    }

    // Teleport player to stand-up position (beside the bed)
    let facing = pickaxe_data::bed_facing(head_block);
    let (dx, dz) = pickaxe_data::bed_head_offset(facing);
    // Stand-up position: foot side of the bed, offset by 1 in the opposite facing direction
    let stand_x = sleeping.x as f64 + 0.5 - dx as f64;
    let stand_y = sleeping.y as f64 + 0.6;
    let stand_z = sleeping.z as f64 + 0.5 - dz as f64;
    if let Ok(mut pos) = world.get::<&mut Position>(entity) {
        pos.0 = Vec3d::new(stand_x, stand_y, stand_z);
    }

    // Send teleport to the sleeping player
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let _ = sender.0.send(InternalPacket::SynchronizePlayerPosition {
            position: Vec3d::new(stand_x, stand_y, stand_z),
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
            teleport_id: 200,
        });
    }

    // Remove sleeping state
    let _ = world.remove_one::<SleepingState>(entity);
}

/// Tick sleeping: increment timers, check for night skip when all players are sleeping.
fn tick_sleeping(
    world: &mut World,
    world_state: &mut WorldState,
    scripting: &ScriptRuntime,
) {
    // Count total players and sleeping players
    let mut total_players = 0u32;
    let mut sleeping_long_enough = 0u32;
    let mut all_sleepers: Vec<(hecs::Entity, i32)> = Vec::new();

    // Increment sleep timers
    for (entity, (eid, sleep)) in world.query_mut::<(&EntityId, &mut SleepingState)>() {
        sleep.sleep_timer += 1;
        all_sleepers.push((entity, eid.0));
        if sleep.sleep_timer >= 100 {
            sleeping_long_enough += 1;
        }
    }

    // Count total players
    for _ in world.query::<&Profile>().iter() {
        total_players += 1;
    }

    // Check for night skip: all players must be sleeping long enough
    if total_players > 0 && sleeping_long_enough == total_players {
        // Advance time to dawn (time_of_day = 0 of next day cycle)
        let time = world_state.time_of_day;
        let skip_ticks = if time >= 12542 {
            24000 - time
        } else {
            // Already past dawn, shouldn't happen but handle gracefully
            0
        };

        if skip_ticks > 0 {
            world_state.time_of_day = 0;
            world_state.world_age += skip_ticks;

            // Broadcast time update immediately
            broadcast_to_all(world, &InternalPacket::UpdateTime {
                world_age: world_state.world_age,
                time_of_day: world_state.time_of_day,
            });

            // Fire Lua event
            scripting.fire_event_in_context(
                "night_skip",
                &[],
                world as *mut _ as *mut (),
                world_state as *mut _ as *mut (),
            );

            info!("All players sleeping — skipping night (advanced {} ticks)", skip_ticks);
        }

        // Wake all sleeping players
        for (entity, eid) in all_sleepers {
            wake_player(world, world_state, entity, eid);
        }
    }
}

/// Spawn a mob entity in the world.
fn spawn_mob(
    world: &mut World,
    next_eid: &Arc<AtomicI32>,
    mob_type: i32,
    x: f64,
    y: f64,
    z: f64,
) -> hecs::Entity {
    let entity_id = next_eid.fetch_add(1, Ordering::Relaxed);
    let max_hp = pickaxe_data::mob_max_health(mob_type);
    let yaw: f32 = rand::random::<f32>() * 360.0;

    world.spawn((
        EntityId(entity_id),
        EntityUuid(Uuid::new_v4()),
        Position(Vec3d::new(x, y, z)),
        PreviousPosition(Vec3d::new(x, y, z)),
        Rotation { yaw, pitch: 0.0 },
        PreviousRotation { yaw, pitch: 0.0 },
        OnGround(true),
        Velocity(Vec3d::new(0.0, 0.0, 0.0)),
        MobEntity {
            mob_type,
            health: max_hp,
            max_health: max_hp,
            target: None,
            ai_state: MobAiState::Idle,
            ai_timer: rand::random::<u32>() % 80 + 20,
            ambient_sound_timer: rand::random::<u32>() % 200 + 100,
            no_damage_ticks: 0,
            fuse_timer: -1,
            attack_cooldown: 0,
        },
    ))
}

/// Handle a player attacking a mob entity.
fn attack_mob(
    world: &mut World,
    world_state: &mut WorldState,
    attacker: hecs::Entity,
    _attacker_eid: i32,
    target: hecs::Entity,
    target_eid: i32,
    damage: f32,
    is_critical: bool,
    scripting: &ScriptRuntime,
    next_eid: &Arc<AtomicI32>,
) {
    // Check invulnerability
    let no_dmg = world.get::<&MobEntity>(target).map(|m| m.no_damage_ticks > 0).unwrap_or(false);
    if no_dmg {
        return;
    }

    let mob_type = world.get::<&MobEntity>(target).map(|m| m.mob_type).unwrap_or(0);
    let mob_name = pickaxe_data::mob_type_name(mob_type).unwrap_or("unknown");
    let attacker_name = world.get::<&Profile>(attacker).map(|p| p.0.name.clone()).unwrap_or_default();

    // Fire Lua event
    let cancelled = scripting.fire_event_in_context(
        "mob_damage",
        &[
            ("attacker", &attacker_name),
            ("mob_type", mob_name),
            ("amount", &format!("{:.1}", damage)),
            ("entity_id", &target_eid.to_string()),
        ],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );
    if cancelled {
        return;
    }

    // Apply damage
    let died = {
        let mut mob = world.get::<&mut MobEntity>(target).unwrap();
        mob.health -= damage;
        mob.no_damage_ticks = 10; // 0.5s invulnerability
        // Hostile mobs target the attacker
        if pickaxe_data::mob_is_hostile(mob.mob_type) {
            mob.target = Some(attacker);
            mob.ai_state = MobAiState::Chasing;
        }
        mob.health <= 0.0
    };

    let mob_pos = world.get::<&Position>(target).map(|p| p.0).unwrap_or(Vec3d::new(0.0, 0.0, 0.0));

    // Play hurt sound
    let (_, hurt_sound, death_sound) = pickaxe_data::mob_sounds(mob_type);

    if died {
        // Play death sound
        play_sound_at_entity(world, mob_pos.x, mob_pos.y, mob_pos.z, death_sound, SOUND_HOSTILE, 1.0, 1.0);

        // Broadcast entity event (death animation = status 3)
        broadcast_to_all(world, &InternalPacket::EntityEvent {
            entity_id: target_eid,
            event_id: 3, // death
        });

        // Drop items
        let drops = pickaxe_data::mob_drops(mob_type);
        for (item_name, min, max) in drops {
            let count = if min == max {
                *min
            } else {
                let range = (max - min + 1) as u32;
                *min + (rand::random::<u32>() % range) as i32
            };
            if count > 0 {
                if let Some(item_id) = pickaxe_data::item_name_to_id(item_name) {
                    let item = ItemStack::new(item_id, count as i8);
                    spawn_item_entity(world, world_state, next_eid, mob_pos.x, mob_pos.y + 0.5, mob_pos.z, item, 10, scripting);
                }
            }
        }

        // Award XP
        let xp = pickaxe_data::mob_xp_drop(mob_type);
        if xp > 0 {
            award_xp(world, attacker, xp);
        }

        // Despawn mob
        let _ = world.despawn(target);
        broadcast_to_all(world, &InternalPacket::RemoveEntities {
            entity_ids: vec![target_eid],
        });
        for (_, tracked) in world.query_mut::<&mut TrackedEntities>() {
            tracked.visible.remove(&target_eid);
        }

        // Fire Lua event
        scripting.fire_event_in_context(
            "mob_death",
            &[
                ("mob_type", mob_name),
                ("killer", &attacker_name),
                ("entity_id", &target_eid.to_string()),
            ],
            world as *mut _ as *mut (),
            world_state as *mut _ as *mut (),
        );
    } else {
        // Play hurt sound + hurt animation
        play_sound_at_entity(world, mob_pos.x, mob_pos.y, mob_pos.z, hurt_sound, SOUND_HOSTILE, 1.0, 1.0);
        broadcast_to_all(world, &InternalPacket::EntityEvent {
            entity_id: target_eid,
            event_id: 2, // hurt
        });
    }

    let _ = is_critical; // used by caller for particles
}

/// Tick mob AI: wandering, chasing, ambient sounds, gravity.
fn tick_mob_ai(
    world: &mut World,
    world_state: &mut WorldState,
    _scripting: &ScriptRuntime,
    next_eid: &Arc<AtomicI32>,
) {
    // Collect player positions for targeting
    let mut player_positions: Vec<(hecs::Entity, i32, Vec3d)> = Vec::new();
    for (e, (eid, pos, _profile)) in world.query::<(&EntityId, &Position, &Profile)>().iter() {
        let health = world.get::<&Health>(e).map(|h| h.current).unwrap_or(0.0);
        if health > 0.0 {
            player_positions.push((e, eid.0, pos.0));
        }
    }

    // Collect mob data for AI updates
    #[allow(dead_code)]
    struct MobUpdate {
        entity: hecs::Entity,
        eid: i32,
        mob_type: i32,
        pos: Vec3d,
        new_state: MobAiState,
        move_x: f64,
        move_z: f64,
        new_yaw: f32,
        ambient_sound: bool,
    }

    let mut updates: Vec<MobUpdate> = Vec::new();

    for (entity, (eid, pos, rot, mob)) in world.query::<(&EntityId, &Position, &Rotation, &mut MobEntity)>().iter() {
        // Decrement timers
        if mob.no_damage_ticks > 0 {
            mob.no_damage_ticks -= 1;
        }

        let mut ambient_sound = false;
        if mob.ambient_sound_timer > 0 {
            mob.ambient_sound_timer -= 1;
        } else {
            ambient_sound = true;
            mob.ambient_sound_timer = rand::random::<u32>() % 300 + 200;
        }

        if mob.ai_timer > 0 {
            mob.ai_timer -= 1;
            // Continue current behavior
            let speed = pickaxe_data::mob_speed(mob.mob_type);
            let (mx, mz) = match mob.ai_state {
                MobAiState::Wandering => {
                    let yaw_rad = rot.yaw * std::f32::consts::PI / 180.0;
                    ((-yaw_rad.sin() as f64) * speed, (yaw_rad.cos() as f64) * speed)
                }
                MobAiState::Chasing => {
                    if let Some(target) = mob.target {
                        if let Ok(tp) = world.get::<&Position>(target) {
                            let dx = tp.0.x - pos.0.x;
                            let dz = tp.0.z - pos.0.z;
                            let dist = (dx * dx + dz * dz).sqrt();

                            // Skeleton: keep distance (8-12 blocks) for ranged attacks
                            if mob.mob_type == pickaxe_data::MOB_SKELETON {
                                if dist < 6.0 {
                                    // Too close — retreat
                                    let chase_speed = speed * 1.2;
                                    (-dx / dist * chase_speed, -dz / dist * chase_speed)
                                } else if dist > 14.0 {
                                    // Too far — close in
                                    let chase_speed = speed * 1.3;
                                    (dx / dist * chase_speed, dz / dist * chase_speed)
                                } else {
                                    // In sweet spot — strafe slightly
                                    let strafe_speed = speed * 0.5;
                                    (-dz / dist * strafe_speed, dx / dist * strafe_speed)
                                }
                            } else if dist > 1.5 {
                                let chase_speed = speed * 1.3;
                                (dx / dist * chase_speed, dz / dist * chase_speed)
                            } else {
                                (0.0, 0.0) // close enough, stop moving
                            }
                        } else {
                            mob.target = None;
                            mob.ai_state = MobAiState::Idle;
                            (0.0, 0.0)
                        }
                    } else {
                        mob.ai_state = MobAiState::Idle;
                        (0.0, 0.0)
                    }
                }
                MobAiState::Fleeing => {
                    // Move away from target (bat retreat, creeper post-fuse cancel)
                    if let Some(target) = mob.target {
                        if let Ok(tp) = world.get::<&Position>(target) {
                            let dx = pos.0.x - tp.0.x; // reversed: away from target
                            let dz = pos.0.z - tp.0.z;
                            let dist = (dx * dx + dz * dz).sqrt();
                            if dist > 0.1 {
                                (dx / dist * speed, dz / dist * speed)
                            } else {
                                (0.0, 0.0)
                            }
                        } else {
                            mob.target = None;
                            mob.ai_state = MobAiState::Idle;
                            (0.0, 0.0)
                        }
                    } else {
                        mob.ai_state = MobAiState::Idle;
                        (0.0, 0.0)
                    }
                }
                MobAiState::Idle => (0.0, 0.0),
            };

            // Calculate new yaw for chasing
            let new_yaw = if (mob.ai_state == MobAiState::Chasing || mob.ai_state == MobAiState::Fleeing) && (mx != 0.0 || mz != 0.0) {
                (-mx.atan2(mz) * 180.0 / std::f64::consts::PI) as f32
            } else {
                rot.yaw
            };

            updates.push(MobUpdate {
                entity, eid: eid.0, mob_type: mob.mob_type,
                pos: pos.0, new_state: mob.ai_state,
                move_x: mx, move_z: mz, new_yaw,
                ambient_sound,
            });
            continue;
        }

        // AI decision time
        // Spiders are only hostile at night
        let is_hostile = if mob.mob_type == pickaxe_data::MOB_SPIDER {
            let time = world_state.time_of_day % 24000;
            time >= 13000 && time < 23000 // hostile only at night
        } else {
            pickaxe_data::mob_is_hostile(mob.mob_type)
        };

        // Bats just flutter around, never chase
        let is_bat = mob.mob_type == pickaxe_data::MOB_BAT;

        if is_hostile && !is_bat {
            // Find nearest player within 16 blocks
            let mut nearest: Option<(hecs::Entity, f64)> = None;
            for &(pe, _peid, ppos) in &player_positions {
                let dx = ppos.x - pos.0.x;
                let dz = ppos.z - pos.0.z;
                let dist = (dx * dx + dz * dz).sqrt();
                if dist < 16.0 {
                    if nearest.is_none() || dist < nearest.unwrap().1 {
                        nearest = Some((pe, dist));
                    }
                }
            }

            if let Some((target_entity, _)) = nearest {
                mob.target = Some(target_entity);
                mob.ai_state = MobAiState::Chasing;
                mob.ai_timer = 40 + rand::random::<u32>() % 20;
            } else {
                // Wander randomly
                let r: f32 = rand::random();
                if r < 0.3 {
                    mob.ai_state = MobAiState::Wandering;
                    mob.ai_timer = 40 + rand::random::<u32>() % 60;
                } else {
                    mob.ai_state = MobAiState::Idle;
                    mob.ai_timer = 40 + rand::random::<u32>() % 80;
                }
            }
        } else {
            // Passive mob or bat: wander or idle
            let r: f32 = rand::random();
            if r < 0.3 {
                mob.ai_state = MobAiState::Wandering;
                mob.ai_timer = 40 + rand::random::<u32>() % 60;
            } else {
                mob.ai_state = MobAiState::Idle;
                mob.ai_timer = 60 + rand::random::<u32>() % 100;
            }
        }

        // New random direction for wandering
        let new_yaw = if mob.ai_state == MobAiState::Wandering {
            rand::random::<f32>() * 360.0
        } else {
            rot.yaw
        };

        updates.push(MobUpdate {
            entity, eid: eid.0, mob_type: mob.mob_type,
            pos: pos.0, new_state: mob.ai_state,
            move_x: 0.0, move_z: 0.0, new_yaw,
            ambient_sound,
        });
    }

    // Apply movement + sounds
    for update in &updates {
        // Ambient sound
        if update.ambient_sound {
            let (ambient, _, _) = pickaxe_data::mob_sounds(update.mob_type);
            if !ambient.is_empty() {
                let sound_cat = if pickaxe_data::mob_is_hostile(update.mob_type) { SOUND_HOSTILE } else { SOUND_NEUTRAL };
                play_sound_at_entity(world, update.pos.x, update.pos.y, update.pos.z, ambient, sound_cat, 1.0, 1.0);
            }
        }

        // Apply movement with proper collision (2-block height check + step-up)
        if update.move_x != 0.0 || update.move_z != 0.0 {
            if let Ok(mut pos) = world.get::<&mut Position>(update.entity) {
                let new_x = pos.0.x + update.move_x;
                let new_z = pos.0.z + update.move_z;
                let feet_y = pos.0.y.floor() as i32;
                let bx = new_x.floor() as i32;
                let bz = new_z.floor() as i32;

                // Check 2-block clearance at target position (feet and head)
                let block_feet = world_state.get_block(&BlockPos::new(bx, feet_y, bz));
                let block_head = world_state.get_block(&BlockPos::new(bx, feet_y + 1, bz));

                if block_feet == 0 && block_head == 0 {
                    // Clear path — check there's ground below to prevent walking off edges
                    let block_below = world_state.get_block(&BlockPos::new(bx, feet_y - 1, bz));
                    if block_below != 0 {
                        pos.0.x = new_x;
                        pos.0.z = new_z;
                    }
                    // else: no ground ahead, mob stays put (avoids walking off cliffs)
                } else if block_feet != 0 && block_head == 0 {
                    // 1-block obstacle — try stepping up
                    let step_feet = world_state.get_block(&BlockPos::new(bx, feet_y + 1, bz));
                    let step_head = world_state.get_block(&BlockPos::new(bx, feet_y + 2, bz));
                    if step_feet == 0 && step_head == 0 {
                        pos.0.x = new_x;
                        pos.0.y = (feet_y + 1) as f64;
                        pos.0.z = new_z;
                    }
                }
                // else: blocked by 2+ block wall, stay put
            }
        }

        // Update yaw
        if let Ok(mut rot) = world.get::<&mut Rotation>(update.entity) {
            rot.yaw = update.new_yaw;
        }

        // Gravity
        if let Ok(og) = world.get::<&OnGround>(update.entity) {
            if !og.0 {
                if let Ok(mut vel) = world.get::<&mut Velocity>(update.entity) {
                    vel.0.y -= 0.08; // gravity
                    vel.0.y *= 0.98; // drag
                }
            }
        }

        // Apply velocity (for knockback / gravity)
        let vel = world.get::<&Velocity>(update.entity).map(|v| v.0).unwrap_or(Vec3d::new(0.0, 0.0, 0.0));
        if vel.x.abs() > 0.001 || vel.y.abs() > 0.001 || vel.z.abs() > 0.001 {
            if let Ok(mut pos) = world.get::<&mut Position>(update.entity) {
                pos.0.x += vel.x;
                pos.0.y += vel.y;
                pos.0.z += vel.z;

                // Ground collision
                let block_below = world_state.get_block(&BlockPos::new(
                    pos.0.x.floor() as i32,
                    (pos.0.y - 0.01).floor() as i32,
                    pos.0.z.floor() as i32,
                ));
                if block_below != 0 && vel.y <= 0.0 {
                    pos.0.y = (pos.0.y - 0.01).floor() + 1.0;
                    if let Ok(mut og) = world.get::<&mut OnGround>(update.entity) {
                        og.0 = true;
                    }
                    if let Ok(mut v) = world.get::<&mut Velocity>(update.entity) {
                        v.0.y = 0.0;
                    }
                } else if block_below == 0 {
                    if let Ok(mut og) = world.get::<&mut OnGround>(update.entity) {
                        og.0 = false;
                    }
                }
            }

            // Dampen horizontal velocity
            if let Ok(mut v) = world.get::<&mut Velocity>(update.entity) {
                v.0.x *= 0.6;
                v.0.z *= 0.6;
                if v.0.x.abs() < 0.003 { v.0.x = 0.0; }
                if v.0.z.abs() < 0.003 { v.0.z = 0.0; }
            }
        }
    }

    // --- Undead sunlight burning (zombies, skeletons) ---
    let is_daytime = {
        let time = world_state.time_of_day % 24000;
        time < 13000 || time >= 23000
    };
    if is_daytime && world_state.tick_count % 20 == 0 {
        let mut burn_targets: Vec<(hecs::Entity, i32, Vec3d)> = Vec::new();
        for (entity, (eid, pos, mob)) in world.query::<(&EntityId, &Position, &MobEntity)>().iter() {
            if mob.mob_type == pickaxe_data::MOB_ZOMBIE || mob.mob_type == pickaxe_data::MOB_SKELETON {
                // Check if exposed to sky (no solid block above)
                let bx = pos.0.x.floor() as i32;
                let by = pos.0.y.floor() as i32;
                let bz = pos.0.z.floor() as i32;
                let mut exposed = true;
                for check_y in (by + 2)..=320 {
                    if let Some(b) = world_state.get_block_if_loaded(&BlockPos::new(bx, check_y, bz)) {
                        if b != 0 { exposed = false; break; }
                    } else {
                        break; // Chunk not loaded, assume exposed (flat world)
                    }
                }
                if exposed {
                    burn_targets.push((entity, eid.0, pos.0));
                }
            }
        }
        for (entity, eid, pos) in burn_targets {
            // Deal 1 damage per second (1 HP every 20 ticks)
            let health = world.get::<&Health>(entity).map(|h| h.current).unwrap_or(0.0);
            if health > 0.0 {
                let new_health = (health - 1.0).max(0.0);
                if let Ok(mut h) = world.get::<&mut Health>(entity) {
                    h.current = new_health;
                }
                // Play fire damage effects
                broadcast_to_all(world, &InternalPacket::EntityEvent { entity_id: eid, event_id: 2 }); // hurt
                play_sound_at_entity(world, pos.x, pos.y, pos.z, "entity.generic.burn", SOUND_HOSTILE, 0.8, 1.0);
                if new_health <= 0.0 {
                    // Kill mob — drop loot
                    let mob_type = world.get::<&MobEntity>(entity).map(|m| m.mob_type).unwrap_or(0);
                    let (_, _, death_sound) = pickaxe_data::mob_sounds(mob_type);
                    play_sound_at_entity(world, pos.x, pos.y, pos.z, death_sound, SOUND_HOSTILE, 1.0, 1.0);
                    broadcast_to_all(world, &InternalPacket::EntityEvent { entity_id: eid, event_id: 3 });
                    let drops = pickaxe_data::mob_drops(mob_type);
                    for (item_name, min, max) in drops {
                        let count = if min == max { *min } else { *min + (rand::random::<u32>() % (max - min + 1) as u32) as i32 };
                        if count > 0 {
                            if let Some(item_id) = pickaxe_data::item_name_to_id(item_name) {
                                spawn_item_entity(world, world_state, next_eid, pos.x, pos.y + 0.5, pos.z, ItemStack::new(item_id, count as i8), 10, _scripting);
                            }
                        }
                    }
                    let _ = world.despawn(entity);
                    broadcast_to_all(world, &InternalPacket::RemoveEntities { entity_ids: vec![eid] });
                    for (_, tracked) in world.query_mut::<&mut TrackedEntities>() {
                        tracked.visible.remove(&eid);
                    }
                }
            }
        }
    }

    // --- Mob combat: melee attacks, skeleton arrows, creeper fuse ---

    // Collect melee attacks from all melee hostiles (zombie, spider, enderman, slime)
    struct MeleeAttack {
        target: hecs::Entity,
        mob_type: i32,
        mob_pos: Vec3d,
    }
    let mut melee_attacks: Vec<MeleeAttack> = Vec::new();

    // Collect skeleton ranged attacks
    struct RangedAttack {
        target: hecs::Entity,
        mob_entity: hecs::Entity,
        mob_pos: Vec3d,
    }
    let mut ranged_attacks: Vec<RangedAttack> = Vec::new();

    // Collect creeper fuse updates
    struct CreeperFuse {
        mob_entity: hecs::Entity,
        mob_eid: i32,
        mob_pos: Vec3d,
        fuse_timer: i32,
    }
    let mut creeper_fuses: Vec<CreeperFuse> = Vec::new();

    for (entity, (eid, pos, mob)) in world.query::<(&EntityId, &Position, &MobEntity)>().iter() {
        let Some(target) = mob.target else { continue };
        let Ok(tp) = world.get::<&Position>(target) else { continue };

        let dx = tp.0.x - pos.0.x;
        let dy = tp.0.y - pos.0.y;
        let dz = tp.0.z - pos.0.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();

        match mob.mob_type {
            // Creeper: start fuse when close, explode at 0
            t if t == pickaxe_data::MOB_CREEPER => {
                if dist < 3.0 {
                    // Start or continue fuse
                    let new_fuse = if mob.fuse_timer < 0 { 30 } else { mob.fuse_timer - 1 };
                    creeper_fuses.push(CreeperFuse {
                        mob_entity: entity,
                        mob_eid: eid.0,
                        mob_pos: pos.0,
                        fuse_timer: new_fuse,
                    });
                } else if mob.fuse_timer >= 0 {
                    // Player moved away — cancel fuse
                    creeper_fuses.push(CreeperFuse {
                        mob_entity: entity,
                        mob_eid: eid.0,
                        mob_pos: pos.0,
                        fuse_timer: -1, // reset
                    });
                }
            }
            // Skeleton: ranged attack every 40 ticks when in range
            t if t == pickaxe_data::MOB_SKELETON => {
                if dist < 16.0 && mob.attack_cooldown == 0 {
                    ranged_attacks.push(RangedAttack {
                        target,
                        mob_entity: entity,
                        mob_pos: pos.0,
                    });
                }
            }
            // All other melee hostiles: zombie, spider, enderman, slime
            _ => {
                if !pickaxe_data::mob_is_hostile(mob.mob_type) { continue; }
                if mob.no_damage_ticks > 0 { continue; }
                if mob.attack_cooldown > 0 { continue; }
                if dist < 1.8 {
                    melee_attacks.push(MeleeAttack {
                        target,
                        mob_type: mob.mob_type,
                        mob_pos: pos.0,
                    });
                }
            }
        }
    }

    // Decrement attack cooldowns
    for (_e, mob) in world.query::<&mut MobEntity>().iter() {
        if mob.attack_cooldown > 0 {
            mob.attack_cooldown -= 1;
        }
    }

    // Process melee attacks
    for attack in melee_attacks {
        let damage = pickaxe_data::mob_attack_damage(attack.mob_type);
        let mob_name = pickaxe_data::mob_type_name(attack.mob_type).unwrap_or("mob");
        let target_eid = world.get::<&EntityId>(attack.target).map(|e| e.0).unwrap_or(0);
        apply_damage(world, world_state, attack.target, target_eid, damage, mob_name, _scripting);

        // Apply knockback to target player (vanilla: 0.4 strength)
        if let Ok(target_sender) = world.get::<&ConnectionSender>(attack.target) {
            let target_on_ground = world.get::<&OnGround>(attack.target).map(|og| og.0).unwrap_or(true);
            let target_pos = world.get::<&Position>(attack.target).map(|p| p.0).unwrap_or(Vec3d::new(0.0, 0.0, 0.0));
            // Direction: from mob toward target
            let dx = target_pos.x - attack.mob_pos.x;
            let dz = target_pos.z - attack.mob_pos.z;
            let dist = (dx * dx + dz * dz).sqrt().max(0.01);
            let dir_x = dx / dist;
            let dir_z = dz / dist;
            let kb = 0.4_f64;
            let new_vx = -dir_x * kb;
            let new_vy = if target_on_ground { 0.4_f64.min(kb) } else { 0.0 };
            let new_vz = -dir_z * kb;
            let _ = target_sender.0.send(InternalPacket::SetEntityVelocity {
                entity_id: target_eid,
                velocity_x: (new_vx.clamp(-3.9, 3.9) * 8000.0) as i16,
                velocity_y: (new_vy.clamp(-3.9, 3.9) * 8000.0) as i16,
                velocity_z: (new_vz.clamp(-3.9, 3.9) * 8000.0) as i16,
            });
        }

        // Play attack sound
        let (_, hurt_sound, _) = pickaxe_data::mob_sounds(attack.mob_type);
        let attack_sound = match attack.mob_type {
            t if t == pickaxe_data::MOB_ZOMBIE => "entity.zombie.attack",
            t if t == pickaxe_data::MOB_SPIDER => "entity.spider.hurt",
            t if t == pickaxe_data::MOB_ENDERMAN => "entity.enderman.hurt",
            _ => hurt_sound,
        };
        play_sound_at_entity(world, attack.mob_pos.x, attack.mob_pos.y, attack.mob_pos.z, attack_sound, SOUND_HOSTILE, 1.0, 1.0);

        // Set attack cooldown (20 ticks = 1 second between attacks)
        // Find the mob entity to set cooldown (search by position match)
        for (_e, (pos, mob)) in world.query::<(&Position, &mut MobEntity)>().iter() {
            if (pos.0.x - attack.mob_pos.x).abs() < 0.01 && (pos.0.z - attack.mob_pos.z).abs() < 0.01 {
                mob.attack_cooldown = 20;
                break;
            }
        }
    }

    // Process skeleton ranged attacks — spawn arrow entities
    for attack in ranged_attacks {
        let target_pos = match world.get::<&Position>(attack.target) {
            Ok(p) => p.0,
            Err(_) => continue,
        };

        // Calculate velocity toward target with some randomness
        let dx = target_pos.x - attack.mob_pos.x;
        let dy = (target_pos.y + 1.0) - (attack.mob_pos.y + 1.5); // aim at body, fire from eye
        let dz = target_pos.z - attack.mob_pos.z;
        let dist = (dx * dx + dz * dz).sqrt();
        let speed = 1.6; // skeleton arrow speed
        let norm = (dx * dx + dy * dy + dz * dz).sqrt().max(0.1);
        // Add arc: extra Y velocity for distance
        let arc_y = dist * 0.2 * 0.05;
        let vx = (dx / norm) * speed;
        let vy = (dy / norm) * speed + arc_y;
        let vz = (dz / norm) * speed;

        let mut rng = rand::thread_rng();
        let spread = 0.05;
        let vx = vx + rng.gen_range(-spread..spread);
        let vy = vy + rng.gen_range(-spread..spread);
        let vz = vz + rng.gen_range(-spread..spread);

        let damage = pickaxe_data::mob_attack_damage(pickaxe_data::MOB_SKELETON);
        spawn_arrow(
            world, next_eid,
            attack.mob_pos.x, attack.mob_pos.y + 1.5, attack.mob_pos.z,
            vx, vy, vz,
            damage,
            Some(attack.mob_entity),
            false, // not critical
            false, // not from player
        );
        play_sound_at_entity(world, attack.mob_pos.x, attack.mob_pos.y, attack.mob_pos.z, "entity.skeleton.shoot", SOUND_HOSTILE, 1.0, 1.0);
        // Set cooldown
        if let Ok(mut mob) = world.get::<&mut MobEntity>(attack.mob_entity) {
            mob.attack_cooldown = 40; // 2 seconds between arrows
        }
    }

    // Process creeper fuses
    let mut creeper_explosions: Vec<(hecs::Entity, i32, Vec3d)> = Vec::new();
    for fuse in &creeper_fuses {
        if let Ok(mut mob) = world.get::<&mut MobEntity>(fuse.mob_entity) {
            mob.fuse_timer = fuse.fuse_timer;
        }
        if fuse.fuse_timer == 30 {
            // Fuse just started — play fuse sound
            play_sound_at_entity(world, fuse.mob_pos.x, fuse.mob_pos.y, fuse.mob_pos.z, "entity.creeper.primed", SOUND_HOSTILE, 1.0, 1.0);
        }
        if fuse.fuse_timer == 0 {
            creeper_explosions.push((fuse.mob_entity, fuse.mob_eid, fuse.mob_pos));
        }
    }

    // Process creeper explosions
    for (creeper_entity, creeper_eid, creeper_pos) in creeper_explosions {
        // Despawn the creeper first
        let _ = world.despawn(creeper_entity);
        broadcast_to_all(world, &InternalPacket::RemoveEntities {
            entity_ids: vec![creeper_eid],
        });
        for (_, tracked) in world.query_mut::<&mut TrackedEntities>() {
            tracked.visible.remove(&creeper_eid);
        }

        // Creeper explosion: radius 3.0, destroys blocks
        do_explosion(
            world, world_state, next_eid, _scripting,
            creeper_pos.x, creeper_pos.y + 1.0, creeper_pos.z,
            3.0,
            true,
        );
    }
}

/// Periodically spawn mobs in loaded chunks near players.
fn tick_mob_spawning(
    world: &mut World,
    world_state: &WorldState,
    next_eid: &Arc<AtomicI32>,
    tick_count: u64,
) {
    // Only attempt spawning every 2 seconds (40 ticks)
    if tick_count % 40 != 0 {
        return;
    }

    // Count existing mobs
    let mob_count = world.query::<&MobEntity>().iter().count();
    let player_count = world.query::<&Profile>().iter().count();

    if player_count == 0 {
        return;
    }

    // Cap total mobs
    let max_mobs = player_count * 20;
    if mob_count >= max_mobs {
        return;
    }

    // Collect player positions
    let player_positions: Vec<Vec3d> = world.query::<(&Position, &Profile)>().iter()
        .map(|(_, (p, _))| p.0)
        .collect();

    // Try to spawn near a random player
    let player_pos = player_positions[rand::random::<usize>() % player_positions.len()];

    // Random offset 8-24 blocks from player
    let angle = rand::random::<f64>() * 2.0 * std::f64::consts::PI;
    let dist = 8.0 + rand::random::<f64>() * 16.0;
    let spawn_x = player_pos.x + angle.cos() * dist;
    let spawn_z = player_pos.z + angle.sin() * dist;

    // Find ground level (scan down from surface in flat world)
    let bx = spawn_x.floor() as i32;
    let bz = spawn_z.floor() as i32;
    let mut spawn_y = None;
    for y in (-60..=-45).rev() {
        let block = world_state.get_block_if_loaded(&BlockPos::new(bx, y, bz));
        let above = world_state.get_block_if_loaded(&BlockPos::new(bx, y + 1, bz));
        let above2 = world_state.get_block_if_loaded(&BlockPos::new(bx, y + 2, bz));
        if let (Some(b), Some(a1), Some(a2)) = (block, above, above2) {
            if b != 0 && a1 == 0 && a2 == 0 {
                spawn_y = Some(y + 1);
                break;
            }
        }
    }

    let spawn_y = match spawn_y {
        Some(y) => y as f64,
        None => return,
    };

    // Choose mob type based on time of day
    let is_night = {
        let time = world_state.time_of_day % 24000;
        time >= 13000 && time < 23000
    };

    let mob_type = if is_night && rand::random::<f32>() < 0.5 {
        // 50% chance of hostile mob at night
        let hostile_types = [
            pickaxe_data::MOB_ZOMBIE,
            pickaxe_data::MOB_SKELETON,
            pickaxe_data::MOB_SPIDER,
            pickaxe_data::MOB_CREEPER,
            pickaxe_data::MOB_ENDERMAN,
        ];
        hostile_types[rand::random::<usize>() % hostile_types.len()]
    } else {
        let passive_types = [
            pickaxe_data::MOB_PIG,
            pickaxe_data::MOB_COW,
            pickaxe_data::MOB_SHEEP,
            pickaxe_data::MOB_CHICKEN,
            pickaxe_data::MOB_BAT,
        ];
        passive_types[rand::random::<usize>() % passive_types.len()]
    };

    spawn_mob(world, next_eid, mob_type, spawn_x, spawn_y, spawn_z);
}

/// Despawn mobs that are too far from any player (>128 blocks).
fn tick_mob_despawn(world: &mut World) {
    let player_positions: Vec<Vec3d> = world.query::<(&Position, &Profile)>().iter()
        .map(|(_, (p, _))| p.0)
        .collect();

    if player_positions.is_empty() {
        return;
    }

    let mut to_despawn: Vec<(hecs::Entity, i32)> = Vec::new();
    for (entity, (eid, pos, _mob)) in world.query::<(&EntityId, &Position, &MobEntity)>().iter() {
        let min_dist = player_positions.iter()
            .map(|pp| {
                let dx = pp.x - pos.0.x;
                let dz = pp.z - pos.0.z;
                (dx * dx + dz * dz).sqrt()
            })
            .fold(f64::MAX, f64::min);

        if min_dist > 128.0 {
            to_despawn.push((entity, eid.0));
        }
    }

    for (entity, eid) in to_despawn {
        let _ = world.despawn(entity);
        broadcast_to_all(world, &InternalPacket::RemoveEntities {
            entity_ids: vec![eid],
        });
        for (_, tracked) in world.query_mut::<&mut TrackedEntities>() {
            tracked.visible.remove(&eid);
        }
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

    // Determine spawn point: use bed spawn if available and bed still exists
    let (spawn, spawn_yaw) = if let Ok(sp) = world.get::<&SpawnPoint>(entity) {
        let bed_block = world_state.get_block(&sp.position);
        if pickaxe_data::is_bed(bed_block) {
            // Spawn beside the bed (foot side)
            let facing = pickaxe_data::bed_facing(bed_block);
            let (dx, dz) = pickaxe_data::bed_head_offset(facing);
            let x = sp.position.x as f64 + 0.5 - dx as f64;
            let y = sp.position.y as f64 + 0.6;
            let z = sp.position.z as f64 + 0.5 - dz as f64;
            (Vec3d::new(x, y, z), sp.yaw)
        } else {
            // Bed destroyed — fall back to world spawn
            (Vec3d::new(0.5, -49.0, 0.5), 0.0)
        }
    } else {
        (Vec3d::new(0.5, -49.0, 0.5), 0.0)
    };

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
            yaw: spawn_yaw,
            pitch: 0.0,
            flags: 0,
            teleport_id: 100,
        });
        let _ = sender.0.send(InternalPacket::SetHealth {
            health: 20.0,
            food: 20,
            saturation: 5.0,
        });

        // Clear all active effects on respawn
        if let Ok(effects) = world.get::<&ActiveEffects>(entity) {
            let effect_ids: Vec<i32> = effects.effects.keys().copied().collect();
            for eff_id in effect_ids {
                let _ = sender.0.send(InternalPacket::RemoveMobEffect {
                    entity_id: _entity_id,
                    effect_id: eff_id,
                });
            }
        }
    }
    if let Ok(mut effects) = world.get::<&mut ActiveEffects>(entity) {
        effects.effects.clear();
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

fn tick_shield_cooldown(world: &mut World) {
    let mut expired: Vec<hecs::Entity> = Vec::new();
    for (e, cd) in world.query_mut::<&mut ShieldCooldown>() {
        if cd.remaining_ticks > 0 {
            cd.remaining_ticks -= 1;
        }
        if cd.remaining_ticks == 0 {
            expired.push(e);
        }
    }
    for e in expired {
        let _ = world.remove_one::<ShieldCooldown>(e);
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

/// Tick status effects: decrement durations, apply periodic effects, remove expired ones.
fn tick_effects(
    world: &mut World,
    world_state: &mut WorldState,
    scripting: &ScriptRuntime,
    tick_count: u64,
) {
    // Collect effect actions to apply outside the borrow
    struct EffectAction {
        entity: hecs::Entity,
        entity_id: i32,
        effect_id: i32,
        amplifier: i32,
    }
    let mut regen_actions: Vec<EffectAction> = Vec::new();
    let mut poison_actions: Vec<EffectAction> = Vec::new();
    let mut wither_actions: Vec<EffectAction> = Vec::new();
    let mut hunger_actions: Vec<(hecs::Entity, i32)> = Vec::new(); // (entity, amplifier)
    let mut expired: Vec<(hecs::Entity, i32, i32)> = Vec::new(); // (entity, eid, effect_id)
    let mut health_updates: Vec<(hecs::Entity, f32, i32, f32)> = Vec::new();

    for (entity, (eid, effects, health, food)) in world
        .query::<(&EntityId, &mut ActiveEffects, &Health, &FoodData)>()
        .iter()
    {
        if health.current <= 0.0 {
            continue;
        }

        let mut to_remove = Vec::new();
        for (effect_id, inst) in effects.effects.iter_mut() {
            // Decrement duration (skip infinite = -1)
            if inst.duration > 0 {
                inst.duration -= 1;
                if inst.duration == 0 {
                    to_remove.push(*effect_id);
                    continue;
                }
            }

            // Periodic effects based on tick timing
            let tick_val = if inst.duration == -1 { tick_count as i32 } else { inst.duration };
            match *effect_id {
                // Regeneration: heal every 50 >> amplifier ticks
                9 => {
                    let interval = (50 >> inst.amplifier.min(5)).max(1);
                    if tick_val % interval == 0 {
                        regen_actions.push(EffectAction {
                            entity, entity_id: eid.0, effect_id: *effect_id, amplifier: inst.amplifier,
                        });
                    }
                }
                // Poison: damage every 25 >> amplifier ticks (won't kill)
                18 => {
                    let interval = (25 >> inst.amplifier.min(5)).max(1);
                    if tick_val % interval == 0 {
                        poison_actions.push(EffectAction {
                            entity, entity_id: eid.0, effect_id: *effect_id, amplifier: inst.amplifier,
                        });
                    }
                }
                // Wither: damage every 40 >> amplifier ticks (can kill)
                19 => {
                    let interval = (40 >> inst.amplifier.min(5)).max(1);
                    if tick_val % interval == 0 {
                        wither_actions.push(EffectAction {
                            entity, entity_id: eid.0, effect_id: *effect_id, amplifier: inst.amplifier,
                        });
                    }
                }
                // Hunger: exhaustion every tick
                16 => {
                    hunger_actions.push((entity, inst.amplifier));
                }
                _ => {}
            }
        }

        for eff_id in to_remove {
            effects.effects.remove(&eff_id);
            expired.push((entity, eid.0, eff_id));
        }

        health_updates.push((entity, health.current, food.food_level, food.saturation));
    }

    // Apply regeneration (heal 1 HP)
    for action in &regen_actions {
        let max = world.get::<&Health>(action.entity).map(|h| h.max).unwrap_or(20.0);
        if let Ok(mut h) = world.get::<&mut Health>(action.entity) {
            h.current = (h.current + 1.0).min(max);
        }
    }

    // Apply poison (1 HP damage, won't go below 1 HP)
    for action in &poison_actions {
        if let Ok(mut h) = world.get::<&mut Health>(action.entity) {
            if h.current > 1.0 {
                h.current = (h.current - 1.0).max(1.0);
                h.invulnerable_ticks = 0; // poison bypasses invuln
            }
        }
        // Hurt animation
        broadcast_to_all(world, &InternalPacket::HurtAnimation {
            entity_id: action.entity_id,
            yaw: 0.0,
        });
    }

    // Apply wither (1 HP damage, can kill)
    for action in &wither_actions {
        apply_damage(world, world_state, action.entity, action.entity_id, 1.0, "wither", scripting);
    }

    // Apply hunger exhaustion
    for (entity, amplifier) in &hunger_actions {
        if let Ok(mut food) = world.get::<&mut FoodData>(*entity) {
            food.exhaustion = (food.exhaustion + 0.005 * (*amplifier as f32 + 1.0)).min(40.0);
        }
    }

    // Broadcast removal of expired effects
    for (entity, eid, effect_id) in &expired {
        if let Ok(sender) = world.get::<&ConnectionSender>(*entity) {
            let _ = sender.0.send(InternalPacket::RemoveMobEffect {
                entity_id: *eid,
                effect_id: *effect_id,
            });
        }
        // Fire Lua event
        let name = world.get::<&Profile>(*entity).map(|p| p.0.name.clone()).unwrap_or_default();
        let eff_name = pickaxe_data::effect_id_to_name(*effect_id).unwrap_or("unknown");
        scripting.fire_event_in_context(
            "effect_expire",
            &[("name", &name), ("effect", eff_name)],
            world as *mut _ as *mut (),
            world_state as *mut _ as *mut (),
        );
    }

    // Send health updates if any regen/poison/wither changed health
    if !regen_actions.is_empty() || !poison_actions.is_empty() || !wither_actions.is_empty() {
        for (entity, _, _, _) in &health_updates {
            let (h, f, s) = {
                let health = world.get::<&Health>(*entity).map(|h| h.current).unwrap_or(20.0);
                let (food, sat) = world.get::<&FoodData>(*entity)
                    .map(|f| (f.food_level, f.saturation))
                    .unwrap_or((20, 5.0));
                (health, food, sat)
            };
            if let Ok(sender) = world.get::<&ConnectionSender>(*entity) {
                let _ = sender.0.send(InternalPacket::SetHealth {
                    health: h,
                    food: f,
                    saturation: s,
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

        let is_potion = sat_mod < 0.0;

        if is_potion {
            // Potion drinking completion — nutrition stores potion type index
            let potion_index = nutrition;
            let effects = pickaxe_data::potion_effects(potion_index);
            let eid = world.get::<&EntityId>(entity).map(|e| e.0).unwrap_or(0);

            for eff in &effects {
                // Handle instant effects directly
                match eff.effect_id {
                    5 => { // instant_health
                        let heal = 4.0 * (1 << eff.amplifier.min(30)) as f32;
                        let max = world.get::<&Health>(entity).map(|h| h.max).unwrap_or(20.0);
                        if let Ok(mut h) = world.get::<&mut Health>(entity) {
                            h.current = (h.current + heal).min(max);
                        }
                    }
                    6 => { // instant_damage
                        let damage = 6.0 * (1 << eff.amplifier.min(30)) as f32;
                        if let Ok(mut h) = world.get::<&mut Health>(entity) {
                            h.current = (h.current - damage).max(0.0);
                        }
                    }
                    22 => { // saturation
                        if let Ok(mut food) = world.get::<&mut FoodData>(entity) {
                            food.food_level = (food.food_level + eff.amplifier + 1).min(20);
                            food.saturation = (food.saturation + (eff.amplifier + 1) as f32).min(food.food_level as f32);
                        }
                    }
                    _ => {
                        // Duration-based effect: add to ActiveEffects + send packet
                        let inst = EffectInstance {
                            effect_id: eff.effect_id,
                            amplifier: eff.amplifier,
                            duration: eff.duration,
                            ambient: false,
                            show_particles: true,
                            show_icon: true,
                        };
                        let flags: u8 = 0x02 | 0x04; // visible + show_icon
                        if let Ok(mut active) = world.get::<&mut ActiveEffects>(entity) {
                            active.effects.insert(eff.effect_id, inst);
                        }
                        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                            let _ = sender.0.send(InternalPacket::UpdateMobEffect {
                                entity_id: eid,
                                effect_id: eff.effect_id,
                                amplifier: eff.amplifier,
                                duration: eff.duration,
                                flags,
                            });
                        }
                    }
                }
            }
        } else {
            // Normal food — apply food restoration
            if let Ok(mut food) = world.get::<&mut FoodData>(entity) {
                food.food_level = (food.food_level + nutrition).min(20);
                let sat_gain = nutrition as f32 * sat_mod * 2.0;
                food.saturation = (food.saturation + sat_gain).min(food.food_level as f32);
            }
        }

        // Consume the item from the hand slot
        let held_slot = world
            .get::<&HeldSlot>(entity)
            .map(|h| h.0)
            .unwrap_or(0);
        let slot_idx = if hand == 1 { 45 } else { 36 + held_slot as usize };
        let new_slot_item = if is_potion {
            // Potions: replace with glass_bottle (or remove if stack > 1 and add bottle elsewhere)
            let glass_bottle_id = pickaxe_data::item_name_to_id("glass_bottle").unwrap_or(0);
            let mut inv = match world.get::<&mut Inventory>(entity) {
                Ok(inv) => inv,
                Err(_) => continue,
            };
            if let Some(ref mut item) = inv.slots[slot_idx] {
                if item.item_id == item_id {
                    if item.count <= 1 {
                        // Replace directly with glass bottle
                        inv.slots[slot_idx] = Some(ItemStack {
                            item_id: glass_bottle_id,
                            count: 1,
                            damage: 0,
                            max_damage: 0,
                            enchantments: Vec::new(),
                        });
                    } else {
                        // Decrement potion stack, put glass bottle elsewhere
                        item.count -= 1;
                        // Try to add glass bottle to inventory
                        let bottle = ItemStack {
                            item_id: glass_bottle_id,
                            count: 1,
                            damage: 0,
                            max_damage: 0,
                            enchantments: Vec::new(),
                        };
                        if let Some(target) = inv.find_slot_for_item(glass_bottle_id, 64) {
                            if let Some(ref mut existing) = inv.slots[target] {
                                existing.count += 1;
                            } else {
                                inv.slots[target] = Some(bottle);
                            }
                        }
                    }
                }
            }
            inv.state_id = inv.state_id.wrapping_add(1);
            (inv.slots[slot_idx].clone(), inv.state_id)
        } else {
            // Normal food: just decrement count
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

            // Update redstone neighbors when button resets
            update_redstone_neighbors(world, world_state, &position);
        }
    }
}

/// Tick drowning and lava damage for all players.
/// Checks eye position for water submersion (eye at Y + 1.62).
/// Air decreases 1/tick when submerged, deals 2 HP every 20 ticks (at air == -20).
/// Air recovers +4/tick when not submerged.
fn tick_drowning_and_lava(
    world: &mut World,
    world_state: &mut WorldState,
    scripting: &ScriptRuntime,
) {
    // Collect player data for fluid checks
    struct FluidCheck {
        entity: hecs::Entity,
        eid: i32,
        pos: Vec3d,
        game_mode: GameMode,
    }
    let mut checks: Vec<FluidCheck> = Vec::new();
    for (entity, (eid, pos, gm, _profile)) in world
        .query::<(&EntityId, &Position, &PlayerGameMode, &Profile)>()
        .iter()
    {
        checks.push(FluidCheck {
            entity,
            eid: eid.0,
            pos: pos.0,
            game_mode: gm.0,
        });
    }

    let mut drown_damage: Vec<(hecs::Entity, i32)> = Vec::new();
    let mut lava_damage: Vec<(hecs::Entity, i32)> = Vec::new();
    let mut fire_damage: Vec<(hecs::Entity, i32, bool)> = Vec::new(); // entity, eid, is_soul_fire
    let mut air_updates: Vec<(hecs::Entity, i32, i32)> = Vec::new(); // entity, eid, new_air

    for check in &checks {
        if check.game_mode == GameMode::Creative || check.game_mode == GameMode::Spectator {
            continue;
        }

        // Check if player's eye is in water (eye at Y + 1.62)
        let eye_y = check.pos.y + 1.62;
        let eye_block_pos = BlockPos::new(
            check.pos.x.floor() as i32,
            eye_y.floor() as i32,
            check.pos.z.floor() as i32,
        );
        let eye_block = world_state.get_block(&eye_block_pos);
        let eye_in_water = if pickaxe_data::is_water(eye_block) {
            let fluid_top = eye_block_pos.y as f64 + pickaxe_data::fluid_height(eye_block);
            fluid_top > eye_y
        } else {
            false
        };

        // Check if player is standing in lava (feet level)
        let feet_block_pos = BlockPos::new(
            check.pos.x.floor() as i32,
            check.pos.y.floor() as i32,
            check.pos.z.floor() as i32,
        );
        let feet_block = world_state.get_block(&feet_block_pos);
        let in_lava = pickaxe_data::is_lava(feet_block);

        // Check for water_breathing and fire_resistance effects
        let has_water_breathing = world.get::<&ActiveEffects>(check.entity)
            .map(|e| e.effects.contains_key(&12))
            .unwrap_or(false);
        let has_fire_resistance = world.get::<&ActiveEffects>(check.entity)
            .map(|e| e.effects.contains_key(&11))
            .unwrap_or(false);

        // Handle air supply
        if let Ok(mut air) = world.get::<&mut AirSupply>(check.entity) {
            let old_air = air.current;

            if eye_in_water && !has_water_breathing {
                air.current -= 1;
                if air.current == -20 {
                    air.current = 0;
                    drown_damage.push((check.entity, check.eid));
                }
            } else if air.current < air.max {
                air.current = (air.current + 4).min(air.max);
            }

            if air.current != old_air {
                air_updates.push((check.entity, check.eid, air.current));
            }
        }

        // Lava damage every 10 ticks (0.5s), unless fire_resistance
        if in_lava && !has_fire_resistance {
            lava_damage.push((check.entity, check.eid));
        }

        // Fire damage: check if player is standing in fire
        if !has_fire_resistance {
            let feet_block = world_state.get_block(&feet_block_pos);
            if pickaxe_data::is_fire(feet_block) {
                let is_soul = feet_block == pickaxe_data::SOUL_FIRE_STATE;
                fire_damage.push((check.entity, check.eid, is_soul));
            }
        }
    }

    // Apply drown damage (2 HP)
    for (entity, eid) in drown_damage {
        apply_damage(world, world_state, entity, eid, 2.0, "drowning", scripting);
    }

    // Apply lava damage (4 HP, but only every 10 ticks to avoid instant death)
    for (entity, eid) in lava_damage {
        let invuln = world.get::<&Health>(entity).map(|h| h.invulnerable_ticks > 0).unwrap_or(false);
        if !invuln {
            apply_damage(world, world_state, entity, eid, 4.0, "lava", scripting);
        }
    }

    // Apply fire damage (1 HP for fire, 2 HP for soul fire)
    for (entity, eid, is_soul) in fire_damage {
        let invuln = world.get::<&Health>(entity).map(|h| h.invulnerable_ticks > 0).unwrap_or(false);
        if !invuln {
            let dmg = if is_soul { 2.0 } else { 1.0 };
            apply_damage(world, world_state, entity, eid, dmg, "fire", scripting);
        }
    }

    // Send air supply metadata to clients
    for (entity, eid, air) in air_updates {
        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
            use pickaxe_protocol_core::EntityMetadataEntry;
            let mut data = Vec::new();
            pickaxe_protocol_core::write_varint_vec(&mut data, air);
            let _ = sender.0.send(InternalPacket::SetEntityMetadata {
                entity_id: eid,
                metadata: vec![EntityMetadataEntry {
                    index: 1,    // DATA_AIR_SUPPLY_ID
                    type_id: 1,  // VarInt
                    data,
                }],
            });
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

    // Collect all mob entities
    struct MobData {
        eid: i32,
        uuid: Uuid,
        pos: Vec3d,
        yaw: f32,
        pitch: f32,
        mob_type: i32,
    }
    let mut mob_data: Vec<MobData> = Vec::new();
    for (_e, (eid, euuid, pos, rot, mob)) in world
        .query::<(&EntityId, &EntityUuid, &Position, &Rotation, &MobEntity)>()
        .iter()
    {
        mob_data.push(MobData {
            eid: eid.0,
            uuid: euuid.0,
            pos: pos.0,
            yaw: rot.yaw,
            pitch: rot.pitch,
            mob_type: mob.mob_type,
        });
    }

    // Collect all arrow entities
    struct ArrowData {
        eid: i32,
        uuid: Uuid,
        pos: Vec3d,
        vel: Vec3d,
        yaw: f32,
        pitch: f32,
        owner_eid: i32, // 0 if no owner
        is_critical: bool,
    }
    let mut arrow_data: Vec<ArrowData> = Vec::new();
    for (_e, (eid, euuid, pos, vel, rot, arrow)) in world
        .query::<(&EntityId, &EntityUuid, &Position, &Velocity, &Rotation, &ArrowEntity)>()
        .iter()
    {
        let owner_eid = arrow.owner
            .and_then(|o| world.get::<&EntityId>(o).ok().map(|e| e.0))
            .unwrap_or(0);
        arrow_data.push(ArrowData {
            eid: eid.0,
            uuid: euuid.0,
            pos: pos.0,
            vel: vel.0,
            yaw: rot.yaw,
            pitch: rot.pitch,
            owner_eid,
            is_critical: arrow.is_critical,
        });
    }

    // Collect all fishing bobber entities
    struct BobberData {
        eid: i32,
        uuid: Uuid,
        pos: Vec3d,
        vel: Vec3d,
        owner_eid: i32,
    }
    let mut bobber_data: Vec<BobberData> = Vec::new();
    for (_e, (eid, euuid, pos, vel, bobber)) in world
        .query::<(&EntityId, &EntityUuid, &Position, &Velocity, &FishingBobber)>()
        .iter()
    {
        let owner_eid = world.get::<&EntityId>(bobber.owner)
            .ok().map(|e| e.0).unwrap_or(0);
        bobber_data.push(BobberData {
            eid: eid.0,
            uuid: euuid.0,
            pos: pos.0,
            vel: vel.0,
            owner_eid,
        });
    }

    // Collect all primed TNT entities
    struct TntData {
        eid: i32,
        uuid: Uuid,
        pos: Vec3d,
        vel: Vec3d,
        fuse: i32,
    }
    let mut tnt_data: Vec<TntData> = Vec::new();
    for (_e, (eid, euuid, pos, vel, tnt)) in world
        .query::<(&EntityId, &EntityUuid, &Position, &Velocity, &TntEntity)>()
        .iter()
    {
        tnt_data.push(TntData {
            eid: eid.0,
            uuid: euuid.0,
            pos: pos.0,
            vel: vel.0,
            fuse: tnt.fuse,
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

        // Mob entities in view distance
        for mob in &mob_data {
            let mob_cx = (mob.pos.x.floor() as i32) >> 4;
            let mob_cz = (mob.pos.z.floor() as i32) >> 4;
            if (mob_cx - obs_cx).abs() <= obs_vd && (mob_cz - obs_cz).abs() <= obs_vd {
                should_see.insert(mob.eid);
            }
        }

        // Arrow entities in view distance
        for arrow in &arrow_data {
            let arrow_cx = (arrow.pos.x.floor() as i32) >> 4;
            let arrow_cz = (arrow.pos.z.floor() as i32) >> 4;
            if (arrow_cx - obs_cx).abs() <= obs_vd && (arrow_cz - obs_cz).abs() <= obs_vd {
                should_see.insert(arrow.eid);
            }
        }

        // Fishing bobber entities in view distance
        for bobber in &bobber_data {
            let bobber_cx = (bobber.pos.x.floor() as i32) >> 4;
            let bobber_cz = (bobber.pos.z.floor() as i32) >> 4;
            if (bobber_cx - obs_cx).abs() <= obs_vd && (bobber_cz - obs_cz).abs() <= obs_vd {
                should_see.insert(bobber.eid);
            }
        }

        // TNT entities in view distance
        for tnt in &tnt_data {
            let tnt_cx = (tnt.pos.x.floor() as i32) >> 4;
            let tnt_cz = (tnt.pos.z.floor() as i32) >> 4;
            if (tnt_cx - obs_cx).abs() <= obs_vd && (tnt_cz - obs_cz).abs() <= obs_vd {
                should_see.insert(tnt.eid);
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
                // Send equipment (armor + held items)
                if let Some(&(target_entity, _, _, _, _, _, _, _, _)) =
                    player_data.iter().find(|d| d.1 == eid)
                {
                    let equipment = build_equipment(world, target_entity);
                    if !equipment.is_empty() {
                        let _ = observer_sender.send(InternalPacket::SetEquipment {
                            entity_id: eid,
                            equipment,
                        });
                    }
                }
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
            } else if let Some(mob) = mob_data.iter().find(|d| d.eid == eid) {
                // Mob entity
                let _ = observer_sender.send(InternalPacket::SpawnEntity {
                    entity_id: eid,
                    entity_uuid: mob.uuid,
                    entity_type: mob.mob_type,
                    x: mob.pos.x,
                    y: mob.pos.y,
                    z: mob.pos.z,
                    pitch: degrees_to_angle(mob.pitch),
                    yaw: degrees_to_angle(mob.yaw),
                    head_yaw: degrees_to_angle(mob.yaw),
                    data: 0,
                    velocity_x: 0,
                    velocity_y: 0,
                    velocity_z: 0,
                });
                let _ = observer_sender.send(InternalPacket::SetHeadRotation {
                    entity_id: eid,
                    head_yaw: degrees_to_angle(mob.yaw),
                });
            } else if let Some(arrow) = arrow_data.iter().find(|d| d.eid == eid) {
                // Arrow entity (type 4)
                let vx = (arrow.vel.x * 8000.0) as i16;
                let vy = (arrow.vel.y * 8000.0) as i16;
                let vz = (arrow.vel.z * 8000.0) as i16;
                // data field = shooter entity ID + 1 (0 means no owner)
                let data = if arrow.owner_eid > 0 { arrow.owner_eid + 1 } else { 0 };
                let _ = observer_sender.send(InternalPacket::SpawnEntity {
                    entity_id: eid,
                    entity_uuid: arrow.uuid,
                    entity_type: 4, // arrow
                    x: arrow.pos.x,
                    y: arrow.pos.y,
                    z: arrow.pos.z,
                    pitch: degrees_to_angle(arrow.pitch),
                    yaw: degrees_to_angle(arrow.yaw),
                    head_yaw: 0,
                    data,
                    velocity_x: vx,
                    velocity_y: vy,
                    velocity_z: vz,
                });
            } else if let Some(bobber) = bobber_data.iter().find(|d| d.eid == eid) {
                // Fishing bobber entity (type 129)
                let vx = (bobber.vel.x * 8000.0) as i16;
                let vy = (bobber.vel.y * 8000.0) as i16;
                let vz = (bobber.vel.z * 8000.0) as i16;
                // data field = owner entity ID
                let _ = observer_sender.send(InternalPacket::SpawnEntity {
                    entity_id: eid,
                    entity_uuid: bobber.uuid,
                    entity_type: 129, // fishing_bobber
                    x: bobber.pos.x,
                    y: bobber.pos.y,
                    z: bobber.pos.z,
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: bobber.owner_eid,
                    velocity_x: vx,
                    velocity_y: vy,
                    velocity_z: vz,
                });
            } else if let Some(tnt) = tnt_data.iter().find(|d| d.eid == eid) {
                // Primed TNT entity (type 106)
                let vx = (tnt.vel.x * 8000.0) as i16;
                let vy = (tnt.vel.y * 8000.0) as i16;
                let vz = (tnt.vel.z * 8000.0) as i16;
                let _ = observer_sender.send(InternalPacket::SpawnEntity {
                    entity_id: eid,
                    entity_uuid: tnt.uuid,
                    entity_type: pickaxe_data::ENTITY_TNT,
                    x: tnt.pos.x,
                    y: tnt.pos.y,
                    z: tnt.pos.z,
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                    velocity_x: vx,
                    velocity_y: vy,
                    velocity_z: vz,
                });
                // Send TNT metadata (fuse ticks + block state)
                let metadata = build_tnt_metadata(tnt.fuse, 2095); // default TNT block state
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

    // Collect mob entities that moved or rotated
    let mut mob_movers: Vec<(i32, Vec3d, Vec3d, f32, f32, f32, f32, bool)> = Vec::new();
    for (_e, (eid, pos, prev_pos, rot, prev_rot, og, _mob)) in world
        .query::<(
            &EntityId,
            &Position,
            &PreviousPosition,
            &Rotation,
            &PreviousRotation,
            &OnGround,
            &MobEntity,
        )>()
        .iter()
    {
        let pos_changed =
            pos.0.x != prev_pos.0.x || pos.0.y != prev_pos.0.y || pos.0.z != prev_pos.0.z;
        let rot_changed = rot.yaw != prev_rot.yaw || rot.pitch != prev_rot.pitch;
        if pos_changed || rot_changed {
            mob_movers.push((
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

    // Collect arrow entities that moved or rotated
    let mut arrow_movers: Vec<(i32, Vec3d, Vec3d, f32, f32, bool)> = Vec::new();
    for (_e, (eid, pos, prev_pos, rot, og, _arrow)) in world
        .query::<(&EntityId, &Position, &PreviousPosition, &Rotation, &OnGround, &ArrowEntity)>()
        .iter()
    {
        let pos_changed =
            pos.0.x != prev_pos.0.x || pos.0.y != prev_pos.0.y || pos.0.z != prev_pos.0.z;
        if pos_changed {
            arrow_movers.push((eid.0, pos.0, prev_pos.0, rot.yaw, rot.pitch, og.0));
        }
    }

    // Collect fishing bobber entities that moved
    let mut bobber_movers: Vec<(i32, Vec3d, Vec3d, bool)> = Vec::new();
    for (_e, (eid, pos, prev_pos, og, _bobber)) in world
        .query::<(&EntityId, &Position, &PreviousPosition, &OnGround, &FishingBobber)>()
        .iter()
    {
        let pos_changed =
            pos.0.x != prev_pos.0.x || pos.0.y != prev_pos.0.y || pos.0.z != prev_pos.0.z;
        if pos_changed {
            bobber_movers.push((eid.0, pos.0, prev_pos.0, og.0));
        }
    }

    // Collect TNT entities that moved
    let mut tnt_movers: Vec<(i32, Vec3d, Vec3d, bool)> = Vec::new();
    for (_e, (eid, pos, prev_pos, og, _tnt)) in world
        .query::<(&EntityId, &Position, &PreviousPosition, &OnGround, &TntEntity)>()
        .iter()
    {
        let pos_changed =
            pos.0.x != prev_pos.0.x || pos.0.y != prev_pos.0.y || pos.0.z != prev_pos.0.z;
        if pos_changed {
            tnt_movers.push((eid.0, pos.0, prev_pos.0, og.0));
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

    // For each item mover, always use teleport for exact positioning
    // (delta updates accumulate rounding errors, and client-side prediction diverges)
    for &(mover_eid, new_pos, _old_pos, on_ground) in &item_movers {
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

            let _ = sender.0.send(InternalPacket::TeleportEntity {
                entity_id: mover_eid,
                x: new_pos.x,
                y: new_pos.y,
                z: new_pos.z,
                yaw: 0,
                pitch: 0,
                on_ground,
            });
        }
    }

    // For each mob mover, send position+rotation updates
    for &(mover_eid, new_pos, old_pos, yaw, pitch, _old_yaw, _old_pitch, on_ground) in &mob_movers {
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

            let _ = sender.0.send(InternalPacket::SetHeadRotation {
                entity_id: mover_eid,
                head_yaw: degrees_to_angle(yaw),
            });
        }
    }

    // For each arrow mover, send position+rotation updates
    for &(mover_eid, new_pos, old_pos, yaw, pitch, on_ground) in &arrow_movers {
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
                    yaw: degrees_to_angle(yaw),
                    pitch: degrees_to_angle(pitch),
                    on_ground,
                });
            } else {
                let _ = sender.0.send(InternalPacket::UpdateEntityPositionAndRotation {
                    entity_id: mover_eid,
                    delta_x: dx,
                    delta_y: dy,
                    delta_z: dz,
                    yaw: degrees_to_angle(yaw),
                    pitch: degrees_to_angle(pitch),
                    on_ground,
                });
            }
        }
    }

    // For each bobber mover, send position-only updates (like items)
    for &(mover_eid, new_pos, old_pos, on_ground) in &bobber_movers {
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

    // For each TNT mover, send position-only updates (like items/bobbers)
    for &(mover_eid, new_pos, old_pos, on_ground) in &tnt_movers {
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

/// Advance the weather cycle. Matches vanilla MC logic:
/// - Rain/thunder timers count down, toggling state when they reach 0
/// - Rain/thunder levels transition gradually at ±0.01 per tick
/// - GameEvent packets are broadcast when levels change
fn tick_weather_cycle(world: &World, world_state: &mut WorldState, scripting: &ScriptRuntime) {
    let was_raining = world_state.raining;

    if world_state.clear_weather_time > 0 {
        world_state.clear_weather_time -= 1;
        world_state.thunder_time = if world_state.thundering { 0 } else { 1 };
        world_state.rain_time = if world_state.raining { 0 } else { 1 };
        world_state.thundering = false;
        world_state.raining = false;
    } else {
        // Thunder timer
        if world_state.thunder_time > 0 {
            world_state.thunder_time -= 1;
            if world_state.thunder_time == 0 {
                world_state.thundering = !world_state.thundering;
            }
        } else if world_state.thundering {
            // Duration: 3600-15600 ticks (3-13 minutes)
            world_state.thunder_time = 3600 + rand::random::<i32>().unsigned_abs() as i32 % 12000;
        } else {
            // Delay: 12000-180000 ticks (10-150 minutes)
            world_state.thunder_time = 12000 + rand::random::<i32>().unsigned_abs() as i32 % 168000;
        }

        // Rain timer
        if world_state.rain_time > 0 {
            world_state.rain_time -= 1;
            if world_state.rain_time == 0 {
                world_state.raining = !world_state.raining;
            }
        } else if world_state.raining {
            // Duration: 12000-24000 ticks (10-20 minutes)
            world_state.rain_time = 12000 + rand::random::<i32>().unsigned_abs() as i32 % 12000;
        } else {
            // Delay: 12000-180000 ticks (10-150 minutes)
            world_state.rain_time = 12000 + rand::random::<i32>().unsigned_abs() as i32 % 168000;
        }
    }

    // Gradual level transitions (±0.01 per tick, clamped to 0.0-1.0)
    let old_rain_level = world_state.rain_level;
    let old_thunder_level = world_state.thunder_level;

    if world_state.raining {
        world_state.rain_level = (world_state.rain_level + 0.01).min(1.0);
    } else {
        world_state.rain_level = (world_state.rain_level - 0.01).max(0.0);
    }

    if world_state.thundering {
        world_state.thunder_level = (world_state.thunder_level + 0.01).min(1.0);
    } else {
        world_state.thunder_level = (world_state.thunder_level - 0.01).max(0.0);
    }

    // Broadcast level changes
    if world_state.rain_level != old_rain_level {
        broadcast_to_all(world, &InternalPacket::GameEvent {
            event: 7, // RAIN_LEVEL_CHANGE
            value: world_state.rain_level,
        });
    }

    if world_state.thunder_level != old_thunder_level {
        broadcast_to_all(world, &InternalPacket::GameEvent {
            event: 8, // THUNDER_LEVEL_CHANGE
            value: world_state.thunder_level,
        });
    }

    // Start/stop rain events
    if was_raining != world_state.raining {
        if world_state.raining {
            broadcast_to_all(world, &InternalPacket::GameEvent {
                event: 1, // START_RAINING
                value: 0.0,
            });
        } else {
            broadcast_to_all(world, &InternalPacket::GameEvent {
                event: 2, // STOP_RAINING
                value: 0.0,
            });
        }

        // Also send current levels after state change
        broadcast_to_all(world, &InternalPacket::GameEvent {
            event: 7,
            value: world_state.rain_level,
        });
        broadcast_to_all(world, &InternalPacket::GameEvent {
            event: 8,
            value: world_state.thunder_level,
        });

        // Fire Lua event
        let weather = if world_state.thundering {
            "thunder"
        } else if world_state.raining {
            "rain"
        } else {
            "clear"
        };
        scripting.fire_event_in_context(
            "weather_change",
            &[("weather", weather)],
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
    }
}

/// Strike lightning at a position. Deals 5 damage to entities within 3 blocks.
fn strike_lightning(
    world: &mut World,
    world_state: &mut WorldState,
    next_eid: &Arc<AtomicI32>,
    x: f64,
    y: f64,
    z: f64,
    scripting: &ScriptRuntime,
) {
    // Spawn lightning bolt entity (type 120) briefly for visual effect
    let eid = next_eid.fetch_add(1, Ordering::Relaxed);
    let uuid = Uuid::new_v4();
    broadcast_to_all(world, &InternalPacket::SpawnEntity {
        entity_id: eid,
        entity_uuid: uuid,
        entity_type: 120, // lightning_bolt
        x,
        y,
        z,
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
        velocity_x: 0,
        velocity_y: 0,
        velocity_z: 0,
    });

    // Play thunder sound
    play_sound_at_entity(world, x, y, z, "entity.lightning_bolt.thunder", SOUND_WEATHER, 10000.0, 1.0);
    play_sound_at_entity(world, x, y, z, "entity.lightning_bolt.impact", SOUND_WEATHER, 2.0, 1.0);

    // Damage nearby entities (players and mobs) within 3 blocks
    let damage_radius = 3.0_f64;
    let lightning_damage = 5.0_f32;

    // Damage players
    let mut player_hits: Vec<(hecs::Entity, i32)> = Vec::new();
    for (entity, (entity_id, pos, _profile)) in world.query::<(&EntityId, &Position, &Profile)>().iter() {
        let dx = pos.0.x - x;
        let dy = pos.0.y - y;
        let dz = pos.0.z - z;
        if (dx * dx + dy * dy + dz * dz).sqrt() < damage_radius {
            player_hits.push((entity, entity_id.0));
        }
    }
    for (entity, entity_id) in player_hits {
        apply_damage(world, world_state, entity, entity_id, lightning_damage, "lightning", scripting);
    }

    // Damage mobs
    let mut mob_hits: Vec<(hecs::Entity, i32)> = Vec::new();
    for (entity, (entity_id, pos, _mob)) in world.query::<(&EntityId, &Position, &MobEntity)>().iter() {
        let dx = pos.0.x - x;
        let dy = pos.0.y - y;
        let dz = pos.0.z - z;
        if (dx * dx + dy * dy + dz * dz).sqrt() < damage_radius {
            mob_hits.push((entity, entity_id.0));
        }
    }
    for (entity, entity_id) in mob_hits {
        // Simple damage to mobs
        let died = {
            if let Ok(mut mob) = world.get::<&mut MobEntity>(entity) {
                mob.health -= lightning_damage;
                mob.health <= 0.0
            } else {
                false
            }
        };
        if died {
            let mob_type = world.get::<&MobEntity>(entity).map(|m| m.mob_type).unwrap_or(0);
            let (_, _, death_sound) = pickaxe_data::mob_sounds(mob_type);
            let mob_pos = world.get::<&Position>(entity).map(|p| p.0).unwrap_or(Vec3d::new(x, y, z));
            play_sound_at_entity(world, mob_pos.x, mob_pos.y, mob_pos.z, death_sound, SOUND_HOSTILE, 1.0, 1.0);
            broadcast_to_all(world, &InternalPacket::EntityEvent {
                entity_id,
                event_id: 3,
            });
            let _ = world.despawn(entity);
            broadcast_to_all(world, &InternalPacket::RemoveEntities {
                entity_ids: vec![entity_id],
            });
            for (_, tracked) in world.query_mut::<&mut TrackedEntities>() {
                tracked.visible.remove(&entity_id);
            }
        }
    }

    // Remove lightning entity after a brief delay (we'll use RemoveEntities immediately
    // since the client handles the visual effect duration itself)
    broadcast_to_all(world, &InternalPacket::RemoveEntities {
        entity_ids: vec![eid],
    });

    // Fire Lua event
    scripting.fire_event_in_context(
        "lightning_strike",
        &[
            ("x", &x.to_string()),
            ("y", &y.to_string()),
            ("z", &z.to_string()),
        ],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );
}

/// Tick thunderstorm lightning. During thunder, randomly strike near players.
fn tick_lightning(
    world: &mut World,
    world_state: &mut WorldState,
    next_eid: &Arc<AtomicI32>,
    scripting: &ScriptRuntime,
) {
    // Only during thunderstorms
    if world_state.thunder_level <= 0.0 {
        return;
    }

    // MC: 1 in 100000 chance per loaded chunk per tick.
    // Simplified: 1 in 2000 chance per tick (about once per 100s during thunder)
    if rand::random::<u32>() % 2000 != 0 {
        return;
    }

    // Pick a random player to strike near
    let player_positions: Vec<Vec3d> = world.query::<(&Position, &Profile)>().iter()
        .map(|(_, (p, _))| p.0)
        .collect();

    if player_positions.is_empty() {
        return;
    }

    let player_pos = player_positions[rand::random::<usize>() % player_positions.len()];

    // Random offset within 64 blocks
    let angle = rand::random::<f64>() * 2.0 * std::f64::consts::PI;
    let dist = rand::random::<f64>() * 64.0;
    let strike_x = player_pos.x + angle.cos() * dist;
    let strike_z = player_pos.z + angle.sin() * dist;

    // Find ground level (scan down from surface in flat world)
    let bx = strike_x.floor() as i32;
    let bz = strike_z.floor() as i32;
    let mut strike_y = -49.0; // default flat world surface
    for y in (-60..=-45).rev() {
        let block = world_state.get_block_if_loaded(&BlockPos::new(bx, y, bz));
        if let Some(b) = block {
            if b != 0 {
                strike_y = (y + 1) as f64;
                break;
            }
        }
    }

    strike_lightning(world, world_state, next_eid, strike_x, strike_y, strike_z, scripting);
}

/// Calculate how many ticks it takes to break a block in survival mode.
/// Returns None if the block is unbreakable, Some(0) for instant break, Some(ticks) otherwise.
/// Consults Lua block overrides before falling back to codegen data.
fn calculate_break_ticks(
    block_state: i32,
    held_item_id: Option<i32>,
    efficiency_level: i32,
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

    let mut seconds = if has_correct_tool {
        hardness * 1.5
    } else {
        hardness * 5.0
    };
    // Efficiency enchantment: reduce break time (level^2 + 1 speed bonus)
    if efficiency_level > 0 && has_correct_tool {
        let speed_bonus = (efficiency_level * efficiency_level + 1) as f64;
        // Base tool speed varies; approximate by reducing time proportionally
        seconds /= 1.0 + speed_bonus * 0.3;
    }
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

    // Special handling for beds: break other half and wake sleeping players
    if pickaxe_data::is_bed(old_block) {
        let facing = pickaxe_data::bed_facing(old_block);
        let (dx, dz) = pickaxe_data::bed_head_offset(facing);
        let other_pos = if pickaxe_data::bed_is_head(old_block) {
            // Broke head, remove foot
            BlockPos::new(position.x - dx, position.y, position.z - dz)
        } else {
            // Broke foot, remove head
            BlockPos::new(position.x + dx, position.y, position.z + dz)
        };
        let other_block = world_state.get_block(&other_pos);
        if pickaxe_data::is_bed(other_block) {
            world_state.set_block(&other_pos, 0);
            broadcast_to_all(world, &InternalPacket::BlockUpdate {
                position: other_pos,
                block_id: 0,
            });
        }
        // Wake any player sleeping in this bed
        let head_pos = if pickaxe_data::bed_is_head(old_block) {
            *position
        } else {
            other_pos
        };
        let mut sleepers_to_wake: Vec<(hecs::Entity, i32)> = Vec::new();
        for (e, (eid, sleep)) in world.query::<(&EntityId, &SleepingState)>().iter() {
            if sleep.bed_pos == head_pos {
                sleepers_to_wake.push((e, eid.0));
            }
        }
        for (e, eid) in sleepers_to_wake {
            wake_player(world, world_state, e, eid);
        }
    }

    // Special handling for pistons: breaking base removes head, breaking head removes base extension
    if pickaxe_data::is_any_piston(old_block) && pickaxe_data::piston_is_extended(old_block) {
        if let Some(facing) = pickaxe_data::piston_facing(old_block) {
            let (dx, dy, dz) = pickaxe_data::facing6_to_offset(facing);
            let head_pos = BlockPos::new(position.x + dx, position.y + dy, position.z + dz);
            let head_block = world_state.get_block(&head_pos);
            if pickaxe_data::is_piston_head(head_block) {
                world_state.set_block(&head_pos, 0);
                broadcast_to_all(world, &InternalPacket::BlockUpdate {
                    position: head_pos,
                    block_id: 0,
                });
            }
        }
    }
    if pickaxe_data::is_piston_head(old_block) {
        if let Some((facing, _, is_sticky)) = pickaxe_data::piston_head_props(old_block) {
            let (dx, dy, dz) = pickaxe_data::facing6_to_offset(facing);
            let base_pos = BlockPos::new(position.x - dx, position.y - dy, position.z - dz);
            let base_block = world_state.get_block(&base_pos);
            if pickaxe_data::is_any_piston(base_block) && pickaxe_data::piston_is_extended(base_block) {
                let retracted = pickaxe_data::piston_state(facing, false, is_sticky);
                world_state.set_block(&base_pos, retracted);
                broadcast_to_all(world, &InternalPacket::BlockUpdate {
                    position: base_pos,
                    block_id: retracted,
                });
            }
        }
    }

    // Mining exhaustion (MC: 0.005 per block broken)
    if let Ok(mut food) = world.get::<&mut FoodData>(entity) {
        food.exhaustion = (food.exhaustion + 0.005).min(40.0);
    }

    // Tool durability loss on block break (1 per block, survival only)
    let game_mode_for_durability = world.get::<&PlayerGameMode>(entity).map(|gm| gm.0).unwrap_or(GameMode::Survival);
    if game_mode_for_durability == GameMode::Survival {
        damage_held_item(world, entity, entity_id, 1);
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

    // Update redstone neighbors when a block is broken
    update_redstone_neighbors(world, world_state, position);

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
        // Handle crop drops specially
        if let Some((drop_name, drop_min, drop_max, seed_name, seed_min, seed_max)) = pickaxe_data::crop_drops(old_block) {
            let mut rng = rand::thread_rng();
            // Drop main item
            let count = rng.gen_range(drop_min..=drop_max);
            if count > 0 {
                if let Some(drop_id) = pickaxe_data::item_name_to_id(drop_name) {
                    spawn_item_entity(
                        world, world_state, next_eid,
                        position.x as f64 + 0.5, position.y as f64 + 0.25, position.z as f64 + 0.5,
                        ItemStack::new(drop_id, count as i8), 10, scripting,
                    );
                }
            }
            // Drop seeds (if applicable)
            if !seed_name.is_empty() && seed_max > 0 {
                let seed_count = rng.gen_range(seed_min..=seed_max);
                if seed_count > 0 {
                    if let Some(seed_id) = pickaxe_data::item_name_to_id(seed_name) {
                        spawn_item_entity(
                            world, world_state, next_eid,
                            position.x as f64 + 0.5, position.y as f64 + 0.25, position.z as f64 + 0.5,
                            ItemStack::new(seed_id, seed_count as i8), 10, scripting,
                        );
                    }
                }
            }
            // Apply exhaustion for mining
            if let Ok(mut food) = world.get::<&mut FoodData>(entity) {
                food.exhaustion += 0.005;
            }
            return; // Skip normal drop logic for crops
        }

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
            // Get fortune and silk touch levels from held item
            let (fortune_level, silk_touch) = {
                let slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
                if let Ok(inv) = world.get::<&Inventory>(entity) {
                    if let Some(ref item) = inv.held_item(slot) {
                        (item.enchantment_level(23), item.enchantment_level(21) > 0)
                    } else {
                        (0, false)
                    }
                } else {
                    (0, false)
                }
            };

            // Silk touch: drop the block itself instead of normal drops
            if silk_touch {
                if let Some(bn) = block_name {
                    if let Some(block_item_id) = pickaxe_data::item_name_to_id(bn) {
                        spawn_item_entity(
                            world, world_state, next_eid,
                            position.x as f64 + 0.5, position.y as f64 + 0.25, position.z as f64 + 0.5,
                            ItemStack::new(block_item_id, 1), 10, scripting,
                        );
                    }
                }
            } else {
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

                // Fortune: multiply ore drops (1 + random 0..=fortune_level)
                let fortune_multiplier = if fortune_level > 0 {
                    let mut rng = rand::thread_rng();
                    1 + rng.gen_range(0..=fortune_level)
                } else {
                    1
                };

                for &drop_item_id in &drop_ids {
                    let count = if fortune_level > 0 && is_ore_drop(drop_item_id) {
                        fortune_multiplier as i8
                    } else {
                        1
                    };
                    spawn_item_entity(
                        world,
                        world_state,
                        next_eid,
                        position.x as f64 + 0.5,
                        position.y as f64 + 0.25,
                        position.z as f64 + 0.5,
                        ItemStack::new(drop_item_id, count),
                        10, // pickup delay ticks
                        scripting,
                    );
                }
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
            BlockEntity::BrewingStand { bottles, ingredient, fuel, .. } => {
                let mut v: Vec<ItemStack> = bottles.into_iter().flatten().collect();
                v.extend(ingredient.into_iter());
                v.extend(fuel.into_iter());
                v
            }
            BlockEntity::Sign { .. } => Vec::new(), // Signs have no items to drop
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

        // Vanilla: skip physics when resting on ground with negligible horizontal velocity
        // (only processes every 4th tick for stationary items)
        let horiz_speed_sq = vel.0.x * vel.0.x + vel.0.z * vel.0.z;
        if og.0 && horiz_speed_sq < 1.0e-5 && vel.0.y.abs() < 0.001 {
            // Only process every 4th tick when stationary (vanilla: (tickCount + id) % 4 == 0)
            if (item_ent.age as i32 + eid.0) % 4 != 0 {
                continue;
            }
        }

        // 1. Apply gravity (vanilla: 0.04 per tick, applied before move)
        vel.0.y -= 0.04;

        // 2. Move with collision (simplified AABB collision)
        let old_vel = vel.0;
        let new_x = pos.0.x + vel.0.x;
        let new_y = pos.0.y + vel.0.y;
        let new_z = pos.0.z + vel.0.z;

        // Resolve Y collision (ground check)
        let mut resolved_y = new_y;
        let mut vertical_collision_below = false;
        let check_pos = BlockPos::new(
            new_x.floor() as i32,
            (new_y - 0.01).floor() as i32,
            new_z.floor() as i32,
        );
        let block_below = world_state.get_block(&check_pos);
        if block_below != 0 && vel.0.y < 0.0 {
            let ground_y = check_pos.y as f64 + 1.0;
            if new_y < ground_y {
                resolved_y = ground_y;
                vertical_collision_below = true;
            }
        }

        pos.0.x = new_x;
        pos.0.y = resolved_y;
        pos.0.z = new_z;

        // 3. Vanilla: setOnGroundWithMovement — set on_ground if vertical collision going down
        og.0 = vertical_collision_below;

        // 4. Vanilla: updateEntityAfterFallOn — zero Y velocity on ground hit
        if vertical_collision_below {
            vel.0.y = 0.0;
        }

        // 5. Vanilla: zero horizontal velocity on horizontal collision
        // (simplified — skip horizontal collision for items)

        // 6. Friction (vanilla: applied AFTER move)
        // Ground friction = block_friction * 0.98 (most blocks: 0.6 * 0.98 = 0.588)
        // Air friction = 0.98 for all axes
        let xz_friction = if og.0 { 0.6 * 0.98 } else { 0.98 };
        vel.0.x *= xz_friction;
        vel.0.y *= 0.98;
        vel.0.z *= xz_friction;

        // 7. Vanilla bounce check: if on ground and Y < 0, reverse and dampen
        // (After updateEntityAfterFallOn zeroed Y, and friction made it 0.0, this rarely triggers.
        //  It only triggers if something pushed the item downward while already on ground.)
        if og.0 && vel.0.y < 0.0 {
            vel.0.y *= -0.5;
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

/// Spawn an arrow entity in the world with given position and velocity.
fn spawn_arrow(
    world: &mut World,
    next_eid: &Arc<AtomicI32>,
    x: f64,
    y: f64,
    z: f64,
    vx: f64,
    vy: f64,
    vz: f64,
    damage: f32,
    owner: Option<hecs::Entity>,
    is_critical: bool,
    from_player: bool,
) -> (hecs::Entity, i32) {
    let eid = next_eid.fetch_add(1, Ordering::Relaxed);
    let uuid = Uuid::new_v4();

    // Calculate rotation from velocity
    let horiz = (vx * vx + vz * vz).sqrt();
    let yaw = (vz.atan2(vx).to_degrees() as f32) - 90.0;
    let pitch = -(vy.atan2(horiz).to_degrees() as f32);

    let entity = world.spawn((
        EntityId(eid),
        EntityUuid(uuid),
        Position(Vec3d::new(x, y, z)),
        PreviousPosition(Vec3d::new(x, y, z)),
        Velocity(Vec3d::new(vx, vy, vz)),
        OnGround(false),
        Rotation { yaw, pitch },
        PreviousRotation { yaw, pitch },
        ArrowEntity {
            damage,
            owner,
            in_ground: false,
            age: 0,
            is_critical,
            from_player,
        },
    ));

    (entity, eid)
}

/// Tick crop growth and farmland moisture. Runs every 68 ticks (~3.4 seconds) to approximate
/// MC's random tick system. Scans all loaded chunks for crops and farmland.
fn tick_farming(world: &World, world_state: &mut WorldState) {
    // Collect block updates to apply
    let mut updates: Vec<(BlockPos, i32)> = Vec::new();
    let mut rng = rand::thread_rng();

    // Get all loaded chunk positions
    let chunk_positions: Vec<pickaxe_types::ChunkPos> = world_state.chunks.keys().cloned().collect();

    for chunk_pos in chunk_positions {
        // Simulate random tick: 3 random blocks per chunk section per tick (MC default)
        // Since we run less often, check more blocks
        let chunk = match world_state.chunks.get(&chunk_pos) {
            Some(c) => c,
            None => continue,
        };

        // Check each section for crop blocks and farmland
        for section_y in 0..24 {
            let world_y = section_y as i32 * 16 - 64;
            // Random tick: pick 3 random blocks in this section
            for _ in 0..3 {
                let local_x = rng.gen_range(0..16);
                let local_y = rng.gen_range(0..16);
                let local_z = rng.gen_range(0..16);
                let by = world_y + local_y as i32;
                let block = chunk.get_block(local_x, by, local_z);

                if block == 0 { continue; }

                let bx = chunk_pos.x * 16 + local_x as i32;
                let bz = chunk_pos.z * 16 + local_z as i32;

                // Crop growth
                if let Some((age, max_age)) = pickaxe_data::crop_age(block) {
                    if age < max_age {
                        // Simplified growth: ~4% chance per random tick (f=1.0 equivalent)
                        // Check farmland below is present
                        let below = chunk.get_block(local_x, by - 1, local_z);
                        if pickaxe_data::is_farmland(below) {
                            // Higher chance if farmland is moist
                            let moisture = pickaxe_data::farmland_moisture(below).unwrap_or(0);
                            let growth_chance = if moisture >= 7 { 12 } else { 26 };
                            if rng.gen_range(0..growth_chance) == 0 {
                                if let Some(new_state) = pickaxe_data::crop_grow(block, 1) {
                                    updates.push((BlockPos::new(bx, by, bz), new_state));
                                }
                            }
                        }
                    }
                }

                // Farmland moisture
                if pickaxe_data::is_farmland(block) {
                    let moisture = pickaxe_data::farmland_moisture(block).unwrap_or(0);
                    // Check for water within 4 blocks horizontally, 1 vertically
                    let has_water = 'water: {
                        for wx in (bx - 4)..=(bx + 4) {
                            for wz in (bz - 4)..=(bz + 4) {
                                for wy in by..=(by + 1) {
                                    let wpos = BlockPos::new(wx, wy, wz);
                                    // Only check loaded chunks
                                    if let Some(wblock) = world_state.get_block_if_loaded(&wpos) {
                                        if pickaxe_data::is_water(wblock) {
                                            break 'water true;
                                        }
                                    }
                                }
                            }
                        }
                        false
                    };

                    if has_water {
                        if moisture < 7 {
                            updates.push((BlockPos::new(bx, by, bz), pickaxe_data::farmland_state(7)));
                        }
                    } else if moisture > 0 {
                        updates.push((BlockPos::new(bx, by, bz), pickaxe_data::farmland_state(moisture - 1)));
                    } else {
                        // Check if crop above maintains farmland
                        let above = chunk.get_block(local_x, by + 1, local_z);
                        if !pickaxe_data::is_crop(above) {
                            // Revert to dirt
                            updates.push((BlockPos::new(bx, by, bz), 10)); // dirt
                        }
                    }
                }
            }
        }
    }

    // Apply updates and broadcast
    for (pos, new_state) in updates {
        world_state.set_block(&pos, new_state);
        broadcast_to_all(world, &InternalPacket::BlockUpdate {
            position: pos,
            block_id: new_state,
        });
    }
}

/// Tick fire blocks: age progression, spread, burnout, block destruction.
/// Runs every 35 ticks (~1.75 seconds), simulating MC's 30-40 tick random delay.
fn tick_fire(
    world: &mut World,
    world_state: &mut WorldState,
    next_eid: &Arc<AtomicI32>,
    scripting: &ScriptRuntime,
) {
    use rand::Rng;
    let mut rng = rand::thread_rng();

    // Phase 1: Collect all fire block positions from chunks (immutable borrow)
    let mut fire_blocks: Vec<(BlockPos, i32)> = Vec::new(); // (pos, state_id)
    {
        let chunk_positions: Vec<pickaxe_types::ChunkPos> = world_state.chunks.keys().cloned().collect();
        for chunk_pos in chunk_positions {
            let chunk = match world_state.chunks.get(&chunk_pos) {
                Some(c) => c,
                None => continue,
            };
            for section_y in 0..24 {
                let world_y = section_y as i32 * 16 - 64;
                for local_x in 0..16usize {
                    for local_y in 0..16 {
                        for local_z in 0..16usize {
                            let by = world_y + local_y as i32;
                            let block = chunk.get_block(local_x, by, local_z);
                            if pickaxe_data::is_fire(block) && block != pickaxe_data::SOUL_FIRE_STATE {
                                let bx = chunk_pos.x * 16 + local_x as i32;
                                let bz = chunk_pos.z * 16 + local_z as i32;
                                fire_blocks.push((BlockPos::new(bx, by, bz), block));
                            }
                        }
                    }
                }
            }
        }
    }

    // Phase 2: Process each fire block (can use world_state.get_block freely)
    let mut updates: Vec<(BlockPos, i32)> = Vec::new();
    let mut tnt_ignitions: Vec<BlockPos> = Vec::new();

    for (fire_pos, block) in &fire_blocks {
        let age = pickaxe_data::fire_age(*block);
        let bx = fire_pos.x;
        let by = fire_pos.y;
        let bz = fire_pos.z;

        // Check if fire can survive: needs solid block below or adjacent flammable
        let below = BlockPos::new(bx, by - 1, bz);
        let below_block = world_state.get_block(&below);
        let below_solid = below_block != 0 && !pickaxe_data::is_fire(below_block);

        let has_fuel = {
            let offsets: [(i32,i32,i32); 6] = [(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)];
            offsets.iter().any(|(dx, dy, dz)| {
                let adj = BlockPos::new(bx + dx, by + dy, bz + dz);
                let adj_block = world_state.get_block(&adj);
                let adj_name = pickaxe_data::block_state_to_name(adj_block).unwrap_or("");
                pickaxe_data::is_flammable(adj_name)
            })
        };

        // Fire dies if no support and no fuel, or age 15 with no fuel
        if !below_solid && !has_fuel {
            updates.push((*fire_pos, 0));
            continue;
        }
        if age >= 15 && !has_fuel && rng.gen_range(0..4) == 0 {
            updates.push((*fire_pos, 0));
            continue;
        }

        // Age increment: +0 or +1
        let new_age = (age + rng.gen_range(0..3) / 2).min(15);
        if new_age != age {
            updates.push((*fire_pos, pickaxe_data::fire_state_with_age(new_age)));
        }

        // Burn adjacent flammable blocks (checkBurnOut equivalent)
        let direction_odds: [(i32,i32,i32,i32); 6] = [
            (1, 0, 0, 300),   // east
            (-1, 0, 0, 300),  // west
            (0, -1, 0, 250),  // below
            (0, 1, 0, 250),   // above
            (0, 0, 1, 300),   // south
            (0, 0, -1, 300),  // north
        ];

        for (dx, dy, dz, odds) in &direction_odds {
            let adj = BlockPos::new(bx + dx, by + dy, bz + dz);
            let adj_block = world_state.get_block(&adj);
            let adj_name = pickaxe_data::block_state_to_name(adj_block).unwrap_or("");
            let (_, burn_odds) = pickaxe_data::block_flammability(adj_name);
            if burn_odds > 0 && rng.gen_range(0..*odds) < burn_odds {
                if adj_name == "tnt" {
                    tnt_ignitions.push(adj);
                    updates.push((adj, 0)); // remove the TNT block
                } else if rng.gen_range(0..(new_age + 10)) < 5 {
                    // Replace with fire
                    let fire_age = (new_age + rng.gen_range(0..5) / 4).min(15);
                    updates.push((adj, pickaxe_data::fire_state_with_age(fire_age)));
                } else {
                    // Destroy block
                    updates.push((adj, 0));
                }
            }
        }

        // Fire spread to nearby air blocks adjacent to flammable blocks
        // Search 3x3x5 cube (x: -1 to 1, z: -1 to 1, y: -1 to 4)
        for sx in -1..=1i32 {
            for sz in -1..=1i32 {
                for sy in -1..=4i32 {
                    if sx == 0 && sy == 0 && sz == 0 {
                        continue;
                    }
                    let spread_pos = BlockPos::new(bx + sx, by + sy, bz + sz);
                    let spread_block = world_state.get_block(&spread_pos);
                    if spread_block != 0 {
                        continue; // must be air
                    }

                    // Check if any adjacent block to spread_pos is flammable
                    let mut max_ignite = 0i32;
                    let adj_offsets: [(i32,i32,i32); 6] = [(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)];
                    for (ax, ay, az) in &adj_offsets {
                        let check_pos = BlockPos::new(spread_pos.x + ax, spread_pos.y + ay, spread_pos.z + az);
                        let check_block = world_state.get_block(&check_pos);
                        let check_name = pickaxe_data::block_state_to_name(check_block).unwrap_or("");
                        let (ignite, _) = pickaxe_data::block_flammability(check_name);
                        max_ignite = max_ignite.max(ignite);
                    }

                    if max_ignite > 0 {
                        // Spread difficulty increases with height above fire
                        let mut difficulty = 100;
                        if sy > 1 {
                            difficulty += (sy - 1) * 100;
                        }

                        let spread_chance = (max_ignite + 40) / (new_age + 30);
                        if spread_chance > 0 && rng.gen_range(0..difficulty) <= spread_chance {
                            let fire_age = (new_age + rng.gen_range(0..5) / 4).min(15);
                            updates.push((spread_pos, pickaxe_data::fire_state_with_age(fire_age)));
                        }
                    }
                }
            }
        }
    }

    // Phase 3: Apply all updates and broadcast
    for (pos, new_state) in updates {
        world_state.set_block(&pos, new_state);
        broadcast_to_all(world, &InternalPacket::BlockUpdate {
            position: pos,
            block_id: new_state,
        });
        if new_state == 0 {
            play_sound_at_block(world, &pos, "block.fire.extinguish", SOUND_BLOCKS, 0.5, 1.0);
        }
    }

    // Phase 4: Chain-ignite TNT blocks that were burned by fire
    for pos in tnt_ignitions {
        let fuse = 80 + rng.gen_range(0..40);
        spawn_tnt_entity(
            world, world_state, next_eid,
            pos.x as f64 + 0.5,
            pos.y as f64,
            pos.z as f64 + 0.5,
            fuse,
            None,
            scripting,
        );
    }
}

/// Tick fluid blocks: water and lava flow, source creation, water-lava interactions.
/// Water ticks every 5 game ticks, lava every 30 game ticks.
fn tick_fluids(world: &World, world_state: &mut WorldState, do_water: bool, do_lava: bool) {
    // Phase 1: Collect all fluid block positions
    let mut fluid_blocks: Vec<(BlockPos, i32, bool)> = Vec::new(); // (pos, state, is_water)
    {
        let chunk_positions: Vec<pickaxe_types::ChunkPos> = world_state.chunks.keys().cloned().collect();
        for chunk_pos in chunk_positions {
            let chunk = match world_state.chunks.get(&chunk_pos) {
                Some(c) => c,
                None => continue,
            };
            for section_y in 0..24 {
                let world_y = section_y as i32 * 16 - 64;
                for local_x in 0..16usize {
                    for local_y in 0..16 {
                        for local_z in 0..16usize {
                            let by = world_y + local_y as i32;
                            let block = chunk.get_block(local_x, by, local_z);
                            if pickaxe_data::is_water(block) && do_water {
                                let bx = chunk_pos.x * 16 + local_x as i32;
                                let bz = chunk_pos.z * 16 + local_z as i32;
                                fluid_blocks.push((BlockPos::new(bx, by, bz), block, true));
                            } else if pickaxe_data::is_lava(block) && do_lava {
                                let bx = chunk_pos.x * 16 + local_x as i32;
                                let bz = chunk_pos.z * 16 + local_z as i32;
                                fluid_blocks.push((BlockPos::new(bx, by, bz), block, false));
                            }
                        }
                    }
                }
            }
        }
    }

    // Phase 2: For each fluid block, compute its new state
    let mut updates: Vec<(BlockPos, i32)> = Vec::new();

    // Also collect positions where NEW fluid should appear (flow targets)
    // We need to process both existing fluids AND check air neighbors for flow
    let mut flow_targets: Vec<(BlockPos, i32, bool)> = Vec::new(); // (pos, new_state, is_water)

    for (pos, state, is_water_fluid) in &fluid_blocks {
        let level = if *is_water_fluid {
            pickaxe_data::water_level(*state).unwrap_or(0)
        } else {
            pickaxe_data::lava_level(*state).unwrap_or(0)
        };
        let is_source = level == 0;
        let amount = if is_source { 8 } else if level >= 8 { 8 } else { 8 - level };
        let drop_off = if *is_water_fluid { 1 } else { 2 }; // water drops 1 per block, lava 2

        // Compute what this block SHOULD be based on neighbors
        let new_state = compute_new_fluid_state(world_state, pos, *is_water_fluid, drop_off);

        if new_state != *state {
            updates.push((*pos, new_state));
        }

        // Flow downward: check block below
        let below_pos = BlockPos::new(pos.x, pos.y - 1, pos.z);
        let below_block = world_state.get_block(&below_pos);
        let below_name = pickaxe_data::block_state_to_name(below_block).unwrap_or("");

        if below_block == 0 || pickaxe_data::is_fluid_destructible(below_name) {
            // Flow down: falling fluid (level 8)
            let falling_state = if *is_water_fluid {
                pickaxe_data::water_state_with_level(8)
            } else {
                pickaxe_data::lava_state_with_level(8)
            };
            flow_targets.push((below_pos, falling_state, *is_water_fluid));
        } else if !*is_water_fluid && pickaxe_data::is_water(below_block) {
            // Lava flowing down into water = stone
            updates.push((below_pos, pickaxe_data::block_name_to_default_state("stone").unwrap_or(1)));
        } else if below_block != 0 && !pickaxe_data::is_fluid(below_block) || pickaxe_data::is_fluid_source(below_block) {
            // Can't flow down — try horizontal spread
            if amount > 1 { // Only spread if we have enough fluid
                let new_level = if is_source { drop_off } else { level + drop_off };
                if new_level <= 7 {
                    let spread_state = if *is_water_fluid {
                        pickaxe_data::water_state_with_level(new_level)
                    } else {
                        pickaxe_data::lava_state_with_level(new_level)
                    };

                    // Find which directions to spread (prefer directions with drops)
                    let directions: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
                    let slope_dist = if *is_water_fluid { 4 } else { 2 };

                    // Find shortest path to a drop for each direction
                    let mut best_dist = slope_dist + 1;
                    let mut best_dirs: Vec<(i32, i32)> = Vec::new();

                    for (dx, dz) in &directions {
                        let adj = BlockPos::new(pos.x + dx, pos.y, pos.z + dz);
                        let adj_block = world_state.get_block(&adj);
                        let adj_name = pickaxe_data::block_state_to_name(adj_block).unwrap_or("");

                        // Can we flow into this block?
                        if adj_block != 0 && !pickaxe_data::is_fluid_destructible(adj_name)
                            && !pickaxe_data::is_fluid(adj_block)
                        {
                            if pickaxe_data::is_solid_for_fluid(adj_name) {
                                continue; // solid block, can't flow
                            }
                        }

                        // Check if there's a drop below the adjacent block
                        let adj_below = BlockPos::new(adj.x, adj.y - 1, adj.z);
                        let adj_below_block = world_state.get_block(&adj_below);
                        if adj_below_block == 0 || pickaxe_data::is_fluid_destructible(
                            pickaxe_data::block_state_to_name(adj_below_block).unwrap_or("")) {
                            // Direct drop found
                            if 0 < best_dist {
                                best_dist = 0;
                                best_dirs.clear();
                            }
                            if 0 <= best_dist {
                                best_dirs.push((*dx, *dz));
                            }
                        } else {
                            // Search further for drops (BFS up to slope_dist)
                            let dist = find_slope_distance(world_state, &adj, slope_dist, 1, *dx, *dz);
                            if dist < best_dist {
                                best_dist = dist;
                                best_dirs.clear();
                            }
                            if dist <= best_dist && dist <= slope_dist {
                                best_dirs.push((*dx, *dz));
                            }
                        }
                    }

                    // If no slope preference found, spread to all valid directions
                    if best_dirs.is_empty() {
                        for (dx, dz) in &directions {
                            let adj = BlockPos::new(pos.x + dx, pos.y, pos.z + dz);
                            let adj_block = world_state.get_block(&adj);
                            let adj_name = pickaxe_data::block_state_to_name(adj_block).unwrap_or("");
                            if (adj_block == 0 || pickaxe_data::is_fluid_destructible(adj_name))
                                && !pickaxe_data::is_solid_for_fluid(adj_name)
                            {
                                best_dirs.push((*dx, *dz));
                            }
                        }
                    }

                    for (dx, dz) in &best_dirs {
                        let adj = BlockPos::new(pos.x + dx, pos.y, pos.z + dz);
                        let adj_block = world_state.get_block(&adj);
                        let adj_name = pickaxe_data::block_state_to_name(adj_block).unwrap_or("");

                        // Don't overwrite with weaker flow
                        if pickaxe_data::is_water(adj_block) && *is_water_fluid {
                            let adj_level = pickaxe_data::water_level(adj_block).unwrap_or(0);
                            if adj_level != 0 && adj_level > new_level {
                                flow_targets.push((adj, spread_state, true));
                            }
                            continue;
                        }
                        if pickaxe_data::is_lava(adj_block) && !*is_water_fluid {
                            let adj_level = pickaxe_data::lava_level(adj_block).unwrap_or(0);
                            if adj_level != 0 && adj_level > new_level {
                                flow_targets.push((adj, spread_state, false));
                            }
                            continue;
                        }

                        // Water-lava interactions (horizontal)
                        if *is_water_fluid && pickaxe_data::is_lava(adj_block) {
                            let lava_level = pickaxe_data::lava_level(adj_block).unwrap_or(0);
                            if lava_level == 0 {
                                updates.push((adj, pickaxe_data::block_name_to_default_state("obsidian").unwrap_or(2346)));
                            } else {
                                updates.push((adj, pickaxe_data::block_name_to_default_state("cobblestone").unwrap_or(14)));
                            }
                            continue;
                        }
                        if !*is_water_fluid && pickaxe_data::is_water(adj_block) {
                            if is_source {
                                updates.push((adj, pickaxe_data::block_name_to_default_state("obsidian").unwrap_or(2346)));
                            } else {
                                updates.push((adj, pickaxe_data::block_name_to_default_state("cobblestone").unwrap_or(14)));
                            }
                            continue;
                        }

                        if adj_block == 0 || pickaxe_data::is_fluid_destructible(adj_name) {
                            flow_targets.push((adj, spread_state, *is_water_fluid));
                        }
                    }
                }
            }
        }
    }

    // Phase 3: Check for air blocks that should be empty (flowing fluid with no source)
    // This handles fluid retraction when source is removed
    // Already handled by compute_new_fluid_state returning air

    // Phase 4: Apply flow targets (new fluid placements)
    for (pos, state, _is_water) in &flow_targets {
        let existing = world_state.get_block(pos);
        let existing_name = pickaxe_data::block_state_to_name(existing).unwrap_or("");

        // Don't overwrite solid blocks or source blocks
        if pickaxe_data::is_solid_for_fluid(existing_name) && existing != 0 {
            continue;
        }
        if pickaxe_data::is_fluid_source(existing) {
            continue;
        }

        // Destroy blocks that water can break (flowers, torches, etc. just disappear)
        // In vanilla, some blocks drop items but we skip drops for simplicity

        // Only place if stronger than existing flow
        if pickaxe_data::is_fluid(existing) && !pickaxe_data::is_fluid_source(existing) {
            // Compare levels - only place if we're stronger
            let existing_level = if pickaxe_data::is_water(existing) {
                pickaxe_data::water_level(existing).unwrap_or(7)
            } else {
                pickaxe_data::lava_level(existing).unwrap_or(7)
            };
            let new_level = if pickaxe_data::is_water(*state) {
                pickaxe_data::water_level(*state).unwrap_or(7)
            } else {
                pickaxe_data::lava_level(*state).unwrap_or(7)
            };
            if new_level >= existing_level && existing_level < 8 {
                continue; // existing is already stronger
            }
        }

        updates.push((*pos, *state));
    }

    // Phase 5: Apply all updates and broadcast
    for (pos, new_state) in updates {
        let old = world_state.get_block(&pos);
        if old == new_state {
            continue;
        }
        world_state.set_block(&pos, new_state);
        broadcast_to_all(world, &InternalPacket::BlockUpdate {
            position: pos,
            block_id: new_state,
        });
    }
}

/// Compute what a fluid block at `pos` should become based on its neighbors.
/// Returns the new block state (may be same, different level, or air if drying up).
fn compute_new_fluid_state(
    world_state: &WorldState,
    pos: &BlockPos,
    is_water: bool,
    drop_off: i32,
) -> i32 {
    let current = world_state.get_block_if_loaded(pos).unwrap_or(0);
    let current_level = if is_water {
        pickaxe_data::water_level(current).unwrap_or(0)
    } else {
        pickaxe_data::lava_level(current).unwrap_or(0)
    };

    // Source blocks don't change
    if current_level == 0 {
        return current;
    }

    // Check above: if fluid above, this should be falling (level 8)
    let above = BlockPos::new(pos.x, pos.y + 1, pos.z);
    let above_block = world_state.get_block_if_loaded(&above).unwrap_or(0);
    let same_fluid_above = if is_water {
        pickaxe_data::is_water(above_block)
    } else {
        pickaxe_data::is_lava(above_block)
    };

    if same_fluid_above {
        return if is_water {
            pickaxe_data::water_state_with_level(8)
        } else {
            pickaxe_data::lava_state_with_level(8)
        };
    }

    // Check horizontal neighbors for sources / higher-level fluid
    let mut max_neighbor_amount = 0i32;
    let mut source_count = 0i32;
    let directions: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

    for (dx, dz) in &directions {
        let adj = BlockPos::new(pos.x + dx, pos.y, pos.z + dz);
        let adj_block = world_state.get_block_if_loaded(&adj).unwrap_or(0);
        let is_same = if is_water { pickaxe_data::is_water(adj_block) } else { pickaxe_data::is_lava(adj_block) };
        if is_same {
            let adj_amount = pickaxe_data::fluid_amount(adj_block);
            if pickaxe_data::is_fluid_source(adj_block) {
                source_count += 1;
            }
            max_neighbor_amount = max_neighbor_amount.max(adj_amount);
        }
    }

    // Infinite source creation (water only): 2+ source neighbors + solid below
    if is_water && source_count >= 2 {
        let below = BlockPos::new(pos.x, pos.y - 1, pos.z);
        let below_block = world_state.get_block_if_loaded(&below).unwrap_or(0);
        let below_name = pickaxe_data::block_state_to_name(below_block).unwrap_or("");
        if pickaxe_data::is_solid_for_fluid(below_name) || pickaxe_data::is_fluid_source(below_block) {
            return pickaxe_data::WATER_SOURCE;
        }
    }

    // Compute new level from neighbors
    let new_amount = max_neighbor_amount - drop_off;
    if new_amount <= 0 {
        return 0; // dry up (air)
    }

    let new_level = 8 - new_amount;
    if is_water {
        pickaxe_data::water_state_with_level(new_level)
    } else {
        pickaxe_data::lava_state_with_level(new_level)
    }
}

/// Find the slope distance (shortest path to a drop) in a given direction.
/// Returns the distance (1-max_dist) or max_dist+1 if no drop found.
fn find_slope_distance(
    world_state: &WorldState,
    pos: &BlockPos,
    max_dist: i32,
    current_dist: i32,
    from_dx: i32,
    from_dz: i32,
) -> i32 {
    if current_dist > max_dist {
        return max_dist + 1;
    }

    // Check if there's a drop here
    let below = BlockPos::new(pos.x, pos.y - 1, pos.z);
    let below_block = world_state.get_block_if_loaded(&below).unwrap_or(0);
    let below_name = pickaxe_data::block_state_to_name(below_block).unwrap_or("");
    if below_block == 0 || pickaxe_data::is_fluid_destructible(below_name) {
        return current_dist;
    }

    let mut min_dist = max_dist + 1;
    let directions: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

    for (dx, dz) in &directions {
        // Don't go backward
        if *dx == -from_dx && *dz == -from_dz {
            continue;
        }
        let adj = BlockPos::new(pos.x + dx, pos.y, pos.z + dz);
        let adj_block = world_state.get_block_if_loaded(&adj).unwrap_or(0);
        let adj_name = pickaxe_data::block_state_to_name(adj_block).unwrap_or("");

        if pickaxe_data::is_solid_for_fluid(adj_name) && adj_block != 0 {
            continue;
        }

        let dist = find_slope_distance(world_state, &adj, max_dist, current_dist + 1, *dx, *dz);
        min_dist = min_dist.min(dist);
    }

    min_dist
}

/// Spawn a fishing bobber entity.
fn spawn_fishing_bobber(
    world: &mut World,
    next_eid: &Arc<AtomicI32>,
    owner: hecs::Entity,
    owner_eid: i32,
    x: f64, y: f64, z: f64,
    vx: f64, vy: f64, vz: f64,
) -> (hecs::Entity, i32) {
    let eid = next_eid.fetch_add(1, Ordering::Relaxed);
    let uuid = Uuid::new_v4();

    let mut rng = rand::thread_rng();
    let time_until_lured = rng.gen_range(100..=600);

    let entity = world.spawn((
        EntityId(eid),
        EntityUuid(uuid),
        Position(Vec3d::new(x, y, z)),
        PreviousPosition(Vec3d::new(x, y, z)),
        Velocity(Vec3d::new(vx, vy, vz)),
        OnGround(false),
        Rotation { yaw: 0.0, pitch: 0.0 },
        PreviousRotation { yaw: 0.0, pitch: 0.0 },
        FishingBobber {
            owner,
            state: FishingBobberState::Flying,
            time_until_lured,
            time_until_hooked: 0,
            nibble: 0,
            age: 0,
            hooked_entity: None,
        },
    ));

    let _ = owner_eid; // used by entity tracking for spawn data

    (entity, eid)
}

/// Tick fishing bobber physics and state machine.
fn tick_fishing_bobbers(world: &mut World, world_state: &mut WorldState) {
    // Collect bobber updates
    struct BobberUpdate {
        entity: hecs::Entity,
        eid: i32,
    }
    let mut to_despawn: Vec<BobberUpdate> = Vec::new();

    // Phase 1: Update physics and state machine
    let mut bobber_updates: Vec<(hecs::Entity, i32)> = Vec::new();
    for (e, (eid, _bobber)) in world.query::<(&EntityId, &FishingBobber)>().iter() {
        bobber_updates.push((e, eid.0));
    }

    for (entity, eid) in &bobber_updates {
        let entity = *entity;
        let eid = *eid;

        // Check if owner still exists
        let owner_alive = {
            let owner = match world.get::<&FishingBobber>(entity) {
                Ok(b) => b.owner,
                Err(_) => continue,
            };
            world.get::<&Position>(owner).is_ok()
        };
        if !owner_alive {
            to_despawn.push(BobberUpdate { entity, eid });
            continue;
        }

        // Check distance to owner (max 1024 block squared = 32 blocks)
        let too_far = {
            let owner = world.get::<&FishingBobber>(entity).unwrap().owner;
            let bobber_pos = world.get::<&Position>(entity).ok().map(|p| p.0);
            let owner_pos = world.get::<&Position>(owner).ok().map(|p| p.0);
            if let (Some(bp), Some(op)) = (bobber_pos, owner_pos) {
                let dx = bp.x - op.x;
                let dy = bp.y - op.y;
                let dz = bp.z - op.z;
                (dx * dx + dy * dy + dz * dz) > 1024.0
            } else {
                true
            }
        };
        if too_far {
            to_despawn.push(BobberUpdate { entity, eid });
            continue;
        }

        // Increment age, despawn after 1200 ticks if on ground
        let should_despawn_age = {
            let mut bobber = match world.get::<&mut FishingBobber>(entity) {
                Ok(b) => b,
                Err(_) => continue,
            };
            bobber.age += 1;
            bobber.age > 1200 && world.get::<&OnGround>(entity).ok().map_or(false, |og| og.0)
        };
        if should_despawn_age {
            to_despawn.push(BobberUpdate { entity, eid });
            continue;
        }

        // Get current state
        let state = match world.get::<&FishingBobber>(entity) {
            Ok(b) => b.state,
            Err(_) => continue,
        };

        match state {
            FishingBobberState::Flying => {
                // Apply gravity and drag
                if let (Ok(mut pos), Ok(mut vel)) = (
                    world.get::<&mut Position>(entity),
                    world.get::<&mut Velocity>(entity),
                ) {
                    // Gravity
                    vel.0.y -= 0.03;
                    // Apply velocity
                    pos.0.x += vel.0.x;
                    pos.0.y += vel.0.y;
                    pos.0.z += vel.0.z;
                    // Drag
                    vel.0.x *= 0.92;
                    vel.0.y *= 0.92;
                    vel.0.z *= 0.92;
                }

                // Check if entered water
                let bobber_pos = world.get::<&Position>(entity).ok().map(|p| p.0);
                if let Some(bp) = bobber_pos {
                    let block_y = (bp.y - 0.1).floor() as i32;
                    let bx = bp.x.floor() as i32;
                    let bz = bp.z.floor() as i32;
                    let block = world_state.get_block(&BlockPos::new(bx, block_y, bz));
                    if pickaxe_data::is_water(block) {
                        // Transition to bobbing
                        if let Ok(mut bobber) = world.get::<&mut FishingBobber>(entity) {
                            bobber.state = FishingBobberState::Bobbing;
                        }
                        // Dampen velocity on water entry
                        if let Ok(mut vel) = world.get::<&mut Velocity>(entity) {
                            vel.0.x *= 0.3;
                            vel.0.y *= 0.2;
                            vel.0.z *= 0.3;
                        }
                    } else if block != 0 && bp.y <= (block_y + 1) as f64 {
                        // Hit solid ground
                        if let Ok(mut vel) = world.get::<&mut Velocity>(entity) {
                            vel.0.x = 0.0;
                            vel.0.y = 0.0;
                            vel.0.z = 0.0;
                        }
                        if let Ok(mut og) = world.get::<&mut OnGround>(entity) {
                            og.0 = true;
                        }
                    }
                }
            }
            FishingBobberState::Bobbing => {
                // Float on water surface
                let bobber_pos = world.get::<&Position>(entity).ok().map(|p| p.0);
                if let Some(bp) = bobber_pos {
                    let bx = bp.x.floor() as i32;
                    let bz = bp.z.floor() as i32;
                    // Find water surface
                    let water_y_check = bp.y.floor() as i32;
                    let block_at = world_state.get_block(&BlockPos::new(bx, water_y_check, bz));
                    let block_above = world_state.get_block(&BlockPos::new(bx, water_y_check + 1, bz));

                    if !pickaxe_data::is_water(block_at) && !pickaxe_data::is_water(world_state.get_block(&BlockPos::new(bx, water_y_check - 1, bz))) {
                        // Bobber left water — go back to flying
                        if let Ok(mut bobber) = world.get::<&mut FishingBobber>(entity) {
                            bobber.state = FishingBobberState::Flying;
                        }
                    } else {
                        // Float at water surface
                        let surface_y = if pickaxe_data::is_water(block_above) {
                            (water_y_check + 2) as f64 - 0.12
                        } else {
                            (water_y_check + 1) as f64 - 0.12
                        };

                        if let Ok(mut pos) = world.get::<&mut Position>(entity) {
                            // Gently move toward surface
                            let dy = surface_y - pos.0.y;
                            pos.0.y += dy * 0.2;
                        }

                        // Bobbing motion
                        if let Ok(mut vel) = world.get::<&mut Velocity>(entity) {
                            vel.0.y *= 0.9;
                            vel.0.x *= 0.9;
                            vel.0.z *= 0.9;
                        }

                        // Fish state machine
                        let mut rng = rand::thread_rng();
                        let bobber_data = {
                            let b = world.get::<&FishingBobber>(entity).unwrap();
                            (b.nibble, b.time_until_hooked, b.time_until_lured)
                        };

                        if bobber_data.0 > 0 {
                            // Nibbling phase — decrement
                            if let Ok(mut bobber) = world.get::<&mut FishingBobber>(entity) {
                                bobber.nibble -= 1;
                            }
                        } else if bobber_data.1 > 0 {
                            // Waiting for bite — decrement
                            if let Ok(mut bobber) = world.get::<&mut FishingBobber>(entity) {
                                bobber.time_until_hooked -= 1;
                                if bobber.time_until_hooked <= 0 {
                                    // Fish bites!
                                    bobber.nibble = rng.gen_range(20..=40);
                                    // Bobber dips down
                                    if let Ok(mut vel) = world.get::<&mut Velocity>(entity) {
                                        vel.0.y -= 0.2;
                                    }
                                    // Splash sound
                                    if let Some(bpos) = world.get::<&Position>(entity).ok().map(|p| p.0) {
                                        play_sound_at_entity(world, bpos.x, bpos.y, bpos.z,
                                            "entity.fishing_bobber.splash", SOUND_NEUTRAL, 0.25, 1.0);
                                    }
                                }
                            }
                        } else if bobber_data.2 > 0 {
                            // Luring phase — decrement
                            if let Ok(mut bobber) = world.get::<&mut FishingBobber>(entity) {
                                bobber.time_until_lured -= 1;
                                if bobber.time_until_lured <= 0 {
                                    // Luring done, start hooked timer
                                    bobber.time_until_hooked = rng.gen_range(20..=80);
                                }
                            }
                        }
                    }
                }
            }
            FishingBobberState::HookedInEntity => {
                // Follow hooked entity (not implemented for simplicity)
                // Despawn if hooked entity gone
                let hooked = world.get::<&FishingBobber>(entity).ok().and_then(|b| b.hooked_entity);
                if let Some(hooked_e) = hooked {
                    if world.get::<&Position>(hooked_e).is_err() {
                        to_despawn.push(BobberUpdate { entity, eid });
                    }
                }
            }
        }
    }

    // Despawn collected bobbers
    for bu in &to_despawn {
        let _ = world.despawn(bu.entity);
        broadcast_to_all(world, &InternalPacket::RemoveEntities {
            entity_ids: vec![bu.eid],
        });
        for (_e, tracked) in world.query::<&mut TrackedEntities>().iter() {
            tracked.visible.remove(&bu.eid);
        }
    }
}

/// Apply physics to arrow entities: gravity, drag, collision, despawn.
fn tick_arrow_physics(world: &mut World, world_state: &mut WorldState, next_eid: &Arc<AtomicI32>, scripting: &ScriptRuntime) {
    // Collect arrows to despawn
    let mut to_despawn: Vec<(hecs::Entity, i32)> = Vec::new();
    // Collect arrow-entity hits
    struct ArrowHit {
        arrow_entity: hecs::Entity,
        arrow_eid: i32,
        target_entity: hecs::Entity,
        target_eid: i32,
        damage: f32,
        hit_pos: Vec3d,
        from_player: bool,
        is_mob_target: bool,
        owner: Option<hecs::Entity>,
        is_critical: bool,
    }
    let mut entity_hits: Vec<ArrowHit> = Vec::new();

    // Collect all player positions for hit detection
    let mut player_positions: Vec<(hecs::Entity, i32, Vec3d, Option<hecs::Entity>)> = Vec::new();
    for (e, (eid, pos, _profile)) in world
        .query::<(&EntityId, &Position, &Profile)>()
        .iter()
    {
        player_positions.push((e, eid.0, pos.0, None));
    }

    // Collect all mob positions for hit detection
    let mut mob_positions: Vec<(hecs::Entity, i32, Vec3d)> = Vec::new();
    for (e, (eid, pos, _mob)) in world
        .query::<(&EntityId, &Position, &MobEntity)>()
        .iter()
    {
        mob_positions.push((e, eid.0, pos.0));
    }

    // Apply physics to arrows
    for (e, (eid, pos, vel, og, rot, arrow)) in world
        .query::<(&EntityId, &mut Position, &mut Velocity, &mut OnGround, &mut Rotation, &mut ArrowEntity)>()
        .iter()
    {
        arrow.age += 1;

        // Despawn after 1200 ticks (60 seconds)
        if arrow.age >= 1200 {
            to_despawn.push((e, eid.0));
            continue;
        }

        // If arrow is stuck in ground, just age it
        if arrow.in_ground {
            continue;
        }

        // Apply gravity (MC uses 0.05 for arrows)
        vel.0.y -= 0.05;

        // Move arrow
        let old_pos = pos.0;
        pos.0.x += vel.0.x;
        pos.0.y += vel.0.y;
        pos.0.z += vel.0.z;

        // Check entity collision (before block collision) — simple distance check
        // Check against players
        for &(target_e, target_eid, target_pos, _) in &player_positions {
            // Don't hit the shooter
            if arrow.owner == Some(target_e) {
                continue;
            }
            let dx = pos.0.x - target_pos.x;
            let dy = (pos.0.y - target_pos.y) - 0.9; // aim at center of body
            let dz = pos.0.z - target_pos.z;
            let dist_sq = dx * dx + dy * dy + dz * dz;
            if dist_sq < 0.6 * 0.6 {
                // Hit!
                let damage = if arrow.is_critical {
                    arrow.damage * 1.5 + 0.5
                } else {
                    arrow.damage
                };
                entity_hits.push(ArrowHit {
                    arrow_entity: e, arrow_eid: eid.0,
                    target_entity: target_e, target_eid,
                    damage, hit_pos: pos.0, from_player: arrow.from_player,
                    is_mob_target: false, owner: arrow.owner, is_critical: arrow.is_critical,
                });
                break;
            }
        }

        // Check against mobs
        if !entity_hits.iter().any(|h| h.arrow_entity == e) {
            for &(target_e, target_eid, target_pos) in &mob_positions {
                // Don't hit the shooter mob
                if arrow.owner == Some(target_e) {
                    continue;
                }
                let dx = pos.0.x - target_pos.x;
                let dy = (pos.0.y - target_pos.y) - 0.5;
                let dz = pos.0.z - target_pos.z;
                let dist_sq = dx * dx + dy * dy + dz * dz;
                if dist_sq < 0.8 * 0.8 {
                    let damage = if arrow.is_critical {
                        arrow.damage * 1.5 + 0.5
                    } else {
                        arrow.damage
                    };
                    entity_hits.push(ArrowHit {
                        arrow_entity: e, arrow_eid: eid.0,
                        target_entity: target_e, target_eid,
                        damage, hit_pos: pos.0, from_player: arrow.from_player,
                        is_mob_target: true, owner: arrow.owner, is_critical: arrow.is_critical,
                    });
                    break;
                }
            }
        }

        // Block collision check — check if the new position is inside a solid block
        let block_pos = BlockPos::new(
            pos.0.x.floor() as i32,
            pos.0.y.floor() as i32,
            pos.0.z.floor() as i32,
        );
        let block_at = world_state.get_block(&block_pos);
        if block_at != 0 {
            // Arrow hit a block — stop it
            // Snap to the block face (approximately)
            arrow.in_ground = true;
            vel.0 = Vec3d::new(0.0, 0.0, 0.0);
            og.0 = true;

            // Play arrow hit sound
            play_sound_at_entity(world, pos.0.x, pos.0.y, pos.0.z, "entity.arrow.hit_block", SOUND_NEUTRAL, 1.0, 1.0);
            // Broadcast velocity zero
            broadcast_to_all(world, &InternalPacket::SetEntityVelocity {
                entity_id: eid.0,
                velocity_x: 0,
                velocity_y: 0,
                velocity_z: 0,
            });
            continue;
        }

        // Air drag
        vel.0.x *= 0.99;
        vel.0.y *= 0.99;
        vel.0.z *= 0.99;

        // Update rotation based on velocity
        let horiz = (vel.0.x * vel.0.x + vel.0.z * vel.0.z).sqrt();
        rot.yaw = (vel.0.z.atan2(vel.0.x).to_degrees() as f32) - 90.0;
        rot.pitch = -(vel.0.y.atan2(horiz).to_degrees() as f32);

        let _ = old_pos; // suppress unused warning
    }

    // Process entity hits
    for hit in &entity_hits {
        if hit.is_mob_target {
            // Arrow hit a mob — use attack_mob
            if let Some(owner) = hit.owner {
                let owner_eid = world.get::<&EntityId>(owner).map(|e| e.0).unwrap_or(0);
                attack_mob(world, world_state, owner, owner_eid, hit.target_entity, hit.target_eid,
                    hit.damage, hit.is_critical, scripting, next_eid);
            } else {
                // No owner (shouldn't happen but handle gracefully) — direct mob damage
                if let Ok(mut mob) = world.get::<&mut MobEntity>(hit.target_entity) {
                    mob.health -= hit.damage;
                    mob.no_damage_ticks = 10;
                }
            }
        } else {
            // Arrow hit a player — use apply_damage
            apply_damage(world, world_state, hit.target_entity, hit.target_eid, hit.damage, "arrow", scripting);
        }

        // Play hit sound
        let sound = if hit.is_mob_target { "entity.arrow.hit_player" } else { "entity.arrow.hit_player" };
        play_sound_at_entity(world, hit.hit_pos.x, hit.hit_pos.y, hit.hit_pos.z, sound, SOUND_NEUTRAL, 1.0, 1.0);

        // Broadcast hurt animation
        broadcast_to_all(world, &InternalPacket::HurtAnimation {
            entity_id: hit.target_eid,
            yaw: 0.0,
        });

        // If arrow is from a player, drop it as pickup
        if hit.from_player {
            let arrow_item_id = pickaxe_data::item_name_to_id("arrow").unwrap_or(802);
            spawn_item_entity(
                world,
                world_state,
                next_eid,
                hit.hit_pos.x,
                hit.hit_pos.y,
                hit.hit_pos.z,
                ItemStack::new(arrow_item_id, 1),
                10, // pickup delay
                scripting,
            );
        }

        // Remove the arrow
        to_despawn.push((hit.arrow_entity, hit.arrow_eid));
    }

    // Despawn arrows
    for (entity, eid) in &to_despawn {
        broadcast_to_all(world, &InternalPacket::RemoveEntities {
            entity_ids: vec![*eid],
        });
        for (_e, tracked) in world.query::<&mut TrackedEntities>().iter() {
            tracked.visible.remove(eid);
        }
        let _ = world.despawn(*entity);
    }
}

/// Spawn a primed TNT entity at the given position.
fn spawn_tnt_entity(
    world: &mut World,
    world_state: &mut WorldState,
    next_eid: &Arc<AtomicI32>,
    x: f64,
    y: f64,
    z: f64,
    fuse: i32,
    owner: Option<hecs::Entity>,
    scripting: &ScriptRuntime,
) -> i32 {
    let eid = next_eid.fetch_add(1, Ordering::Relaxed);
    let uuid = Uuid::new_v4();

    // Small random initial velocity (MC: ±0.02 at random angle)
    let mut rng = rand::thread_rng();
    let angle: f64 = rng.gen_range(0.0..std::f64::consts::TAU);
    let vx = -angle.sin() * 0.02;
    let vy = 0.2; // small upward pop
    let vz = angle.cos() * 0.02;

    world.spawn((
        EntityId(eid),
        EntityUuid(uuid),
        Position(Vec3d::new(x, y, z)),
        PreviousPosition(Vec3d::new(x, y, z)),
        Velocity(Vec3d::new(vx, vy, vz)),
        OnGround(false),
        TntEntity { fuse, owner },
        Rotation { yaw: 0.0, pitch: 0.0 },
    ));

    // Play fuse sound
    play_sound_at_entity(world, x, y, z, "entity.tnt.primed", SOUND_BLOCKS, 1.0, 1.0);

    // Fire entity_spawn event
    scripting.fire_event_in_context(
        "entity_spawn",
        &[
            ("entity_id", &eid.to_string()),
            ("entity_type", "tnt"),
            ("x", &format!("{:.2}", x)),
            ("y", &format!("{:.2}", y)),
            ("z", &format!("{:.2}", z)),
            ("fuse", &fuse.to_string()),
        ],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );

    eid
}

/// Tick primed TNT entities: gravity, velocity, fuse countdown, explosion.
fn tick_tnt_entities(
    world: &mut World,
    world_state: &mut WorldState,
    next_eid: &Arc<AtomicI32>,
    scripting: &ScriptRuntime,
) {
    // Collect TNT updates
    struct TntUpdate {
        entity: hecs::Entity,
        eid: i32,
        pos: Vec3d,
        should_explode: bool,
    }

    let mut updates: Vec<TntUpdate> = Vec::new();

    for (e, (eid, pos, vel, og, tnt)) in world
        .query::<(&EntityId, &mut Position, &mut Velocity, &mut OnGround, &mut TntEntity)>()
        .iter()
    {
        // Apply gravity
        vel.0.y -= 0.04;

        // Apply velocity to position
        pos.0.x += vel.0.x;
        pos.0.y += vel.0.y;
        pos.0.z += vel.0.z;

        // Ground collision
        let feet_x = pos.0.x.floor() as i32;
        let feet_y = (pos.0.y - 0.01).floor() as i32;
        let feet_z = pos.0.z.floor() as i32;
        let ground_block = world_state.get_block(&BlockPos::new(feet_x, feet_y, feet_z));
        if ground_block != 0 {
            // On solid ground
            let ground_y = (feet_y + 1) as f64;
            if pos.0.y < ground_y {
                pos.0.y = ground_y;
                vel.0.y *= -0.5; // bounce
                vel.0.x *= 0.7;
                vel.0.z *= 0.7;
                og.0 = true;
            }
        } else {
            og.0 = false;
        }

        // Friction/drag
        vel.0.x *= 0.98;
        vel.0.y *= 0.98;
        vel.0.z *= 0.98;

        // Decrement fuse
        tnt.fuse -= 1;

        updates.push(TntUpdate {
            entity: e,
            eid: eid.0,
            pos: pos.0,
            should_explode: tnt.fuse <= 0,
        });
    }

    // Process explosions
    for update in &updates {
        if update.should_explode {
            do_explosion(
                world,
                world_state,
                next_eid,
                scripting,
                update.pos.x,
                update.pos.y + 0.0625, // MC offsets slightly upward
                update.pos.z,
                4.0,
                true, // destroy blocks
            );

            // Despawn the TNT entity
            let _ = world.despawn(update.entity);
            broadcast_to_all(world, &InternalPacket::RemoveEntities {
                entity_ids: vec![update.eid],
            });
            for (_e, tracked) in world.query::<&mut TrackedEntities>().iter() {
                tracked.visible.remove(&update.eid);
            }
        }
    }
}

/// Perform an explosion at the given location with the given radius.
/// Handles ray-casting block destruction, entity damage, knockback, chain TNT, and packets.
fn do_explosion(
    world: &mut World,
    world_state: &mut WorldState,
    next_eid: &Arc<AtomicI32>,
    scripting: &ScriptRuntime,
    center_x: f64,
    center_y: f64,
    center_z: f64,
    radius: f32,
    destroy_blocks: bool,
) {
    use std::collections::HashSet;

    let mut rng = rand::thread_rng();
    let base_x = center_x.floor() as i32;
    let base_y = center_y.floor() as i32;
    let base_z = center_z.floor() as i32;

    // Phase 1: Ray-casting to find destroyed blocks
    let mut destroyed_positions: HashSet<(i32, i32, i32)> = HashSet::new();

    if destroy_blocks {
        for j in 0..16i32 {
            for k in 0..16i32 {
                for l in 0..16i32 {
                    // Only edge rays
                    if j != 0 && j != 15 && k != 0 && k != 15 && l != 0 && l != 15 {
                        continue;
                    }

                    // Direction from center of 16x16x16 cube
                    let mut dx = (j as f64 / 15.0) * 2.0 - 1.0;
                    let mut dy = (k as f64 / 15.0) * 2.0 - 1.0;
                    let mut dz = (l as f64 / 15.0) * 2.0 - 1.0;
                    let len = (dx * dx + dy * dy + dz * dz).sqrt();
                    dx /= len;
                    dy /= len;
                    dz /= len;

                    // Ray intensity: radius * (0.7 + random * 0.6)
                    let mut intensity = radius as f64 * (0.7 + rng.gen::<f64>() * 0.6);

                    let mut rx = center_x;
                    let mut ry = center_y;
                    let mut rz = center_z;

                    while intensity > 0.0 {
                        let bx = rx.floor() as i32;
                        let by = ry.floor() as i32;
                        let bz = rz.floor() as i32;

                        let block = world_state.get_block(&BlockPos::new(bx, by, bz));
                        if block != 0 {
                            let resistance = pickaxe_data::block_state_to_resistance(block);
                            intensity -= (resistance as f64 + 0.3) * 0.3;

                            if intensity > 0.0 {
                                // Block is destroyed (not indestructible)
                                if resistance < 100.0 {
                                    destroyed_positions.insert((bx, by, bz));
                                }
                            }
                        } else {
                            // Air: small attenuation
                            intensity -= 0.3 * 0.3; // (0 + 0.3) * 0.3 = 0.09
                        }

                        rx += dx * 0.3;
                        ry += dy * 0.3;
                        rz += dz * 0.3;

                        // Safety: stop after traveling past max radius
                        let dist_sq = (rx - center_x).powi(2) + (ry - center_y).powi(2) + (rz - center_z).powi(2);
                        if dist_sq > ((radius as f64 * 2.0) + 2.0).powi(2) {
                            break;
                        }
                    }
                }
            }
        }
    }

    // Phase 2: Destroy blocks, spawn drops, chain-ignite TNT
    let mut chain_tnt: Vec<(f64, f64, f64)> = Vec::new();
    let mut block_offsets: Vec<(i8, i8, i8)> = Vec::new();

    for &(bx, by, bz) in &destroyed_positions {
        let pos = BlockPos::new(bx, by, bz);
        let block = world_state.get_block(&pos);
        if block == 0 {
            continue; // Already air (another ray got it)
        }

        let block_name = pickaxe_data::block_state_to_name(block).unwrap_or("");

        // Check for TNT chain reaction
        if block_name == "tnt" {
            chain_tnt.push((bx as f64 + 0.5, by as f64, bz as f64 + 0.5));
        } else {
            // Spawn item drops (1/radius chance per block in explosions, MC uses 1/radius)
            if rng.gen::<f64>() < (1.0 / radius as f64) {
                let drops = pickaxe_data::block_state_to_drops(block);
                for &drop_id in drops {
                    let drop_item = ItemStack::new(drop_id, 1);
                    spawn_item_entity(
                        world,
                        world_state,
                        next_eid,
                        bx as f64 + 0.5,
                        by as f64 + 0.5,
                        bz as f64 + 0.5,
                        drop_item,
                        10,
                        scripting,
                    );
                }
            }
        }

        // Remove block entity if any
        world_state.remove_block_entity(&pos);

        // Set to air
        world_state.set_block(&pos, 0);
        broadcast_to_all(world, &InternalPacket::BlockUpdate {
            position: pos,
            block_id: 0,
        });

        // Calculate offset for explosion packet
        let dx = (bx - base_x) as i8;
        let dy = (by - base_y) as i8;
        let dz = (bz - base_z) as i8;
        block_offsets.push((dx, dy, dz));
    }

    // Phase 3: Entity damage and knockback
    let damage_radius = radius as f64 * 2.0;

    // Collect player info for damage + per-player knockback
    struct PlayerExplosionInfo {
        entity: hecs::Entity,
        eid: i32,
        damage: f32,
        knockback_x: f32,
        knockback_y: f32,
        knockback_z: f32,
    }
    let mut player_infos: Vec<PlayerExplosionInfo> = Vec::new();

    for (pe, (peid, ppos, _profile)) in world.query::<(&EntityId, &Position, &Profile)>().iter() {
        let dx = ppos.0.x - center_x;
        let dy = ppos.0.y - center_y;
        let dz = ppos.0.z - center_z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        if dist < damage_radius && dist > 0.0 {
            let d0 = dist / damage_radius;
            let d1 = 1.0 - d0; // simplified LOS (assume full exposure)
            let damage = ((d1 * d1 + d1) / 2.0 * 7.0 * damage_radius + 1.0) as f32;
            let knockback = d1;
            let nx = dx / dist;
            let ny = dy / dist;
            let nz = dz / dist;
            player_infos.push(PlayerExplosionInfo {
                entity: pe,
                eid: peid.0,
                damage,
                knockback_x: (nx * knockback) as f32,
                knockback_y: (ny * knockback) as f32,
                knockback_z: (nz * knockback) as f32,
            });
        }
    }

    // Apply damage to players
    for info in &player_infos {
        apply_damage(world, world_state, info.entity, info.eid, info.damage, "explosion", scripting);
    }

    // Damage mobs
    let mut mob_damage: Vec<(hecs::Entity, i32, f32, Vec3d)> = Vec::new();
    for (me, (meid, mpos, _mob)) in world.query::<(&EntityId, &Position, &MobEntity)>().iter() {
        let dx = mpos.0.x - center_x;
        let dy = mpos.0.y - center_y;
        let dz = mpos.0.z - center_z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        if dist < damage_radius && dist > 0.0 {
            let d0 = dist / damage_radius;
            let d1 = 1.0 - d0;
            let damage = ((d1 * d1 + d1) / 2.0 * 7.0 * damage_radius + 1.0) as f32;
            mob_damage.push((me, meid.0, damage, mpos.0));
        }
    }

    for (me, meid, damage, mpos) in &mob_damage {
        if let Ok(mut mob) = world.get::<&mut MobEntity>(*me) {
            if mob.no_damage_ticks <= 0 {
                mob.health -= damage;
                mob.no_damage_ticks = 10;
                broadcast_to_all(world, &InternalPacket::HurtAnimation {
                    entity_id: *meid,
                    yaw: 0.0,
                });
                play_sound_at_entity(world, mpos.x, mpos.y, mpos.z, "entity.generic.hurt", SOUND_HOSTILE, 1.0, 1.0);
            }
        }
    }

    // Phase 4: Send per-player explosion packets with individual knockback
    let block_interaction = if destroy_blocks { 2 } else { 0 }; // DESTROY_WITH_DECAY or KEEP
    for (pe, (peid, sender)) in world.query::<(&EntityId, &ConnectionSender)>().iter() {
        // Find this player's knockback
        let (kx, ky, kz) = player_infos.iter()
            .find(|i| i.eid == peid.0)
            .map(|i| (i.knockback_x, i.knockback_y, i.knockback_z))
            .unwrap_or((0.0, 0.0, 0.0));
        let _ = sender.0.send(InternalPacket::Explosion {
            x: center_x,
            y: center_y,
            z: center_z,
            power: radius,
            destroyed_blocks: block_offsets.clone(),
            knockback_x: kx,
            knockback_y: ky,
            knockback_z: kz,
            block_interaction,
        });
    }

    // Phase 5: Chain-ignite TNT blocks
    for (tx, ty, tz) in chain_tnt {
        let fuse = rng.gen_range(10..30); // shorter fuse for chain reaction
        spawn_tnt_entity(world, world_state, next_eid, tx, ty, tz, fuse, None, scripting);
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
            play_sound_at_entity(world, pos.0.x, pos.0.y, pos.0.z, "entity.item.pickup", SOUND_PLAYERS, 0.2, (rand::random::<f32>() - 0.5) * 1.4 + 1.0);
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

/// Update redstone components in response to a block change at `origin`.
/// Propagates power changes to adjacent redstone wire, torches, repeaters, and lamps.
fn update_redstone_neighbors(
    world: &World,
    world_state: &mut WorldState,
    origin: &BlockPos,
) {
    use std::collections::{HashSet, VecDeque};

    // Early exit: quick scan of origin + 6 neighbors for any redstone component.
    // If nothing redstone-related is nearby, skip the expensive BFS entirely.
    let offsets: [(i32, i32, i32); 7] = [
        (0, 0, 0),
        (1, 0, 0), (-1, 0, 0),
        (0, 1, 0), (0, -1, 0),
        (0, 0, 1), (0, 0, -1),
    ];
    let mut has_redstone = false;
    for &(dx, dy, dz) in &offsets {
        let pos = BlockPos::new(origin.x + dx, origin.y + dy, origin.z + dz);
        if let Some(s) = world_state.get_block_if_loaded(&pos) {
            if pickaxe_data::is_redstone_wire(s)
                || pickaxe_data::is_redstone_torch(s)
                || pickaxe_data::is_repeater(s)
                || pickaxe_data::is_redstone_lamp(s)
                || pickaxe_data::is_any_piston(s)
                || pickaxe_data::is_lever_powered(s)
                || pickaxe_data::is_button_powered(s)
                || pickaxe_data::block_state_to_name(s) == Some("redstone_block")
            {
                has_redstone = true;
                break;
            }
        }
    }
    if !has_redstone { return; }

    // Collect positions that need checking: the origin + all 6 neighbors
    let mut to_check: VecDeque<BlockPos> = VecDeque::new();
    let mut visited: HashSet<(i32, i32, i32)> = HashSet::new();

    for &(dx, dy, dz) in &offsets {
        let pos = BlockPos::new(origin.x + dx, origin.y + dy, origin.z + dz);
        if visited.insert((pos.x, pos.y, pos.z)) {
            to_check.push_back(pos);
        }
    }

    // Also check positions 2 blocks away through solid blocks (strong power propagation)
    // and positions diagonally adjacent for wire connections
    let diag_offsets: [(i32, i32, i32); 4] = [
        (1, 1, 0), (-1, 1, 0), (0, 1, 1), (0, 1, -1),
    ];
    for &(dx, dy, dz) in &diag_offsets {
        let pos = BlockPos::new(origin.x + dx, origin.y + dy, origin.z + dz);
        if visited.insert((pos.x, pos.y, pos.z)) {
            to_check.push_back(pos);
        }
        // Also below
        let pos2 = BlockPos::new(origin.x + dx, origin.y - 1 + dy, origin.z + dz);
        if visited.insert((pos2.x, pos2.y, pos2.z)) {
            to_check.push_back(pos2);
        }
    }

    // Process all redstone blocks in the check set
    // We may add more positions as wire power propagates
    let mut wire_updates: Vec<(BlockPos, i32, i32)> = Vec::new(); // (pos, old_state, new_state)
    let mut block_updates: Vec<(BlockPos, i32, i32)> = Vec::new(); // other redstone block updates
    let mut piston_actions: Vec<(BlockPos, i32, bool)> = Vec::new(); // (pos, state, should_extend)

    while let Some(pos) = to_check.pop_front() {
        let state = match world_state.get_block_if_loaded(&pos) {
            Some(s) => s,
            None => continue,
        };

        // --- Redstone Wire ---
        if pickaxe_data::is_redstone_wire(state) {
            let new_power = calculate_wire_power(world_state, &pos);
            let old_power = pickaxe_data::redstone_wire_power(state).unwrap_or(0);
            if new_power != old_power {
                let new_state = pickaxe_data::redstone_wire_state(new_power);
                wire_updates.push((pos, state, new_state));

                // When wire power changes, check its neighbors too (propagation)
                for &(dx, dy, dz) in &[(1i32,0i32,0i32),(-1,0,0),(0,0,1),(0,0,-1),(0,1,0),(0,-1,0)] {
                    let npos = BlockPos::new(pos.x + dx, pos.y + dy, pos.z + dz);
                    if visited.insert((npos.x, npos.y, npos.z)) {
                        to_check.push_back(npos);
                    }
                }
                // Wire can also power through diagonals (up/down slopes)
                for &(dx, dz) in &[(1i32,0i32),(-1,0),(0,1),(0,-1)] {
                    let above = BlockPos::new(pos.x + dx, pos.y + 1, pos.z + dz);
                    if visited.insert((above.x, above.y, above.z)) {
                        to_check.push_back(above);
                    }
                    let below = BlockPos::new(pos.x + dx, pos.y - 1, pos.z + dz);
                    if visited.insert((below.x, below.y, below.z)) {
                        to_check.push_back(below);
                    }
                }
            }
        }

        // --- Redstone Torch ---
        if pickaxe_data::is_redstone_torch(state) {
            let should_be_lit = !is_torch_receiving_power(world_state, &pos, state);
            let is_lit = pickaxe_data::redstone_torch_is_lit(state);
            if should_be_lit != is_lit {
                let new_state = pickaxe_data::redstone_torch_set_lit(state, should_be_lit);
                block_updates.push((pos, state, new_state));
                // Torch state change affects its neighbors
                for &(dx, dy, dz) in &offsets {
                    let npos = BlockPos::new(pos.x + dx, pos.y + dy, pos.z + dz);
                    if visited.insert((npos.x, npos.y, npos.z)) {
                        to_check.push_back(npos);
                    }
                }
            }
        }

        // --- Repeater ---
        if pickaxe_data::is_repeater(state) {
            if let Some((delay, facing, locked, powered)) = pickaxe_data::repeater_props(state) {
                if !locked {
                    let has_input = repeater_has_input(world_state, &pos, facing);
                    if has_input != powered {
                        let new_state = pickaxe_data::repeater_state(delay, facing, locked, has_input);
                        block_updates.push((pos, state, new_state));
                        // Repeater output affects the block it points to
                        let (dx, dz) = pickaxe_data::facing_to_offset(facing);
                        let out_pos = BlockPos::new(pos.x + dx, pos.y, pos.z + dz);
                        if visited.insert((out_pos.x, out_pos.y, out_pos.z)) {
                            to_check.push_back(out_pos);
                        }
                        // Also check behind (input side)
                        let (bdx, bdz) = pickaxe_data::facing_to_offset(pickaxe_data::opposite_facing(facing));
                        let back_pos = BlockPos::new(pos.x + bdx, pos.y, pos.z + bdz);
                        if visited.insert((back_pos.x, back_pos.y, back_pos.z)) {
                            to_check.push_back(back_pos);
                        }
                    }
                }
            }
        }

        // --- Redstone Lamp ---
        if pickaxe_data::is_redstone_lamp(state) {
            let has_power = block_receives_power(world_state, &pos);
            let is_lit = state == pickaxe_data::redstone_lamp_set_lit(true);
            if has_power != is_lit {
                let new_state = pickaxe_data::redstone_lamp_set_lit(has_power);
                block_updates.push((pos, state, new_state));
            }
        }

        // --- Piston ---
        if pickaxe_data::is_any_piston(state) && !pickaxe_data::is_piston_head(state) {
            let is_extended = pickaxe_data::piston_is_extended(state);
            let has_power = block_receives_power(world_state, &pos);
            if has_power && !is_extended {
                // Should extend — collect for processing after all updates
                piston_actions.push((pos, state, true));
            } else if !has_power && is_extended {
                // Should retract
                piston_actions.push((pos, state, false));
            }
        }
    }

    // Apply all wire updates
    for (pos, _old, new_state) in &wire_updates {
        world_state.set_block(pos, *new_state);
        broadcast_to_all(world, &InternalPacket::BlockUpdate {
            position: *pos,
            block_id: *new_state,
        });
    }

    // Apply all other block updates
    for (pos, _old, new_state) in &block_updates {
        world_state.set_block(pos, *new_state);
        broadcast_to_all(world, &InternalPacket::BlockUpdate {
            position: *pos,
            block_id: *new_state,
        });
    }

    // If any torches or repeaters changed, we need a second pass for cascading effects
    if !block_updates.is_empty() {
        for (pos, _, _) in block_updates {
            // Recursively propagate from each changed block
            // Use a simple iteration limit to prevent infinite loops
            update_redstone_cascade(world, world_state, &pos, 0);
        }
    }

    // Process piston actions (extend/retract)
    for (pos, state, should_extend) in piston_actions {
        if should_extend {
            try_extend_piston(world, world_state, &pos, state);
        } else {
            try_retract_piston(world, world_state, &pos, state);
        }
    }
}

/// Cascade redstone updates from a changed block, up to a depth limit.
fn update_redstone_cascade(
    world: &World,
    world_state: &mut WorldState,
    origin: &BlockPos,
    depth: u32,
) {
    if depth >= 16 { return; } // Prevent infinite loops

    use std::collections::HashSet;

    let offsets: [(i32, i32, i32); 6] = [
        (1, 0, 0), (-1, 0, 0),
        (0, 1, 0), (0, -1, 0),
        (0, 0, 1), (0, 0, -1),
    ];

    let mut changes: Vec<(BlockPos, i32)> = Vec::new();

    // Check all neighbors of origin
    for &(dx, dy, dz) in &offsets {
        let pos = BlockPos::new(origin.x + dx, origin.y + dy, origin.z + dz);
        let state = match world_state.get_block_if_loaded(&pos) {
            Some(s) => s,
            None => continue,
        };

        // Wire
        if pickaxe_data::is_redstone_wire(state) {
            let new_power = calculate_wire_power(world_state, &pos);
            let old_power = pickaxe_data::redstone_wire_power(state).unwrap_or(0);
            if new_power != old_power {
                let new_state = pickaxe_data::redstone_wire_state(new_power);
                changes.push((pos, new_state));
            }
        }

        // Torch
        if pickaxe_data::is_redstone_torch(state) {
            let should_be_lit = !is_torch_receiving_power(world_state, &pos, state);
            let is_lit = pickaxe_data::redstone_torch_is_lit(state);
            if should_be_lit != is_lit {
                let new_state = pickaxe_data::redstone_torch_set_lit(state, should_be_lit);
                changes.push((pos, new_state));
            }
        }

        // Repeater
        if pickaxe_data::is_repeater(state) {
            if let Some((delay, facing, locked, powered)) = pickaxe_data::repeater_props(state) {
                if !locked {
                    let has_input = repeater_has_input(world_state, &pos, facing);
                    if has_input != powered {
                        let new_state = pickaxe_data::repeater_state(delay, facing, locked, has_input);
                        changes.push((pos, new_state));
                    }
                }
            }
        }

        // Lamp
        if pickaxe_data::is_redstone_lamp(state) {
            let has_power = block_receives_power(world_state, &pos);
            let is_lit = state == pickaxe_data::redstone_lamp_set_lit(true);
            if has_power != is_lit {
                let new_state = pickaxe_data::redstone_lamp_set_lit(has_power);
                changes.push((pos, new_state));
            }
        }
    }

    // Also check wire on diagonals (up/down)
    for &(dx, dz) in &[(1i32,0i32),(-1,0),(0,1),(0,-1)] {
        for dy in [-1i32, 1] {
            let pos = BlockPos::new(origin.x + dx, origin.y + dy, origin.z + dz);
            let state = match world_state.get_block_if_loaded(&pos) {
                Some(s) => s,
                None => continue,
            };
            if pickaxe_data::is_redstone_wire(state) {
                let new_power = calculate_wire_power(world_state, &pos);
                let old_power = pickaxe_data::redstone_wire_power(state).unwrap_or(0);
                if new_power != old_power {
                    let new_state = pickaxe_data::redstone_wire_state(new_power);
                    changes.push((pos, new_state));
                }
            }
        }
    }

    for (pos, new_state) in &changes {
        world_state.set_block(pos, *new_state);
        broadcast_to_all(world, &InternalPacket::BlockUpdate {
            position: *pos,
            block_id: *new_state,
        });
    }

    // Recurse for any changes
    for (pos, _) in changes {
        update_redstone_cascade(world, world_state, &pos, depth + 1);
    }
}

/// Calculate what power level a redstone wire at `pos` should have.
/// Checks all adjacent power sources and neighboring wires.
fn calculate_wire_power(world_state: &WorldState, pos: &BlockPos) -> i32 {
    let mut max_power: i32 = 0;

    // Check all 6 neighbors for direct power sources (levers, buttons, torches, repeaters, redstone blocks)
    let offsets: [(i32, i32, i32); 6] = [
        (1, 0, 0), (-1, 0, 0),
        (0, 1, 0), (0, -1, 0),
        (0, 0, 1), (0, 0, -1),
    ];

    for &(dx, dy, dz) in &offsets {
        let npos = BlockPos::new(pos.x + dx, pos.y + dy, pos.z + dz);
        let nstate = match world_state.get_block_if_loaded(&npos) {
            Some(s) => s,
            None => continue,
        };

        // Redstone block always outputs 15
        if pickaxe_data::block_state_to_name(nstate) == Some("redstone_block") {
            max_power = 15;
            continue;
        }

        // Lit torch above or beside outputs 15
        if pickaxe_data::is_redstone_torch(nstate) && pickaxe_data::redstone_torch_is_lit(nstate) {
            // Torches don't power wire through the block they're on, they power adjacent blocks
            // Standing torch at (x, y+1) powers wire at (x, y) — yes (below)
            // Wall torch facing away powers wire on the other side — more complex
            if dy == 1 || dy == -1 {
                max_power = 15;
            } else {
                max_power = 15;
            }
            continue;
        }

        // Lever/button: powers adjacent wire
        if pickaxe_data::is_lever_powered(nstate) || pickaxe_data::is_button_powered(nstate) {
            max_power = 15;
            continue;
        }

        // Powered repeater facing into this wire
        if pickaxe_data::is_repeater(nstate) {
            if let Some((_, facing, _, powered)) = pickaxe_data::repeater_props(nstate) {
                if powered {
                    let (fdx, fdz) = pickaxe_data::facing_to_offset(facing);
                    // Repeater at npos facing direction (fdx, fdz) outputs to npos + (fdx, 0, fdz)
                    // So it powers this wire if npos + (fdx, 0, fdz) == pos
                    if npos.x + fdx == pos.x && npos.z + fdz == pos.z && dy == 0 {
                        max_power = 15;
                    }
                }
            }
        }
    }

    // Check horizontal neighbors for wire power (attenuated by 1)
    let horiz: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    for &(dx, dz) in &horiz {
        let npos = BlockPos::new(pos.x + dx, pos.y, pos.z + dz);
        let nstate = match world_state.get_block_if_loaded(&npos) {
            Some(s) => s,
            None => continue,
        };

        // Direct horizontal wire neighbor
        if pickaxe_data::is_redstone_wire(nstate) {
            let npower = pickaxe_data::redstone_wire_power(nstate).unwrap_or(0);
            max_power = max_power.max(npower - 1);
        }

        // Wire going up a slope: check pos above neighbor if neighbor is not a solid block
        let above_neighbor = BlockPos::new(pos.x + dx, pos.y + 1, pos.z + dz);
        let above_state = world_state.get_block_if_loaded(&above_neighbor).unwrap_or(0);
        if pickaxe_data::is_redstone_wire(above_state) {
            // Can connect up only if block directly above wire pos is not solid
            let above_self = BlockPos::new(pos.x, pos.y + 1, pos.z);
            let above_self_state = world_state.get_block_if_loaded(&above_self).unwrap_or(0);
            if !pickaxe_data::is_solid_block(above_self_state) {
                let npower = pickaxe_data::redstone_wire_power(above_state).unwrap_or(0);
                max_power = max_power.max(npower - 1);
            }
        }

        // Wire going down a slope: check pos below neighbor if neighbor is not solid
        if !pickaxe_data::is_solid_block(nstate) {
            let below_neighbor = BlockPos::new(pos.x + dx, pos.y - 1, pos.z + dz);
            let below_state = world_state.get_block_if_loaded(&below_neighbor).unwrap_or(0);
            if pickaxe_data::is_redstone_wire(below_state) {
                let npower = pickaxe_data::redstone_wire_power(below_state).unwrap_or(0);
                max_power = max_power.max(npower - 1);
            }
        }
    }

    // Strong power: check if a solid block adjacent is receiving power from a source
    // (solid blocks pass through strong power to adjacent wire)
    for &(dx, dy, dz) in &offsets {
        let npos = BlockPos::new(pos.x + dx, pos.y + dy, pos.z + dz);
        let nstate = match world_state.get_block_if_loaded(&npos) {
            Some(s) => s,
            None => continue,
        };
        if pickaxe_data::is_solid_block(nstate) {
            // Check if this solid block has a power source directly attached
            let strong = get_strong_power_into_block(world_state, &npos);
            if strong > 0 {
                max_power = max_power.max(strong);
            }
        }
    }

    max_power.clamp(0, 15)
}

/// Get the strong power level being fed into a solid block at `pos`.
/// Strong power comes from: powered repeater output, powered lever/button on the block.
fn get_strong_power_into_block(world_state: &WorldState, pos: &BlockPos) -> i32 {
    let mut power = 0i32;

    let offsets: [(i32, i32, i32); 6] = [
        (1, 0, 0), (-1, 0, 0),
        (0, 1, 0), (0, -1, 0),
        (0, 0, 1), (0, 0, -1),
    ];

    for &(dx, dy, dz) in &offsets {
        let npos = BlockPos::new(pos.x + dx, pos.y + dy, pos.z + dz);
        let nstate = match world_state.get_block_if_loaded(&npos) {
            Some(s) => s,
            None => continue,
        };

        // Powered repeater facing into this block
        if pickaxe_data::is_repeater(nstate) {
            if let Some((_, facing, _, powered)) = pickaxe_data::repeater_props(nstate) {
                if powered {
                    let (fdx, fdz) = pickaxe_data::facing_to_offset(facing);
                    if npos.x + fdx == pos.x && npos.z + fdz == pos.z && dy == 0 {
                        power = 15;
                    }
                }
            }
        }

        // Torch below this block (standing torch)
        if dy == -1 && pickaxe_data::is_redstone_torch(nstate) && pickaxe_data::redstone_torch_is_lit(nstate) {
            // Standing torch at y-1 strongly powers the block above
            if nstate == 5739 { // standing torch, lit
                power = 15;
            }
        }

        // Lever/button directly on this block
        if pickaxe_data::is_lever_powered(nstate) || pickaxe_data::is_button_powered(nstate) {
            power = 15;
        }
    }

    power
}

/// Check if a redstone torch at `pos` is receiving power (should turn off).
/// A torch turns off when the block it's attached to is powered.
fn is_torch_receiving_power(world_state: &WorldState, pos: &BlockPos, state: i32) -> bool {
    // Standing torch: attached to block below
    if state == 5738 || state == 5739 {
        let below = BlockPos::new(pos.x, pos.y - 1, pos.z);
        return block_receives_power(world_state, &below);
    }

    // Wall torch: attached to the block it faces away from
    if let Some(facing) = pickaxe_data::redstone_wall_torch_facing(state) {
        // Wall torch facing direction means it's attached to the opposite side
        let (dx, dz) = pickaxe_data::facing_to_offset(pickaxe_data::opposite_facing(facing));
        let attached = BlockPos::new(pos.x + dx, pos.y, pos.z + dz);
        return block_receives_power(world_state, &attached);
    }

    false
}

/// Check if a block at `pos` is receiving any redstone power.
/// Used for lamps, torches checking their attachment block, etc.
fn block_receives_power(world_state: &WorldState, pos: &BlockPos) -> bool {
    let offsets: [(i32, i32, i32); 6] = [
        (1, 0, 0), (-1, 0, 0),
        (0, 1, 0), (0, -1, 0),
        (0, 0, 1), (0, 0, -1),
    ];

    for &(dx, dy, dz) in &offsets {
        let npos = BlockPos::new(pos.x + dx, pos.y + dy, pos.z + dz);
        let nstate = match world_state.get_block_if_loaded(&npos) {
            Some(s) => s,
            None => continue,
        };

        // Direct power sources
        if pickaxe_data::is_lever_powered(nstate) || pickaxe_data::is_button_powered(nstate) {
            return true;
        }

        // Redstone block
        if pickaxe_data::block_state_to_name(nstate) == Some("redstone_block") {
            return true;
        }

        // Lit redstone torch (powers blocks above and adjacent, not the attachment block)
        if pickaxe_data::is_redstone_torch(nstate) && pickaxe_data::redstone_torch_is_lit(nstate) {
            // Standing torch powers block above it
            if (nstate == 5738 || nstate == 5739) && dy == -1 {
                return true; // torch is below us
            }
            // Wall torch: powers all adjacent except its attachment block
            if let Some(torch_facing) = pickaxe_data::redstone_wall_torch_facing(nstate) {
                let (adx, adz) = pickaxe_data::facing_to_offset(pickaxe_data::opposite_facing(torch_facing));
                // If this block is NOT the attachment block, torch powers it
                if !(dx == -adx && dz == -adz && dy == 0) {
                    return true;
                }
            }
        }

        // Powered repeater facing into this block
        if pickaxe_data::is_repeater(nstate) {
            if let Some((_, facing, _, powered)) = pickaxe_data::repeater_props(nstate) {
                if powered {
                    let (fdx, fdz) = pickaxe_data::facing_to_offset(facing);
                    if npos.x + fdx == pos.x && npos.z + fdz == pos.z && dy == 0 {
                        return true;
                    }
                }
            }
        }

        // Redstone wire with power > 0 provides weak power to adjacent blocks
        if pickaxe_data::is_redstone_wire(nstate) {
            let wp = pickaxe_data::redstone_wire_power(nstate).unwrap_or(0);
            if wp > 0 {
                return true;
            }
        }

        // Strong power: if a solid block adjacent is strongly powered, it passes power through
        if pickaxe_data::is_solid_block(nstate) {
            let strong = get_strong_power_into_block(world_state, &npos);
            if strong > 0 {
                return true;
            }
        }
    }

    false
}

/// Check if a repeater at `pos` with given `facing` has an input signal.
fn repeater_has_input(world_state: &WorldState, pos: &BlockPos, facing: i32) -> bool {
    // Repeater input comes from the opposite direction of its facing
    let input_dir = pickaxe_data::opposite_facing(facing);
    let (dx, dz) = pickaxe_data::facing_to_offset(input_dir);
    let input_pos = BlockPos::new(pos.x + dx, pos.y, pos.z + dz);
    let input_state = world_state.get_block_if_loaded(&input_pos).unwrap_or(0);

    // Direct power sources
    if pickaxe_data::block_power_output(input_state) > 0 {
        return true;
    }

    // Redstone wire with power > 0
    if pickaxe_data::is_redstone_wire(input_state) {
        let wp = pickaxe_data::redstone_wire_power(input_state).unwrap_or(0);
        return wp > 0;
    }

    // Another repeater outputting into this one
    if pickaxe_data::is_repeater(input_state) {
        if let Some((_, other_facing, _, other_powered)) = pickaxe_data::repeater_props(input_state) {
            if other_powered {
                let (fdx, fdz) = pickaxe_data::facing_to_offset(other_facing);
                if input_pos.x + fdx == pos.x && input_pos.z + fdz == pos.z {
                    return true;
                }
            }
        }
    }

    // Solid block receiving strong power
    if pickaxe_data::is_solid_block(input_state) {
        let strong = get_strong_power_into_block(world_state, &input_pos);
        if strong > 0 {
            return true;
        }
    }

    false
}

/// Try to extend a piston at `pos`. Resolves the push structure and moves blocks.
fn try_extend_piston(
    world: &World,
    world_state: &mut WorldState,
    pos: &BlockPos,
    state: i32,
) {
    let facing = match pickaxe_data::piston_facing(state) {
        Some(f) => f,
        None => return,
    };
    let is_sticky = pickaxe_data::is_sticky_piston(state);
    let (dx, dy, dz) = pickaxe_data::facing6_to_offset(facing);

    // Resolve the push structure: collect blocks to push
    let head_pos = BlockPos::new(pos.x + dx, pos.y + dy, pos.z + dz);
    let mut to_push: Vec<BlockPos> = Vec::new();
    let mut to_destroy: Vec<BlockPos> = Vec::new();

    if !resolve_push_structure(world_state, &head_pos, facing, &mut to_push, &mut to_destroy) {
        return; // Can't extend (immovable block in the way or too many blocks)
    }

    // Play piston sound
    play_sound_at_block(world, pos, "block.piston.extend", SOUND_BLOCKS, 0.5, 0.7);

    // Destroy breakable blocks first
    for dpos in &to_destroy {
        world_state.set_block(dpos, 0);
        broadcast_to_all(world, &InternalPacket::BlockUpdate {
            position: *dpos,
            block_id: 0,
        });
    }

    // Move blocks from farthest to nearest (so they don't overwrite each other)
    // The push list is ordered from nearest to farthest, so reverse it
    for bpos in to_push.iter().rev() {
        let block = world_state.get_block(bpos);
        let dest = BlockPos::new(bpos.x + dx, bpos.y + dy, bpos.z + dz);
        world_state.set_block(&dest, block);
        world_state.set_block(bpos, 0);
        broadcast_to_all(world, &InternalPacket::BlockUpdate {
            position: dest,
            block_id: block,
        });
    }

    // Set air at positions that were vacated (already done by move loop above clearing source)
    // But we need to broadcast the air for any position not overwritten
    for bpos in &to_push {
        let current = world_state.get_block(bpos);
        if current != 0 {
            // Position was overwritten by another push — already correct
        } else {
            broadcast_to_all(world, &InternalPacket::BlockUpdate {
                position: *bpos,
                block_id: 0,
            });
        }
    }

    // Place piston head at the head position
    let head_state = pickaxe_data::piston_head_state(facing, false, is_sticky);
    world_state.set_block(&head_pos, head_state);
    broadcast_to_all(world, &InternalPacket::BlockUpdate {
        position: head_pos,
        block_id: head_state,
    });

    // Set piston base to extended
    let extended_state = pickaxe_data::piston_state(facing, true, is_sticky);
    world_state.set_block(pos, extended_state);
    broadcast_to_all(world, &InternalPacket::BlockUpdate {
        position: *pos,
        block_id: extended_state,
    });

    // Trigger redstone update from moved blocks (they may affect wires etc.)
    for bpos in &to_push {
        let dest = BlockPos::new(bpos.x + dx, bpos.y + dy, bpos.z + dz);
        update_redstone_neighbors(world, world_state, &dest);
    }
    update_redstone_neighbors(world, world_state, &head_pos);
}

/// Try to retract a piston at `pos`. Removes head and pulls block for sticky.
fn try_retract_piston(
    world: &World,
    world_state: &mut WorldState,
    pos: &BlockPos,
    state: i32,
) {
    let facing = match pickaxe_data::piston_facing(state) {
        Some(f) => f,
        None => return,
    };
    let is_sticky = pickaxe_data::is_sticky_piston(state);
    let (dx, dy, dz) = pickaxe_data::facing6_to_offset(facing);

    // Remove piston head
    let head_pos = BlockPos::new(pos.x + dx, pos.y + dy, pos.z + dz);
    let head_block = world_state.get_block(&head_pos);
    if pickaxe_data::is_piston_head(head_block) {
        world_state.set_block(&head_pos, 0);
        broadcast_to_all(world, &InternalPacket::BlockUpdate {
            position: head_pos,
            block_id: 0,
        });
    }

    // Set piston base to retracted
    let retracted_state = pickaxe_data::piston_state(facing, false, is_sticky);
    world_state.set_block(pos, retracted_state);
    broadcast_to_all(world, &InternalPacket::BlockUpdate {
        position: *pos,
        block_id: retracted_state,
    });

    // For sticky pistons, try to pull the block 2 positions out
    if is_sticky {
        let pull_pos = BlockPos::new(pos.x + dx * 2, pos.y + dy * 2, pos.z + dz * 2);
        let pull_block = world_state.get_block(&pull_pos);
        if pull_block != 0 && pickaxe_data::is_pushable(pull_block) && !pickaxe_data::is_piston_destroyable(pull_block) {
            // Pull block to head position
            world_state.set_block(&head_pos, pull_block);
            world_state.set_block(&pull_pos, 0);
            broadcast_to_all(world, &InternalPacket::BlockUpdate {
                position: head_pos,
                block_id: pull_block,
            });
            broadcast_to_all(world, &InternalPacket::BlockUpdate {
                position: pull_pos,
                block_id: 0,
            });
            update_redstone_neighbors(world, world_state, &head_pos);
            update_redstone_neighbors(world, world_state, &pull_pos);
        }
    }

    // Play piston sound
    play_sound_at_block(world, pos, "block.piston.contract", SOUND_BLOCKS, 0.5, 0.65);

    update_redstone_neighbors(world, world_state, &head_pos);
}

/// Resolve the push structure for a piston extending in the given direction.
/// Returns true if the push is possible (all blocks can be moved).
/// Populates `to_push` with blocks to move (nearest to farthest) and
/// `to_destroy` with blocks to destroy.
fn resolve_push_structure(
    world_state: &WorldState,
    start: &BlockPos,
    facing: i32,
    to_push: &mut Vec<BlockPos>,
    to_destroy: &mut Vec<BlockPos>,
) -> bool {
    let (dx, dy, dz) = pickaxe_data::facing6_to_offset(facing);
    let mut check_pos = *start;

    for _ in 0..13 { // max 12 pushable blocks + 1 for the space check
        let block = match world_state.get_block_if_loaded(&check_pos) {
            Some(b) => b,
            None => return false, // out of loaded chunks
        };

        // Air or fluid — nothing blocking, push is valid
        if block == 0 {
            return true;
        }

        // Destroyable blocks get destroyed
        if pickaxe_data::is_piston_destroyable(block) {
            to_destroy.push(check_pos);
            return true;
        }

        // Check if pushable
        if !pickaxe_data::is_pushable(block) {
            return false; // immovable block — can't push
        }

        // Check max push limit
        if to_push.len() >= 12 {
            return false; // too many blocks
        }

        to_push.push(check_pos);
        check_pos = BlockPos::new(check_pos.x + dx, check_pos.y + dy, check_pos.z + dz);
    }

    false // shouldn't reach here
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

/// Tick all brewing stands: consume fuel, progress brew, transform potions.
fn tick_brewing_stands(world: &World, world_state: &mut WorldState) {
    let mut updates: Vec<(BlockPos, i16, i16)> = Vec::new();

    for (pos, block_entity) in world_state.block_entities.iter_mut() {
        let BlockEntity::BrewingStand {
            ref mut bottles, ref mut ingredient, ref mut fuel,
            ref mut brew_time, ref mut fuel_uses,
        } = block_entity else { continue };

        // Check if we need fuel and have blaze powder
        let blaze_powder_id = pickaxe_data::item_name_to_id("blaze_powder").unwrap_or(0);
        if *fuel_uses <= 0 {
            if let Some(ref mut f) = fuel {
                if f.item_id == blaze_powder_id {
                    *fuel_uses = 20;
                    f.count -= 1;
                    if f.count <= 0 { *fuel = None; }
                }
            }
        }

        // Check if we have a valid ingredient that can brew any of the bottles
        let ingredient_name = ingredient.as_ref()
            .and_then(|i| pickaxe_data::item_id_to_name(i.item_id));
        let has_valid_recipe = if let Some(ing_name) = ingredient_name {
            bottles.iter().any(|b| {
                if let Some(item) = b {
                    if !pickaxe_data::is_potion(item.item_id) { return false; }
                    pickaxe_data::brewing_recipe(item.damage, ing_name).is_some()
                } else { false }
            })
        } else { false };

        if *brew_time > 0 {
            // Currently brewing — decrement timer
            if !has_valid_recipe {
                // Ingredient removed or changed — cancel brew
                *brew_time = 0;
            } else {
                *brew_time -= 1;
                if *brew_time <= 0 {
                    // Brew complete — transform potions
                    if let Some(ing_name) = ingredient_name {
                        for bottle in bottles.iter_mut() {
                            if let Some(ref mut item) = bottle {
                                if pickaxe_data::is_potion(item.item_id) {
                                    if let Some(output_idx) = pickaxe_data::brewing_recipe(item.damage, ing_name) {
                                        item.damage = output_idx;
                                    }
                                }
                            }
                        }
                    }
                    // Consume ingredient
                    if let Some(ref mut ing) = ingredient {
                        ing.count -= 1;
                        if ing.count <= 0 { *ingredient = None; }
                    }
                }
            }
            updates.push((*pos, *brew_time, *fuel_uses));
        } else if has_valid_recipe && *fuel_uses > 0 {
            // Start brewing
            *fuel_uses -= 1;
            *brew_time = 400;
            updates.push((*pos, *brew_time, *fuel_uses));
        }
    }

    // Send progress updates to players who have this brewing stand open
    for (pos, bt, fu) in &updates {
        for (_e, (sender, open)) in world.query::<(&ConnectionSender, &OpenContainer)>().iter() {
            if let Menu::BrewingStand { pos: bpos } = &open.menu {
                if bpos == pos {
                    let _ = sender.0.send(InternalPacket::SetContainerData { container_id: open.container_id, property: 0, value: *bt });
                    let _ = sender.0.send(InternalPacket::SetContainerData { container_id: open.container_id, property: 1, value: *fu });
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
        "/effect give <effect> [duration] [amplifier] - Apply status effect",
        "/effect clear [effect] - Remove status effects",
        "/potion <player> <potion_name> - Give a potion to a player",
        "/enchant <enchantment> [level] - Enchant held item",
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

/// /effect give <effect> [duration_seconds] [amplifier] — apply a status effect
/// /effect clear [effect] — remove one or all effects
fn cmd_effect(world: &mut World, entity: hecs::Entity, args: &str) {
    if !is_op(world, entity) {
        send_message(world, entity, "You don't have permission to use this command.");
        return;
    }
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() {
        send_message(world, entity, "Usage: /effect <give|clear> ...");
        return;
    }

    let eid = world.get::<&EntityId>(entity).map(|e| e.0).unwrap_or(0);

    match parts[0] {
        "give" => {
            if parts.len() < 2 {
                send_message(world, entity, "Usage: /effect give <effect> [duration_seconds] [amplifier]");
                return;
            }
            let effect_name = parts[1];
            let effect_id = match pickaxe_data::effect_name_to_id(effect_name) {
                Some(id) => id,
                None => {
                    send_message(world, entity, &format!("Unknown effect: {}", effect_name));
                    return;
                }
            };
            let duration_secs: i32 = if parts.len() > 2 {
                parts[2].parse().unwrap_or(30)
            } else {
                30
            };
            let duration_ticks = if duration_secs < 0 { -1 } else { duration_secs * 20 };
            let amplifier: i32 = if parts.len() > 3 {
                parts[3].parse::<i32>().unwrap_or(0).clamp(0, 255)
            } else {
                0
            };

            let inst = EffectInstance {
                effect_id,
                amplifier,
                duration: duration_ticks,
                ambient: false,
                show_particles: true,
                show_icon: true,
            };
            let flags: u8 = 0x02 | 0x04; // visible + show_icon

            // Handle instant effects
            match effect_id {
                5 => { // instant_health
                    let heal = 4.0 * (1 << amplifier.min(30)) as f32;
                    let max = world.get::<&Health>(entity).map(|h| h.max).unwrap_or(20.0);
                    if let Ok(mut h) = world.get::<&mut Health>(entity) {
                        h.current = (h.current + heal).min(max);
                    }
                    let (health, food, sat) = {
                        let h = world.get::<&Health>(entity).map(|h| h.current).unwrap_or(20.0);
                        let (f, s) = world.get::<&FoodData>(entity).map(|f| (f.food_level, f.saturation)).unwrap_or((20, 5.0));
                        (h, f, s)
                    };
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::SetHealth { health, food, saturation: sat });
                    }
                    send_message(world, entity, &format!("Applied Instant Health (level {})", amplifier + 1));
                    return;
                }
                6 => { // instant_damage
                    let damage = 6.0 * (1 << amplifier.min(30)) as f32;
                    if let Ok(mut h) = world.get::<&mut Health>(entity) {
                        h.current = (h.current - damage).max(0.0);
                    }
                    let (health, food, sat) = {
                        let h = world.get::<&Health>(entity).map(|h| h.current).unwrap_or(0.0);
                        let (f, s) = world.get::<&FoodData>(entity).map(|f| (f.food_level, f.saturation)).unwrap_or((20, 5.0));
                        (h, f, s)
                    };
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::SetHealth { health, food, saturation: sat });
                    }
                    send_message(world, entity, &format!("Applied Instant Damage (level {})", amplifier + 1));
                    return;
                }
                22 => { // saturation
                    if let Ok(mut food) = world.get::<&mut FoodData>(entity) {
                        food.food_level = (food.food_level + amplifier + 1).min(20);
                        food.saturation = (food.saturation + (amplifier + 1) as f32).min(food.food_level as f32);
                    }
                    let (health, food, sat) = {
                        let h = world.get::<&Health>(entity).map(|h| h.current).unwrap_or(20.0);
                        let (f, s) = world.get::<&FoodData>(entity).map(|f| (f.food_level, f.saturation)).unwrap_or((20, 5.0));
                        (h, f, s)
                    };
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::SetHealth { health, food, saturation: sat });
                    }
                    send_message(world, entity, &format!("Applied Saturation (level {})", amplifier + 1));
                    return;
                }
                _ => {}
            }

            if let Ok(mut effects) = world.get::<&mut ActiveEffects>(entity) {
                effects.effects.insert(effect_id, inst);
            }
            if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                let _ = sender.0.send(InternalPacket::UpdateMobEffect {
                    entity_id: eid,
                    effect_id,
                    amplifier,
                    duration: duration_ticks,
                    flags,
                });
            }
            let dur_str = if duration_ticks < 0 { "infinite".to_string() } else { format!("{}s", duration_secs) };
            send_message(world, entity, &format!("Applied {} (level {}) for {}", effect_name, amplifier + 1, dur_str));
        }
        "clear" => {
            if parts.len() > 1 {
                // Clear specific effect
                let effect_name = parts[1];
                let effect_id = match pickaxe_data::effect_name_to_id(effect_name) {
                    Some(id) => id,
                    None => {
                        send_message(world, entity, &format!("Unknown effect: {}", effect_name));
                        return;
                    }
                };
                let removed = if let Ok(mut effects) = world.get::<&mut ActiveEffects>(entity) {
                    effects.effects.remove(&effect_id).is_some()
                } else {
                    false
                };
                if removed {
                    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                        let _ = sender.0.send(InternalPacket::RemoveMobEffect {
                            entity_id: eid,
                            effect_id,
                        });
                    }
                    send_message(world, entity, &format!("Removed {}", effect_name));
                } else {
                    send_message(world, entity, &format!("You don't have {}", effect_name));
                }
            } else {
                // Clear all effects
                let effect_ids: Vec<i32> = if let Ok(effects) = world.get::<&ActiveEffects>(entity) {
                    effects.effects.keys().copied().collect()
                } else {
                    Vec::new()
                };
                if effect_ids.is_empty() {
                    send_message(world, entity, "No active effects to clear.");
                    return;
                }
                if let Ok(mut effects) = world.get::<&mut ActiveEffects>(entity) {
                    effects.effects.clear();
                }
                if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
                    for eff_id in &effect_ids {
                        let _ = sender.0.send(InternalPacket::RemoveMobEffect {
                            entity_id: eid,
                            effect_id: *eff_id,
                        });
                    }
                }
                send_message(world, entity, &format!("Cleared {} effects", effect_ids.len()));
            }
        }
        _ => {
            send_message(world, entity, "Usage: /effect <give|clear> ...");
        }
    }
}

/// /potion <player> <potion_name> — give a potion to a player
fn cmd_potion(world: &mut World, entity: hecs::Entity, args: &str) {
    if !is_op(world, entity) {
        send_message(world, entity, "You don't have permission to use this command.");
        return;
    }
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        send_message(world, entity, "Usage: /potion <player> <potion_name>");
        return;
    }
    let target_name = parts[0];
    let potion_name = parts[1];

    let potion_index = match pickaxe_data::potion_name_to_index(potion_name) {
        Some(idx) => idx,
        None => {
            send_message(world, entity, &format!("Unknown potion: {}", potion_name));
            return;
        }
    };

    let potion_id = match pickaxe_data::item_name_to_id("potion") {
        Some(id) => id,
        None => {
            send_message(world, entity, "Potion item not found in data.");
            return;
        }
    };

    // Find target player
    let target = {
        let mut found = None;
        for (e, profile) in world.query::<&Profile>().iter() {
            if profile.0.name.eq_ignore_ascii_case(target_name) {
                found = Some(e);
                break;
            }
        }
        found
    };
    let target = match target {
        Some(t) => t,
        None => {
            send_message(world, entity, &format!("Player '{}' not found.", target_name));
            return;
        }
    };

    // Give the potion item (using damage field to store potion type index)
    let item = ItemStack {
        item_id: potion_id,
        count: 1,
        damage: potion_index,
        max_damage: 0,
        enchantments: Vec::new(),
    };
    let slot_update = {
        let mut inv = match world.get::<&mut Inventory>(target) {
            Ok(inv) => inv,
            Err(_) => {
                send_message(world, entity, "Could not access player inventory.");
                return;
            }
        };
        if let Some(slot) = inv.find_slot_for_item(potion_id, 1) {
            // Potions don't stack (different potion types), always place in empty slot
            if inv.slots[slot].is_none() {
                inv.slots[slot] = Some(item);
            } else {
                // Find empty slot instead since potions with different damage values shouldn't stack
                if let Some(empty) = inv.slots[9..45].iter().position(|s| s.is_none()) {
                    inv.slots[9 + empty] = Some(item);
                } else {
                    send_message(world, entity, "Player's inventory is full.");
                    return;
                }
            }
            inv.state_id = inv.state_id.wrapping_add(1);
            Some((slot, inv.slots[slot].clone(), inv.state_id))
        } else {
            None
        }
    };

    if let Some((slot, slot_item, state_id)) = slot_update {
        if let Ok(sender) = world.get::<&ConnectionSender>(target) {
            let _ = sender.0.send(InternalPacket::SetContainerSlot {
                window_id: 0,
                state_id,
                slot: slot as i16,
                item: slot_item,
            });
        }
        let display_name = pickaxe_data::potion_index_to_name(potion_index).unwrap_or(potion_name);
        send_message(world, entity, &format!("Gave {} a potion of {}", target_name, display_name));
    } else {
        send_message(world, entity, "Player's inventory is full.");
    }
}

fn cmd_enchant(world: &mut World, entity: hecs::Entity, args: &str) {
    if !is_op(world, entity) {
        send_message(world, entity, "You don't have permission to use this command.");
        return;
    }

    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() {
        send_message(world, entity, "Usage: /enchant <enchantment> [level]");
        return;
    }

    let ench_name = parts[0].strip_prefix("minecraft:").unwrap_or(parts[0]);
    let level = if parts.len() > 1 {
        parts[1].parse::<i32>().unwrap_or(1).max(1)
    } else {
        1
    };

    let ench_id = match pickaxe_data::enchantment_name_to_id(ench_name) {
        Some(id) => id,
        None => {
            send_message(world, entity, &format!("Unknown enchantment: {}", ench_name));
            return;
        }
    };

    let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
    let slot_idx = 36 + held_slot as usize;

    let slot_update = {
        let mut inv = match world.get::<&mut Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => {
                send_message(world, entity, "No inventory found.");
                return;
            }
        };
        if let Some(ref mut item) = inv.slots[slot_idx] {
            // Add or update enchantment
            if let Some(entry) = item.enchantments.iter_mut().find(|(id, _)| *id == ench_id) {
                entry.1 = level;
            } else {
                item.enchantments.push((ench_id, level));
            }
            inv.state_id = inv.state_id.wrapping_add(1);
            Some((slot_idx, inv.slots[slot_idx].clone(), inv.state_id))
        } else {
            send_message(world, entity, "You must be holding an item to enchant.");
            None
        }
    };

    if let Some((slot, slot_item, state_id)) = slot_update {
        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
            let _ = sender.0.send(InternalPacket::SetContainerSlot {
                window_id: 0,
                state_id,
                slot: slot as i16,
                item: slot_item,
            });
        }
        send_message(world, entity, &format!("Enchanted held item with {} {}", ench_name, level));
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

    // Send BlockEntityData for signs in loaded chunks
    send_sign_block_entities(sender, world_state, center_cx, center_cz, view_distance);
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

    // Send BlockEntityData for signs in newly loaded chunks
    for cx in (new_cx - vd)..=(new_cx + vd) {
        for cz in (new_cz - vd)..=(new_cz + vd) {
            if (cx - old_cx).abs() > vd || (cz - old_cz).abs() > vd {
                send_sign_block_entities_for_chunk(sender, world_state, cx, cz);
            }
        }
    }
}

/// Send BlockEntityData packets for all signs in chunks within the given range.
fn send_sign_block_entities(
    sender: &mpsc::UnboundedSender<InternalPacket>,
    world_state: &WorldState,
    center_cx: i32,
    center_cz: i32,
    view_distance: i32,
) {
    for cx in (center_cx - view_distance)..=(center_cx + view_distance) {
        for cz in (center_cz - view_distance)..=(center_cz + view_distance) {
            send_sign_block_entities_for_chunk(sender, world_state, cx, cz);
        }
    }
}

/// Send BlockEntityData packets for all signs in a specific chunk.
fn send_sign_block_entities_for_chunk(
    sender: &mpsc::UnboundedSender<InternalPacket>,
    world_state: &WorldState,
    chunk_x: i32,
    chunk_z: i32,
) {
    let min_x = chunk_x * 16;
    let min_z = chunk_z * 16;
    for (pos, be) in &world_state.block_entities {
        if pos.x >= min_x && pos.x < min_x + 16
            && pos.z >= min_z && pos.z < min_z + 16
        {
            if let BlockEntity::Sign { .. } = be {
                let nbt = build_sign_update_nbt(be);
                let _ = sender.send(InternalPacket::BlockEntityData {
                    position: *pos,
                    block_entity_type: 7, // sign
                    nbt,
                });
            }
        }
    }
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

/// Build the full equipment list for a player entity.
/// Returns Vec of (equipment_slot, item) pairs for mainhand, offhand, and armor.
fn build_equipment(world: &World, entity: hecs::Entity) -> Vec<(u8, Option<ItemStack>)> {
    let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
    let inv = match world.get::<&Inventory>(entity) {
        Ok(inv) => inv,
        Err(_) => return Vec::new(),
    };
    vec![
        (0, inv.slots[36 + held_slot as usize].clone()), // mainhand
        (1, inv.slots[45].clone()),                       // offhand
        (2, inv.slots[8].clone()),                        // boots (inv slot 8)
        (3, inv.slots[7].clone()),                        // leggings (inv slot 7)
        (4, inv.slots[6].clone()),                        // chestplate (inv slot 6)
        (5, inv.slots[5].clone()),                        // helmet (inv slot 5)
    ]
}

/// Send a SetEquipment packet for a player to all other players who track them.
fn send_equipment_update(world: &World, entity: hecs::Entity, entity_id: i32) {
    let equipment = build_equipment(world, entity);
    if equipment.is_empty() {
        return;
    }
    let packet = InternalPacket::SetEquipment {
        entity_id,
        equipment,
    };
    broadcast_except(world, entity_id, &packet);
}

/// Damage the held item by `amount`. Breaks it if durability reaches 0.
/// Sends slot update and equipment update to other players.
fn damage_held_item(world: &mut World, entity: hecs::Entity, entity_id: i32, amount: i32) {
    let held_slot = world.get::<&HeldSlot>(entity).map(|h| h.0).unwrap_or(0);
    let inv_slot = 36 + held_slot as usize;
    let (broken, state_id) = {
        let mut inv = match world.get::<&mut Inventory>(entity) {
            Ok(inv) => inv,
            Err(_) => return,
        };
        if let Some(ref mut item) = inv.slots[inv_slot] {
            if item.max_damage > 0 {
                // Unbreaking enchantment: 1/(level+1) chance to consume durability
                let unbreaking = item.enchantment_level(22);
                if unbreaking > 0 && rand::random::<f32>() > 1.0 / (unbreaking as f32 + 1.0) {
                    return;
                }
                item.damage += amount;
                if item.damage >= item.max_damage {
                    inv.set_slot(inv_slot, None);
                    (true, inv.state_id)
                } else {
                    (false, inv.state_id)
                }
            } else {
                return;
            }
        } else {
            return;
        }
    };

    // Send slot update to the player
    if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
        let item = world.get::<&Inventory>(entity)
            .map(|inv| inv.slots[inv_slot].clone())
            .unwrap_or(None);
        let _ = sender.0.send(InternalPacket::SetContainerSlot {
            window_id: 0,
            state_id,
            slot: inv_slot as i16,
            item,
        });
    }

    // If item broke, play break sound
    if broken {
        let pos = world.get::<&Position>(entity).map(|p| p.0).unwrap_or(Vec3d { x: 0.0, y: 0.0, z: 0.0 });
        play_sound_at_entity(world, pos.x, pos.y, pos.z, "entity.item.break", SOUND_PLAYERS, 1.0, 1.0);
    }

    // Broadcast equipment change
    send_equipment_update(world, entity, entity_id);
}

/// SoundSource enum ordinal values matching MC SoundSource.
const SOUND_WEATHER: u8 = 3;
const SOUND_BLOCKS: u8 = 4;
const SOUND_HOSTILE: u8 = 5;
const SOUND_NEUTRAL: u8 = 6;
const SOUND_PLAYERS: u8 = 7;

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

/// Returns true if the item is an ore drop that should be affected by Fortune.
fn is_ore_drop(item_id: i32) -> bool {
    let name = match pickaxe_data::item_id_to_name(item_id) {
        Some(n) => n,
        None => return false,
    };
    matches!(name,
        "diamond" | "emerald" | "coal" | "raw_iron" | "raw_gold" | "raw_copper"
        | "lapis_lazuli" | "redstone" | "nether_quartz" | "amethyst_shard"
    )
}

/// Offset a block position by the given face direction.
/// Build NBT for a sign block entity update (for BlockEntityData packet).
fn build_sign_update_nbt(be: &BlockEntity) -> NbtValue {
    if let BlockEntity::Sign { front_text, back_text, color, has_glowing_text, is_waxed } = be {
        let make_text_nbt = |lines: &[String; 4], col: &str, glowing: bool| -> NbtValue {
            let messages: Vec<NbtValue> = lines.iter().map(|line| {
                if line.is_empty() {
                    NbtValue::String("{\"text\":\"\"}".into())
                } else {
                    NbtValue::String(format!("{{\"text\":\"{}\"}}", line.replace('\\', "\\\\").replace('"', "\\\"")))
                }
            }).collect();
            nbt_compound! {
                "messages" => NbtValue::List(messages),
                "color" => NbtValue::String(col.to_string()),
                "has_glowing_text" => NbtValue::Byte(if glowing { 1 } else { 0 })
            }
        };
        nbt_compound! {
            "front_text" => make_text_nbt(front_text, color, *has_glowing_text),
            "back_text" => make_text_nbt(back_text, color, false),
            "is_waxed" => NbtValue::Byte(if *is_waxed { 1 } else { 0 })
        }
    } else {
        NbtValue::Compound(Vec::new())
    }
}

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
    let simple_cmds = ["gamemode", "gm", "tp", "teleport", "give", "kill", "say", "help", "effect", "potion", "enchant"];
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
