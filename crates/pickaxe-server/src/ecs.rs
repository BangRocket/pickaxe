use pickaxe_protocol_core::InternalPacket;
use pickaxe_types::{BlockPos, GameMode, GameProfile, ItemStack, Vec3d};
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;
use uuid::Uuid;

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

/// Entity velocity vector.
pub struct Velocity(pub Vec3d);

/// Entity's UUID (for non-player entities that need a UUID distinct from Profile).
pub struct EntityUuid(pub Uuid);

/// Marks an entity as a dropped item.
pub struct ItemEntity {
    pub item: ItemStack,
    pub pickup_delay: u32,
    pub age: u64,
}

/// Previous rotation — used to detect rotation changes.
pub struct PreviousRotation {
    pub yaw: f32,
    pub pitch: f32,
}

/// Player inventory: 46 slots.
/// Slots 0: crafting output, 1-4: crafting input, 5-8: armor
/// Slots 9-35: main inventory, 36-44: hotbar, 45: offhand
pub struct Inventory {
    pub slots: [Option<ItemStack>; 46],
    pub state_id: i32,
}

impl Inventory {
    pub fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            state_id: 1,
        }
    }

    /// Get the item in the given hotbar slot (0-8).
    pub fn held_item(&self, hotbar_slot: u8) -> &Option<ItemStack> {
        &self.slots[36 + hotbar_slot as usize]
    }

    /// Set a slot and increment state_id.
    pub fn set_slot(&mut self, index: usize, item: Option<ItemStack>) {
        if index < 46 {
            self.slots[index] = item;
            self.state_id = self.state_id.wrapping_add(1);
        }
    }

    /// Find a slot to add an item to: first tries stacking into an existing
    /// matching slot, then finds the first empty slot. Searches hotbar (36-44)
    /// first, then main inventory (9-35). Returns the slot index if found.
    pub fn find_slot_for_item(&self, item_id: i32, max_stack: i32) -> Option<usize> {
        // Try stacking into existing slots: hotbar first, then main
        for i in (36..=44).chain(9..=35) {
            if let Some(ref existing) = self.slots[i] {
                if existing.item_id == item_id && (existing.count as i32) < max_stack {
                    return Some(i);
                }
            }
        }
        // Then find an empty slot
        for i in (36..=44).chain(9..=35) {
            if self.slots[i].is_none() {
                return Some(i);
            }
        }
        None
    }

    /// Convert to packet format.
    pub fn to_slot_vec(&self) -> Vec<Option<ItemStack>> {
        self.slots.to_vec()
    }
}

/// Currently selected hotbar slot (0-8).
pub struct HeldSlot(pub u8);

/// Player health state.
pub struct Health {
    pub current: f32,
    pub max: f32,
    pub invulnerable_ticks: i32,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            current: 20.0,
            max: 20.0,
            invulnerable_ticks: 0,
        }
    }
}

/// Player hunger, saturation, and exhaustion state.
pub struct FoodData {
    pub food_level: i32,
    pub saturation: f32,
    pub exhaustion: f32,
    pub tick_timer: u32,
}

impl Default for FoodData {
    fn default() -> Self {
        Self {
            food_level: 20,
            saturation: 5.0,
            exhaustion: 0.0,
            tick_timer: 0,
        }
    }
}

/// Accumulated fall distance for fall damage calculation.
pub struct FallDistance(pub f32);

/// Tracks sprint/sneak state from PlayerCommand packets.
pub struct MovementState {
    pub sprinting: bool,
    pub sneaking: bool,
}

/// What type of container menu a player has open.
#[derive(Debug, Clone)]
pub enum Menu {
    Chest { pos: BlockPos },
    Furnace { pos: BlockPos },
    CraftingTable {
        grid: [Option<ItemStack>; 9],
        result: Option<ItemStack>,
    },
}

/// Tracks the container a player currently has open.
pub struct OpenContainer {
    pub container_id: u8,
    pub menu: Menu,
    pub state_id: i32,
}

/// Tracks a player actively eating food.
pub struct EatingState {
    pub remaining_ticks: i32,
    pub hand: i32, // 0=main, 1=off
    pub item_id: i32,
    pub nutrition: i32,
    pub saturation_modifier: f32,
}

/// Tracks attack cooldown for combat (MC: attackStrengthTicker).
/// Counts up from 0 each tick; full strength at >= 10 ticks (0.5s).
pub struct AttackCooldown {
    pub ticks_since_last_attack: u32,
}

impl Default for AttackCooldown {
    fn default() -> Self {
        Self {
            ticks_since_last_attack: 100,
        }
    }
}

/// Tracks a button that needs to auto-reset after a delay.
pub struct ButtonTimer {
    pub position: BlockPos,
    pub remaining_ticks: u32,
}

/// Player experience data.
pub struct ExperienceData {
    pub level: i32,
    pub progress: f32,    // 0.0 to 1.0
    pub total_xp: i32,
}

impl Default for ExperienceData {
    fn default() -> Self {
        Self { level: 0, progress: 0.0, total_xp: 0 }
    }
}

/// Tracks a block a player is currently breaking in survival mode.
pub struct BreakingBlock {
    pub position: BlockPos,
    pub block_state: i32,
    pub started_tick: u64,
    pub total_ticks: u64,
    pub last_stage: i8,
}

/// Player's bed spawn point for respawning.
pub struct SpawnPoint {
    pub position: BlockPos,
    pub yaw: f32,
}

/// Tracks that a player is currently sleeping in a bed.
pub struct SleepingState {
    pub bed_pos: BlockPos,
    pub sleep_timer: u32, // ticks spent sleeping; skip at 100
}

/// Marks an entity as a mob with AI.
pub struct MobEntity {
    pub mob_type: i32,   // entity type ID (from pickaxe_data MOB_* constants)
    pub health: f32,
    pub max_health: f32,
    pub target: Option<hecs::Entity>, // current attack target (for hostile mobs)
    pub ai_state: MobAiState,
    pub ai_timer: u32,          // ticks until next AI decision
    pub ambient_sound_timer: u32, // ticks until next ambient sound
    pub no_damage_ticks: i32,   // invulnerability after hit
    pub fuse_timer: i32,        // creeper fuse countdown (-1 = not fusing, 0 = explode)
    pub attack_cooldown: u32,   // skeleton arrow / generic attack cooldown
}

/// Arrow projectile component.
pub struct ArrowEntity {
    pub damage: f32,         // base damage (2.0 for normal arrows)
    pub owner: Option<hecs::Entity>, // who shot the arrow
    pub in_ground: bool,     // arrow is embedded in a block
    pub age: u32,            // ticks since spawn, despawn at 1200 (60 seconds)
    pub is_critical: bool,   // crit arrow (full bow draw)
    pub from_player: bool,   // can be picked up if from player
}

/// Tracks when a player is drawing a bow.
pub struct BowDrawState {
    pub start_tick: u64,     // when the draw started
    pub hand: i32,           // which hand holds the bow
}

/// Tracks when a player is actively blocking with a shield.
pub struct BlockingState {
    pub start_tick: u64,     // when blocking started (effective after 5 ticks)
    pub hand: i32,           // 0=main, 1=off
}

/// Shield use cooldown after being hit by an axe.
/// Cannot use shield while cooldown_ticks > 0.
pub struct ShieldCooldown {
    pub remaining_ticks: u32,
}

/// Fishing bobber entity component.
pub struct FishingBobber {
    pub owner: hecs::Entity,          // player who cast the rod
    pub state: FishingBobberState,
    pub time_until_lured: i32,        // ticks until fish appears (100-600)
    pub time_until_hooked: i32,       // ticks until fish bites (20-80)
    pub nibble: i32,                  // ticks remaining for bite window (20-40)
    pub age: u32,                     // ticks since spawn
    pub hooked_entity: Option<hecs::Entity>,
}

/// Fishing bobber state machine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FishingBobberState {
    Flying,
    Bobbing,
    HookedInEntity,
}

/// Player air supply for drowning mechanics.
/// Max is 300 (15 seconds), decreases by 1/tick when submerged,
/// increases by 4/tick when out of water.
pub struct AirSupply {
    pub current: i32,
    pub max: i32,
}

impl Default for AirSupply {
    fn default() -> Self {
        Self { current: 300, max: 300 }
    }
}

/// A single active status effect on an entity.
#[derive(Debug, Clone)]
pub struct EffectInstance {
    pub effect_id: i32,      // 0-indexed registry ID
    pub amplifier: i32,      // 0 = level I, 1 = level II, etc.
    pub duration: i32,       // ticks remaining, -1 = infinite
    pub ambient: bool,       // subtle particles (from beacon)
    pub show_particles: bool,
    pub show_icon: bool,
}

/// Collection of active status effects on an entity.
/// Keyed by effect_id for fast lookup and replacement.
pub struct ActiveEffects {
    pub effects: HashMap<i32, EffectInstance>,
}

impl ActiveEffects {
    pub fn new() -> Self {
        Self {
            effects: HashMap::new(),
        }
    }
}

/// Current AI behavior state for a mob.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MobAiState {
    Idle,
    Wandering,
    Chasing,
    Fleeing,    // bat: fly away; creeper: retreat after failed fuse
}
