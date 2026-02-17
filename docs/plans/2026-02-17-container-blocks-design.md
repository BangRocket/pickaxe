# M11: Container Blocks — Chests, Furnaces, Crafting Tables

## Goal

Add container blocks with interactive GUIs: chests for storage, furnaces for smelting, and crafting tables for item crafting. This is the foundation for item progression — players can store items, smelt ores, and craft tools.

## Architecture

### Block Entities

Blocks that carry persistent state beyond their block ID. Stored in `WorldState` as `HashMap<BlockPos, BlockEntity>`.

```rust
enum BlockEntity {
    Chest { inventory: [Option<ItemStack>; 27] },
    Furnace {
        input: Option<ItemStack>,
        fuel: Option<ItemStack>,
        output: Option<ItemStack>,
        burn_time: i16,      // ticks remaining on current fuel
        burn_duration: i16,  // total burn time of current fuel item
        cook_progress: i16,  // ticks cooked so far
        cook_total: i16,     // ticks needed (200 for standard smelting)
    },
}
```

Persisted in chunk NBT save path (block_entities array). Created when block is placed, removed when block is broken (drops contents).

### Container Menu System

When a player right-clicks a container block, the server:
1. Creates a `Menu` attached to the player (ECS component `OpenContainer`)
2. Sends `OpenScreen` packet with the menu type and title
3. Sends `SetContainerContent` with all slots (container + player inventory)

The `Menu` enum determines slot layout and window type:

```rust
enum Menu {
    Chest { pos: BlockPos },
    Furnace { pos: BlockPos },
    CraftingTable { grid: [Option<ItemStack>; 9], result: Option<ItemStack> },
}
```

- `container_id` is per-player, incremented 1-255 (wraps)
- Crafting table grid is ephemeral (not a block entity) — items drop on close

### Container Click Handler

One shared function handles all click modes for all container types. Maps "window slot index" to either a block entity slot or a player inventory slot, then performs the action.

**Supported click modes:**
- Mode 0 (PICKUP): Left click = take/place full stack, right click = take/place half
- Mode 1 (QUICK_MOVE): Shift-click transfers between container and player inventory
- Mode 2 (SWAP): Number key swaps with hotbar slot
- Mode 4 (THROW): Q key drops from slot
- Mode 6 (PICKUP_ALL): Double-click collects matching items

**Deferred:** Mode 5 (QUICK_CRAFT / drag) — server rejects and resyncs. Functional but slightly annoying.

### State ID Synchronization

The server maintains a per-container `state_id` counter. Each modification increments it. The client echoes state_id in ContainerClick packets. On mismatch, the server sends full `SetContainerContent` to resync (prevents race conditions).

### Furnace Ticking

Runs every server tick, but only for lit furnaces (burn_time > 0) or furnaces with valid fuel + input:

1. Decrement `burn_time`; when 0, try consuming next fuel item
2. Increment `cook_progress` while fuel burning and input valid
3. When `cook_progress` reaches `cook_total` (200), move result to output
4. Send `SetContainerData` to any player with this furnace open

Standard fuel values: coal=1600, planks=300, sticks=100, lava_bucket=20000.

### Crafting Recipes

Hardcoded recipe table for M11 (~20 essential recipes):
- Planks, sticks, crafting table, furnace, chest
- Wooden/stone tools (pickaxe, axe, shovel, sword)
- Torches

Recipe lookup: match 3x3 grid pattern against known recipes. Result slot is read-only — taking from it consumes grid items. Server recalculates result whenever grid changes.

## Protocol (MC 1.21.1 / Protocol 767)

### Clientbound Packets

| Packet | ID | Fields |
|--------|----|--------|
| OpenScreen | 0x33 | VarInt container_id, VarInt menu_type, Component title |
| SetContainerContent | 0x13 | u8 container_id, VarInt state_id, Slot[] items, Slot carried |
| SetContainerSlot | 0x15 | i8 container_id, VarInt state_id, i16 slot, Slot item |
| SetContainerData | 0x14 | u8 container_id, i16 property, i16 value |
| ContainerClose | 0x12 | VarInt container_id |

### Serverbound Packets

| Packet | ID | Fields |
|--------|----|--------|
| ContainerClick | 0x0E | u8 window_id, VarInt state_id, i16 slot, i8 button, VarInt mode, ChangedSlot[] slots, Slot cursor |
| CloseContainer | 0x0F | u8 container_id |

### Window Type IDs (MenuType Registry)

| Type | ID |
|------|----|
| generic_9x3 (chest) | 2 |
| furnace | 14 |
| crafting | 12 |

### Slot Numbering

**Chest (generic_9x3):** 0-26 container, 27-53 player main, 54-62 hotbar
**Furnace:** 0 input, 1 fuel, 2 output, 3-29 player main, 30-38 hotbar
**Crafting Table:** 0 result, 1-9 grid (3x3 row-major), 10-36 player main, 37-45 hotbar

### Furnace Properties (SetContainerData)

| Property | Meaning |
|----------|---------|
| 0 | Burn time remaining (ticks) |
| 1 | Total burn duration of current fuel |
| 2 | Cook progress (ticks) |
| 3 | Cook total time (typically 200) |

## Lua Integration

### Events
- `container_open` (cancellable): `{name, block_type, x, y, z}`
- `container_close`: `{name, block_type, x, y, z}`

### API
- `pickaxe.world.get_block_entity(x, y, z)` → table with inventory contents or nil
- `pickaxe.world.set_block_entity(x, y, z, data)` → modify block entity

## Scope Cuts

- No double chests (neighbor detection complexity)
- No drag click mode (resyncs on attempt)
- No recipe book UI
- No hopper/dropper/dispenser
- No chest open/close animation or sound
- Minimal recipe set (~20 recipes)

## Files to Modify

| File | Changes |
|------|---------|
| `crates/pickaxe-server/src/ecs.rs` | OpenContainer, Menu, BlockEntity types |
| `crates/pickaxe-server/src/tick.rs` | Block entity storage, container click handler, furnace tick, crafting logic, use-item-on handling |
| `crates/pickaxe-protocol-core/src/packets.rs` | OpenScreen, SetContainerContent, ContainerClick, CloseContainer, SetContainerData variants |
| `crates/pickaxe-protocol-v1_21/src/adapter.rs` | Encode/decode new packets |
| `crates/pickaxe-server/src/bridge.rs` | Block entity Lua API |
| `crates/pickaxe-data/` | Fuel values, smelting recipes, crafting recipes |
| `lua/vanilla/init.lua` | Container event handlers |
