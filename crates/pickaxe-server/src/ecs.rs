use pickaxe_protocol_core::InternalPacket;
use pickaxe_types::{BlockPos, GameMode, GameProfile, ItemStack, Vec3d};
use std::collections::HashSet;
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

/// Tracks a block a player is currently breaking in survival mode.
pub struct BreakingBlock {
    pub position: BlockPos,
    pub block_state: i32,
    pub started_tick: u64,
    pub total_ticks: u64,
    pub last_stage: i8,
}
