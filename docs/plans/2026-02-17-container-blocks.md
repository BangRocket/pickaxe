# M11: Container Blocks Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add chests, furnaces, and crafting tables with interactive GUIs, server-authoritative click handling, furnace smelting, and basic crafting recipes.

**Architecture:** Block entities stored in WorldState HashMap. Container menus are ECS components on players. One shared click handler maps window slots to block entity or player inventory slots. Furnaces tick on the server. Crafting uses a hardcoded recipe table.

**Tech Stack:** Rust (hecs ECS, tokio mpsc), pickaxe-protocol-core (slot codec), pickaxe-data (recipes, fuel)

---

### Task 1: Block Entity Storage in WorldState

Add `HashMap<BlockPos, BlockEntity>` to WorldState for storing container block data.

**Files:**
- Modify: `crates/pickaxe-server/src/tick.rs` (WorldState struct ~line 320)

**Step 1: Define BlockEntity enum and add to WorldState**

In `tick.rs`, add above the WorldState struct:

```rust
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
```

Add to WorldState struct:
```rust
pub block_entities: HashMap<BlockPos, BlockEntity>,
```

Initialize in `WorldState::new()`:
```rust
block_entities: HashMap::new(),
```

Add helper methods to WorldState impl:
```rust
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
```

**Step 2: Hook block entity creation into block placement**

In `process_packet()` BlockPlace handler (after `world_state.set_block(&target, block_id)`), add:

```rust
// Create block entity for container blocks
let block_name = pickaxe_data::block_state_to_name(block_id).unwrap_or("");
match block_name {
    "chest" => {
        world_state.set_block_entity(target, BlockEntity::Chest {
            inventory: std::array::from_fn(|_| None),
        });
    }
    "furnace" | "lit_furnace" => {
        world_state.set_block_entity(target, BlockEntity::Furnace {
            input: None, fuel: None, output: None,
            burn_time: 0, burn_duration: 0, cook_progress: 0, cook_total: 200,
        });
    }
    "crafting_table" => {} // No block entity — grid is ephemeral on the Menu
    _ => {}
}
```

**Step 3: Hook block entity removal into block breaking**

In the block break handler (complete_block_break or wherever blocks are broken), after removing the block, add:

```rust
// Remove block entity and drop contents
if let Some(block_entity) = world_state.remove_block_entity(&block_pos) {
    let items: Vec<ItemStack> = match block_entity {
        BlockEntity::Chest { inventory } => {
            inventory.into_iter().flatten().collect()
        }
        BlockEntity::Furnace { input, fuel, output, .. } => {
            [input, fuel, output].into_iter().flatten().collect()
        }
    };
    for item in items {
        spawn_item_entity(world, world_state, next_eid,
            block_pos.x as f64 + 0.5, block_pos.y as f64 + 0.5, block_pos.z as f64 + 0.5,
            item, scripting);
    }
}
```

**Step 4: Build and verify**

Run: `cargo build`
Expected: Compiles with no new errors.

Run: `cargo test`
Expected: All existing tests pass.

**Step 5: Commit**

```
feat(m11): add block entity storage to WorldState
```

---

### Task 2: Container Packets — OpenScreen, ContainerClick, CloseContainer, SetContainerData

Add the missing packet variants for container interaction.

**Files:**
- Modify: `crates/pickaxe-protocol-core/src/packets.rs`
- Modify: `crates/pickaxe-protocol-v1_21/src/adapter.rs`

**Step 1: Add packet variants to InternalPacket**

In `packets.rs`, add these variants to the `InternalPacket` enum:

```rust
/// Open Screen (0x33 CB) — open a container GUI.
OpenScreen {
    container_id: i32,
    menu_type: i32,
    title: TextComponent,
},

/// Container Close (0x12 CB) — server tells client to close.
ContainerClose {
    container_id: i32,
},

/// Set Container Data (0x14 CB) — furnace progress bars.
SetContainerData {
    container_id: u8,
    property: i16,
    value: i16,
},

/// Container Click (0x0E SB) — client clicked in a container.
ContainerClick {
    window_id: u8,
    state_id: i32,
    slot: i16,
    button: i8,
    mode: i32,
    changed_slots: Vec<(i16, Option<ItemStack>)>,
    carried_item: Option<ItemStack>,
},

/// Close Container (0x0F SB) — client closed a container.
ClientCloseContainer {
    container_id: u8,
},
```

**Step 2: Add encoding in adapter.rs**

Add packet ID constants:
```rust
const PLAY_OPEN_SCREEN: i32 = 0x33;
const PLAY_CONTAINER_CLOSE: i32 = 0x12;
const PLAY_SET_CONTAINER_DATA: i32 = 0x14;
```

Add encoding cases in `encode_play()`:
```rust
InternalPacket::OpenScreen { container_id, menu_type, title } => {
    write_varint(&mut buf, PLAY_OPEN_SCREEN);
    write_varint(&mut buf, *container_id);
    write_varint(&mut buf, *menu_type);
    title.write_nbt(&mut buf);
}
InternalPacket::ContainerClose { container_id } => {
    write_varint(&mut buf, PLAY_CONTAINER_CLOSE);
    write_varint(&mut buf, *container_id);
}
InternalPacket::SetContainerData { container_id, property, value } => {
    write_varint(&mut buf, PLAY_SET_CONTAINER_DATA);
    buf.put_u8(*container_id);
    buf.put_i16(*property);
    buf.put_i16(*value);
}
```

**Step 3: Add decoding in adapter.rs**

Add serverbound decoding in `decode_play()`:
```rust
0x0E => {
    // Container Click
    let window_id = data.get_u8();
    let state_id = read_varint(data)?;
    let slot = data.get_i16();
    let button = data.get_i8();
    let mode = read_varint(data)?;
    let count = read_varint(data)?;
    let mut changed_slots = Vec::new();
    for _ in 0..count {
        let loc = data.get_i16();
        let item = read_slot(data)?;
        changed_slots.push((loc, item));
    }
    let carried_item = read_slot(data)?;
    Ok(InternalPacket::ContainerClick {
        window_id, state_id, slot, button, mode, changed_slots, carried_item,
    })
}
0x0F => {
    // Close Container
    let container_id = data.get_u8();
    Ok(InternalPacket::ClientCloseContainer { container_id })
}
```

**Step 4: TextComponent NBT encoding**

The OpenScreen title field requires NBT-encoded text component (not JSON). Check if `TextComponent::write_nbt()` exists. If not, add a minimal implementation that writes a String NBT tag:

In `crates/pickaxe-types/src/lib.rs`, add to TextComponent impl:
```rust
pub fn write_nbt(&self, buf: &mut BytesMut) {
    // Write as NBT String tag (no root name for network NBT)
    use pickaxe_nbt::NbtValue;
    let nbt = NbtValue::String(self.text.clone());
    nbt.write_root_network(buf);
}
```

If `write_root_network` doesn't exist on NbtValue, check for the network NBT write method (no root name, just type + payload). It may be called differently — check `pickaxe-nbt/src/nbt.rs`.

**Step 5: Build and verify**

Run: `cargo build`
Expected: Compiles.

Run: `cargo test`
Expected: All tests pass.

**Step 6: Commit**

```
feat(m11): add container packets (OpenScreen, ContainerClick, CloseContainer, SetContainerData)
```

---

### Task 3: Menu System and Container Opening

Add the Menu/OpenContainer ECS component and the logic to open containers when right-clicking.

**Files:**
- Modify: `crates/pickaxe-server/src/ecs.rs`
- Modify: `crates/pickaxe-server/src/tick.rs`

**Step 1: Add Menu and OpenContainer types**

In `ecs.rs`, add:

```rust
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
```

**Step 2: Add container opening to block interaction**

In `tick.rs` process_packet(), at the TOP of the BlockPlace handler (before the held item lookup), add a check for interactable blocks:

```rust
InternalPacket::BlockPlace { position, face, sequence, .. } => {
    // Check if the target block is a container — open it instead of placing
    let target_block = world_state.get_block(&position);
    let target_name = pickaxe_data::block_state_to_name(target_block).unwrap_or("");

    let is_container = matches!(target_name, "chest" | "furnace" | "lit_furnace" | "crafting_table");

    // If sneaking, bypass container open and place block instead
    let sneaking = world.get::<&MovementState>(entity).map(|m| m.sneaking).unwrap_or(false);

    if is_container && !sneaking {
        // Fire container_open event
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

        // Ack the block change (even though we didn't place)
        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
            let _ = sender.0.send(InternalPacket::AcknowledgeBlockChange { sequence });
        }
        return;
    }

    // ... existing block placement code ...
```

**Step 3: Implement open_container function**

```rust
fn open_container(
    world: &mut World,
    world_state: &WorldState,
    entity: hecs::Entity,
    pos: &BlockPos,
    block_name: &str,
) {
    // Determine menu type and container title
    let (menu_type, title, menu) = match block_name {
        "chest" => (2, "Chest", Menu::Chest { pos: *pos }),
        "furnace" | "lit_furnace" => (14, "Furnace", Menu::Furnace { pos: *pos }),
        "crafting_table" => (12, "Crafting", Menu::CraftingTable {
            grid: std::array::from_fn(|_| None),
            result: None,
        }),
        _ => return,
    };

    // Assign container ID (increment per player, wrap at 255)
    let container_id = {
        let old = world.get::<&OpenContainer>(entity).map(|c| c.container_id).unwrap_or(0);
        old.wrapping_add(1).max(1) // 1-255, never 0 (0 = player inventory)
    };

    // Build slot list for SetContainerContent
    let (slots, player_inv_start) = build_container_slots(world_state, world, entity, &menu);

    let open_container = OpenContainer {
        container_id,
        menu,
        state_id: 1,
    };

    // Send packets
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

    // Attach or replace the OpenContainer component
    let _ = world.insert_one(entity, open_container);
}
```

**Step 4: Implement build_container_slots helper**

This maps container + player inventory into the flat slot array the client expects:

```rust
fn build_container_slots(
    world_state: &WorldState,
    world: &World,
    entity: hecs::Entity,
    menu: &Menu,
) -> (Vec<Option<ItemStack>>, usize) {
    let player_inv = world.get::<&Inventory>(entity).ok();

    match menu {
        Menu::Chest { pos } => {
            // 27 chest slots + 27 main inv + 9 hotbar = 63
            let mut slots = Vec::with_capacity(63);
            if let Some(BlockEntity::Chest { inventory }) = world_state.get_block_entity(pos) {
                slots.extend_from_slice(inventory);
            } else {
                slots.resize(27, None);
            }
            // Player main inventory (slots 9-35) then hotbar (36-44)
            if let Some(inv) = &player_inv {
                for i in 9..36 { slots.push(inv.slots[i].clone()); }
                for i in 36..45 { slots.push(inv.slots[i].clone()); }
            } else {
                slots.resize(63, None);
            }
            (slots, 27)
        }
        Menu::Furnace { pos } => {
            // 3 furnace slots + 27 main inv + 9 hotbar = 39
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
            (slots, 3)
        }
        Menu::CraftingTable { grid, result } => {
            // 1 result + 9 grid + 27 main inv + 9 hotbar = 46
            let mut slots = Vec::with_capacity(46);
            slots.push(result.clone());
            for item in grid { slots.push(item.clone()); }
            if let Some(inv) = &player_inv {
                for i in 9..36 { slots.push(inv.slots[i].clone()); }
                for i in 36..45 { slots.push(inv.slots[i].clone()); }
            } else {
                slots.resize(46, None);
            }
            (slots, 10)
        }
    }
}
```

**Step 5: Handle CloseContainer packet**

In `process_packet()`, add a handler for `ClientCloseContainer`:

```rust
InternalPacket::ClientCloseContainer { container_id } => {
    // Drop crafting grid items if it was a crafting table
    if let Ok(open) = world.get::<&OpenContainer>(entity) {
        if open.container_id == *container_id {
            if let Menu::CraftingTable { ref grid, .. } = open.menu {
                let name = world.get::<&Profile>(entity).map(|p| p.0.name.clone()).unwrap_or_default();
                let pos = world.get::<&Position>(entity).map(|p| p.0).unwrap_or(Vec3d::new(0.0, 0.0, 0.0));
                let items: Vec<ItemStack> = grid.iter().filter_map(|s| s.clone()).collect();
                // Need to drop after removing component, so collect first
                let items_to_drop = items;
                let drop_pos = pos;
                // Remove component
                let _ = world.remove_one::<OpenContainer>(entity);
                // Drop items
                for item in items_to_drop {
                    spawn_item_entity(world, world_state, next_eid,
                        drop_pos.x, drop_pos.y + 1.0, drop_pos.z,
                        item, scripting);
                }
                // Fire event
                scripting.fire_event_in_context(
                    "container_close",
                    &[("name", &name), ("block_type", "crafting_table")],
                    world as *mut _ as *mut (),
                    world_state as *mut _ as *mut (),
                );
                return;
            }
        }
    }
    // For non-crafting containers, just remove component and fire event
    let block_type = if let Ok(open) = world.get::<&OpenContainer>(entity) {
        match &open.menu {
            Menu::Chest { .. } => "chest",
            Menu::Furnace { .. } => "furnace",
            Menu::CraftingTable { .. } => "crafting_table",
        }.to_string()
    } else { return; };
    let name = world.get::<&Profile>(entity).map(|p| p.0.name.clone()).unwrap_or_default();
    let _ = world.remove_one::<OpenContainer>(entity);
    scripting.fire_event_in_context(
        "container_close",
        &[("name", &name), ("block_type", &block_type)],
        world as *mut _ as *mut (),
        world_state as *mut _ as *mut (),
    );
}
```

**Step 6: Close container on disconnect**

In the disconnect handler, remove OpenContainer and drop crafting items (same logic as close).

**Step 7: Build and verify**

Run: `cargo build`
Run: `cargo test`

**Step 8: Commit**

```
feat(m11): add container menu system with chest/furnace/crafting table opening
```

---

### Task 4: Container Click Handler

Handle all ContainerClick modes: pickup, shift-click, swap, throw, double-click.

**Files:**
- Modify: `crates/pickaxe-server/src/tick.rs`

**Step 1: Add ContainerClick to process_packet**

```rust
InternalPacket::ContainerClick { window_id, state_id, slot, button, mode, changed_slots, carried_item } => {
    handle_container_click(world, world_state, entity, *window_id, *state_id, *slot, *button, *mode, changed_slots, carried_item, scripting);
}
```

**Step 2: Implement slot mapping**

Create a helper that maps a window slot index to either a container location or player inventory location:

```rust
enum SlotTarget {
    Container(usize),       // Index in block entity inventory
    PlayerInventory(usize), // Index in player Inventory.slots
    CraftResult,            // Crafting table result slot (read-only take)
    CraftGrid(usize),       // Crafting table grid slot (0-8)
}

fn map_slot(menu: &Menu, window_slot: i16) -> Option<SlotTarget> {
    let s = window_slot as usize;
    match menu {
        Menu::Chest { .. } => {
            if s < 27 { Some(SlotTarget::Container(s)) }
            else if s < 54 { Some(SlotTarget::PlayerInventory(s - 27 + 9)) } // main inv
            else if s < 63 { Some(SlotTarget::PlayerInventory(s - 54 + 36)) } // hotbar
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
```

**Step 3: Implement get/set slot helpers using SlotTarget**

```rust
fn get_container_slot(
    world_state: &WorldState,
    world: &World,
    entity: hecs::Entity,
    menu: &Menu,
    target: &SlotTarget,
) -> Option<ItemStack> {
    match target {
        SlotTarget::Container(idx) => {
            match menu {
                Menu::Chest { pos } => {
                    if let Some(BlockEntity::Chest { inventory }) = world_state.get_block_entity(pos) {
                        inventory[*idx].clone()
                    } else { None }
                }
                Menu::Furnace { pos } => {
                    if let Some(BlockEntity::Furnace { input, fuel, output, .. }) = world_state.get_block_entity(pos) {
                        match idx { 0 => input.clone(), 1 => fuel.clone(), 2 => output.clone(), _ => None }
                    } else { None }
                }
                _ => None,
            }
        }
        SlotTarget::PlayerInventory(idx) => {
            world.get::<&Inventory>(entity).ok().and_then(|inv| inv.slots[*idx].clone())
        }
        SlotTarget::CraftResult => {
            if let Menu::CraftingTable { result, .. } = menu { result.clone() } else { None }
        }
        SlotTarget::CraftGrid(idx) => {
            if let Menu::CraftingTable { grid, .. } = menu { grid[*idx].clone() } else { None }
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
        _ => {}
    }
}
```

**Step 4: Implement handle_container_click**

This is the core click handler. Start with Mode 0 (PICKUP) and Mode 1 (QUICK_MOVE):

```rust
fn handle_container_click(
    world: &mut World,
    world_state: &mut WorldState,
    entity: hecs::Entity,
    window_id: u8,
    client_state_id: i32,
    slot: i16,
    button: i8,
    mode: i32,
    changed_slots: &[(i16, Option<ItemStack>)],
    carried_item: &Option<ItemStack>,
    scripting: &ScriptRuntime,
) {
    // Get the open container
    let (container_id, mut menu) = match world.remove_one::<OpenContainer>(entity) {
        Ok(oc) => (oc.container_id, oc.menu),
        Err(_) => return,
    };

    if window_id != container_id {
        let _ = world.insert_one(entity, OpenContainer { container_id, menu, state_id: client_state_id });
        return;
    }

    // Track a cursor item (carried by mouse)
    // For simplicity, we trust the client's carried_item for now and validate via changed_slots
    // A more robust approach would track cursor server-side

    match mode {
        0 => { // PICKUP (left/right click)
            if slot == -999 {
                // Click outside — drop carried item (TODO)
            } else if let Some(target) = map_slot(&menu, slot) {
                // Left click: swap cursor and slot
                // Right click: place one / pick up half
                // For M11, accept the client's proposed changes
                for (changed_slot, changed_item) in changed_slots {
                    if let Some(t) = map_slot(&menu, *changed_slot) {
                        set_container_slot(world_state, world, entity, &mut menu, &t, changed_item.clone());
                    }
                }
                // Handle crafting result take
                if matches!(target, SlotTarget::CraftResult) {
                    if let Menu::CraftingTable { ref mut grid, ref mut result } = menu {
                        // Consume one of each grid item
                        for slot in grid.iter_mut() {
                            if let Some(ref mut item) = slot {
                                item.count -= 1;
                                if item.count <= 0 { *slot = None; }
                            }
                        }
                        // Recalculate result
                        *result = lookup_crafting_recipe(grid);
                    }
                }
                // Recalculate crafting result if grid changed
                if matches!(target, SlotTarget::CraftGrid(_)) {
                    if let Menu::CraftingTable { ref grid, ref mut result } = menu {
                        *result = lookup_crafting_recipe(grid);
                    }
                }
            }
        }
        1 => { // QUICK_MOVE (shift-click)
            // Accept client's proposed changes
            for (changed_slot, changed_item) in changed_slots {
                if let Some(t) = map_slot(&menu, *changed_slot) {
                    set_container_slot(world_state, world, entity, &mut menu, &t, changed_item.clone());
                }
            }
            // Handle crafting result
            if slot >= 0 {
                if let Some(SlotTarget::CraftResult) = map_slot(&menu, slot) {
                    if let Menu::CraftingTable { ref mut grid, ref mut result } = menu {
                        for slot in grid.iter_mut() {
                            if let Some(ref mut item) = slot {
                                item.count -= 1;
                                if item.count <= 0 { *slot = None; }
                            }
                        }
                        *result = lookup_crafting_recipe(grid);
                    }
                }
            }
        }
        2 => { // SWAP (number key)
            for (changed_slot, changed_item) in changed_slots {
                if let Some(t) = map_slot(&menu, *changed_slot) {
                    set_container_slot(world_state, world, entity, &mut menu, &t, changed_item.clone());
                }
            }
        }
        4 => { // THROW (Q key in container)
            for (changed_slot, changed_item) in changed_slots {
                if let Some(t) = map_slot(&menu, *changed_slot) {
                    set_container_slot(world_state, world, entity, &mut menu, &t, changed_item.clone());
                }
            }
            // TODO: spawn dropped item entity
        }
        _ => {
            // Mode 3 (clone), 5 (drag), 6 (pickup_all) — resync
            // Send full container content to reset client state
        }
    }

    // Update state_id and reinsert component
    let new_state_id = client_state_id.wrapping_add(1);
    let _ = world.insert_one(entity, OpenContainer { container_id, menu, state_id: new_state_id });

    // Send confirmation — resync full container content
    // (Simpler than tracking individual slot changes; can optimize later)
    if let Ok(open) = world.get::<&OpenContainer>(entity) {
        let (slots, _) = build_container_slots(world_state, world, entity, &open.menu);
        if let Ok(sender) = world.get::<&ConnectionSender>(entity) {
            let _ = sender.0.send(InternalPacket::SetContainerContent {
                window_id: container_id,
                state_id: new_state_id,
                slots,
                carried_item: carried_item.clone(),
            });
        }
    }
}
```

**Step 5: Build and verify**

Run: `cargo build`
Run: `cargo test`

**Step 6: Commit**

```
feat(m11): add container click handler with slot mapping
```

---

### Task 5: Crafting Recipes

Add a hardcoded recipe table and the `lookup_crafting_recipe()` function.

**Files:**
- Modify: `crates/pickaxe-data/src/lib.rs` (add recipe functions, not codegen — handwritten)
- Modify: `crates/pickaxe-server/src/tick.rs` (lookup_crafting_recipe function)

**Step 1: Add recipe data to pickaxe-data**

In `crates/pickaxe-data/src/lib.rs`, add at the end (after the generated include):

```rust
/// A shaped crafting recipe. Pattern is 3x3, 0 means empty.
/// Item IDs use the same namespace as item_name_to_id().
pub struct CraftingRecipe {
    pub pattern: [i32; 9],  // item IDs, 0 = empty
    pub result_id: i32,
    pub result_count: i8,
    pub width: u8,          // pattern width (1-3)
    pub height: u8,         // pattern height (1-3)
}

/// Returns all crafting recipes.
pub fn crafting_recipes() -> &'static [CraftingRecipe] {
    use std::sync::LazyLock;
    static RECIPES: LazyLock<Vec<CraftingRecipe>> = LazyLock::new(|| build_recipes());
    &RECIPES
}

fn build_recipes() -> Vec<CraftingRecipe> {
    // Helper: look up item ID by name, panic if not found
    let id = |name: &str| -> i32 {
        item_name_to_id(name).unwrap_or_else(|| panic!("Unknown item: {}", name))
    };

    let mut recipes = Vec::new();

    // Planks from logs (4 planks per log) — shapeless 1x1
    for log in &["oak_log", "spruce_log", "birch_log", "jungle_log", "acacia_log", "dark_oak_log"] {
        recipes.push(CraftingRecipe {
            pattern: [id(log), 0,0, 0,0,0, 0,0,0],
            result_id: id("oak_planks"), result_count: 4, width: 1, height: 1,
        });
    }

    // Sticks (4 sticks from 2 planks) — 1x2
    recipes.push(CraftingRecipe {
        pattern: [id("oak_planks"), 0,0, id("oak_planks"), 0,0, 0,0,0],
        result_id: id("stick"), result_count: 4, width: 1, height: 2,
    });

    // Crafting table — 2x2
    let p = id("oak_planks");
    recipes.push(CraftingRecipe {
        pattern: [p, p, 0, p, p, 0, 0, 0, 0],
        result_id: id("crafting_table"), result_count: 1, width: 2, height: 2,
    });

    // Furnace — 3x3 ring of cobblestone
    let c = id("cobblestone");
    recipes.push(CraftingRecipe {
        pattern: [c, c, c, c, 0, c, c, c, c],
        result_id: id("furnace"), result_count: 1, width: 3, height: 3,
    });

    // Chest — 3x3 ring of planks
    recipes.push(CraftingRecipe {
        pattern: [p, p, p, p, 0, p, p, p, p],
        result_id: id("chest"), result_count: 1, width: 3, height: 3,
    });

    // Wooden pickaxe
    let s = id("stick");
    recipes.push(CraftingRecipe {
        pattern: [p, p, p, 0, s, 0, 0, s, 0],
        result_id: id("wooden_pickaxe"), result_count: 1, width: 3, height: 3,
    });

    // Wooden axe
    recipes.push(CraftingRecipe {
        pattern: [p, p, 0, p, s, 0, 0, s, 0],
        result_id: id("wooden_axe"), result_count: 1, width: 3, height: 2,
    });

    // Wooden shovel
    recipes.push(CraftingRecipe {
        pattern: [p, 0, 0, s, 0, 0, s, 0, 0],
        result_id: id("wooden_shovel"), result_count: 1, width: 1, height: 3,
    });

    // Wooden sword
    recipes.push(CraftingRecipe {
        pattern: [p, 0, 0, p, 0, 0, s, 0, 0],
        result_id: id("wooden_sword"), result_count: 1, width: 1, height: 3,
    });

    // Stone pickaxe
    recipes.push(CraftingRecipe {
        pattern: [c, c, c, 0, s, 0, 0, s, 0],
        result_id: id("stone_pickaxe"), result_count: 1, width: 3, height: 3,
    });

    // Stone axe
    recipes.push(CraftingRecipe {
        pattern: [c, c, 0, c, s, 0, 0, s, 0],
        result_id: id("stone_axe"), result_count: 1, width: 3, height: 2,
    });

    // Stone shovel
    recipes.push(CraftingRecipe {
        pattern: [c, 0, 0, s, 0, 0, s, 0, 0],
        result_id: id("stone_shovel"), result_count: 1, width: 1, height: 3,
    });

    // Stone sword
    recipes.push(CraftingRecipe {
        pattern: [c, 0, 0, c, 0, 0, s, 0, 0],
        result_id: id("stone_sword"), result_count: 1, width: 1, height: 3,
    });

    // Torches (4 from coal + stick) — 1x2
    recipes.push(CraftingRecipe {
        pattern: [id("coal"), 0,0, s, 0,0, 0,0,0],
        result_id: id("torch"), result_count: 4, width: 1, height: 2,
    });

    recipes
}
```

**Step 2: Implement lookup_crafting_recipe in tick.rs**

```rust
fn lookup_crafting_recipe(grid: &[Option<ItemStack>; 9]) -> Option<ItemStack> {
    // Convert grid to item IDs
    let grid_ids: [i32; 9] = std::array::from_fn(|i| {
        grid[i].as_ref().map(|item| item.item_id).unwrap_or(0)
    });

    // Find the bounding box of non-empty slots
    let mut min_x = 3usize; let mut max_x = 0usize;
    let mut min_y = 3usize; let mut max_y = 0usize;
    for y in 0..3 {
        for x in 0..3 {
            if grid_ids[y * 3 + x] != 0 {
                min_x = min_x.min(x); max_x = max_x.max(x);
                min_y = min_y.min(y); max_y = max_y.max(y);
            }
        }
    }
    if min_x > max_x { return None; } // Empty grid

    let w = max_x - min_x + 1;
    let h = max_y - min_y + 1;

    // Extract the compact pattern
    let mut compact = [0i32; 9];
    for y in 0..h {
        for x in 0..w {
            compact[y * 3 + x] = grid_ids[(min_y + y) * 3 + (min_x + x)];
        }
    }

    // Try matching against recipes (check both normal and mirrored)
    for recipe in pickaxe_data::crafting_recipes() {
        if recipe.width as usize != w || recipe.height as usize != h { continue; }

        // Normal match
        let mut matches = true;
        for y in 0..h {
            for x in 0..w {
                if compact[y * 3 + x] != recipe.pattern[y * 3 + x] {
                    matches = false; break;
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
                    matches = false; break;
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
```

**Step 3: Add a test for recipe lookup**

In tick.rs or as a separate test, verify a known recipe matches:

```rust
#[cfg(test)]
mod container_tests {
    use super::*;

    #[test]
    fn test_crafting_recipe_sticks() {
        let plank_id = pickaxe_data::item_name_to_id("oak_planks").unwrap();
        let stick_id = pickaxe_data::item_name_to_id("stick").unwrap();
        let mut grid: [Option<ItemStack>; 9] = std::array::from_fn(|_| None);
        grid[0] = Some(ItemStack::new(plank_id, 1));
        grid[3] = Some(ItemStack::new(plank_id, 1));
        let result = lookup_crafting_recipe(&grid);
        assert_eq!(result, Some(ItemStack::new(stick_id, 4)));
    }
}
```

**Step 4: Build and test**

Run: `cargo build`
Run: `cargo test`

**Step 5: Commit**

```
feat(m11): add crafting recipe table and lookup
```

---

### Task 6: Furnace Tick System

Add smelting logic that runs every server tick for active furnaces.

**Files:**
- Modify: `crates/pickaxe-data/src/lib.rs` (fuel values, smelting recipes)
- Modify: `crates/pickaxe-server/src/tick.rs` (furnace tick function)

**Step 1: Add fuel and smelting data to pickaxe-data**

In `crates/pickaxe-data/src/lib.rs`:

```rust
/// Returns the burn time in ticks for a fuel item, or None if not fuel.
pub fn fuel_burn_time(item_id: i32) -> Option<i16> {
    let name = item_id_to_name(item_id)?;
    Some(match name {
        "coal" => 1600,
        "charcoal" => 1600,
        "oak_log" | "spruce_log" | "birch_log" | "jungle_log" | "acacia_log" | "dark_oak_log" => 300,
        "oak_planks" | "spruce_planks" | "birch_planks" | "jungle_planks" | "acacia_planks" | "dark_oak_planks" => 300,
        "stick" => 100,
        "coal_block" => 16000,
        _ => return None,
    })
}

/// Returns the smelting result item ID for an input item, or None if not smeltable.
pub fn smelting_result(item_id: i32) -> Option<(i32, i16)> {
    let name = item_id_to_name(item_id)?;
    let (result_name, cook_time) = match name {
        "cobblestone" => ("stone", 200),
        "sand" => ("glass", 200),
        "iron_ore" | "raw_iron" => ("iron_ingot", 200),
        "gold_ore" | "raw_gold" => ("gold_ingot", 200),
        "oak_log" | "spruce_log" | "birch_log" | "jungle_log" | "acacia_log" | "dark_oak_log" => ("charcoal", 200),
        "clay_ball" => ("brick", 200),
        _ => return None,
    };
    Some((item_name_to_id(result_name)?, cook_time))
}
```

**Step 2: Implement tick_furnaces**

In `tick.rs`:

```rust
fn tick_furnaces(world: &World, world_state: &mut WorldState) {
    // Collect positions of furnaces that need updates sent to viewers
    let mut updates: Vec<(BlockPos, i16, i16, i16, i16)> = Vec::new();

    for (pos, block_entity) in world_state.block_entities.iter_mut() {
        let BlockEntity::Furnace {
            ref mut input, ref mut fuel, ref mut output,
            ref mut burn_time, ref mut burn_duration,
            ref mut cook_progress, ref mut cook_total,
        } = block_entity else { continue };

        let was_lit = *burn_time > 0;

        // Check if we can smelt
        let can_smelt = input.as_ref().and_then(|i| pickaxe_data::smelting_result(i.item_id)).is_some();
        let smelt_result = input.as_ref().and_then(|i| pickaxe_data::smelting_result(i.item_id));

        // Can the output accept the result?
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

            // Cook
            if can_smelt && output_accepts {
                if let Some((_, ct)) = smelt_result {
                    *cook_total = ct;
                }
                *cook_progress += 1;
                if *cook_progress >= *cook_total {
                    *cook_progress = 0;
                    // Transfer result
                    if let Some((result_id, _)) = smelt_result {
                        match output {
                            None => *output = Some(ItemStack::new(result_id, 1)),
                            Some(ref mut o) => o.count += 1,
                        }
                        // Consume input
                        if let Some(ref mut i) = input {
                            i.count -= 1;
                            if i.count <= 0 { *input = None; }
                        }
                    }
                }
            } else {
                // Nothing to smelt — reset progress
                *cook_progress = 0;
            }
        } else {
            *cook_progress = 0;
        }

        // Update lit_furnace / furnace block state if lit status changed
        let is_lit = *burn_time > 0;
        if was_lit != is_lit {
            let new_state = if is_lit {
                pickaxe_data::block_name_to_default_state("lit_furnace")
            } else {
                pickaxe_data::block_name_to_default_state("furnace")
            };
            // TODO: update block state in world — need facing preservation
        }

        updates.push((*pos, *burn_time, *burn_duration, *cook_progress, *cook_total));
    }

    // Send updates to players who have this furnace open
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
```

**Step 3: Add tick_furnaces to the main tick loop**

In `run_tick_loop()`, add after the existing tick systems:

```rust
tick_furnaces(&world, &mut world_state);
```

**Step 4: Build and verify**

Run: `cargo build`
Run: `cargo test`

**Step 5: Commit**

```
feat(m11): add furnace tick system with smelting and fuel consumption
```

---

### Task 7: Block Entity Persistence

Save and load block entities in chunk NBT (Anvil format).

**Files:**
- Modify: `crates/pickaxe-world/src/chunk.rs` (not ideal — block entities are in WorldState, not Chunk)
- Modify: `crates/pickaxe-server/src/tick.rs` (save/load block entities alongside chunks)

**Step 1: Serialize block entities to NBT**

In `tick.rs`, add:

```rust
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
                        "Count" => NbtValue::Byte(item.count),
                    });
                }
            }
            nbt_compound! {
                "id" => NbtValue::String("minecraft:chest".into()),
                "x" => NbtValue::Int(pos.x),
                "y" => NbtValue::Int(pos.y),
                "z" => NbtValue::Int(pos.z),
                "Items" => NbtValue::List(items),
            }
        }
        BlockEntity::Furnace { input, fuel, output, burn_time, burn_duration, cook_progress, cook_total } => {
            let mut items = Vec::new();
            for (i, slot) in [input, fuel, output].iter().enumerate() {
                if let Some(item) = slot {
                    let name = pickaxe_data::item_id_to_name(item.item_id).unwrap_or("air");
                    items.push(nbt_compound! {
                        "Slot" => NbtValue::Byte(i as i8),
                        "id" => NbtValue::String(format!("minecraft:{}", name)),
                        "Count" => NbtValue::Byte(item.count),
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
                "CookTimeTotal" => NbtValue::Short(*cook_total),
            }
        }
    }
}
```

**Step 2: Save block entities alongside chunk saves**

Modify `queue_chunk_save()` to include block entities in the chunk NBT. Add a `block_entities` list to the chunk's NBT compound. The block entities for a chunk are those whose position falls within the chunk's XZ bounds.

Alternatively, add a separate save path for block entities. The simpler approach: when saving a chunk, collect all block entities in that chunk's coordinate range and append them to the NBT.

**Step 3: Load block entities from chunk NBT**

When loading a chunk in `ensure_chunk()`, after parsing the chunk from NBT, also parse the `block_entities` list and insert them into `world_state.block_entities`.

**Step 4: Build and verify**

Run: `cargo build`
Run: `cargo test`

**Step 5: Commit**

```
feat(m11): persist block entities in chunk NBT saves
```

---

### Task 8: Lua API and Integration Testing

Add Lua bridge functions for block entities and test the full flow.

**Files:**
- Modify: `crates/pickaxe-server/src/bridge.rs`
- Modify: `lua/vanilla/init.lua`

**Step 1: Add block entity Lua API**

In `bridge.rs`, extend `register_world_api` to add:

```rust
// pickaxe.world.get_block_entity(x, y, z)
world_table.set("get_block_entity", lua.create_function(|lua, (x, y, z): (i32, i32, i32)| {
    with_world_state(lua, |ws| {
        let pos = BlockPos::new(x, y, z);
        match ws.get_block_entity(&pos) {
            Some(BlockEntity::Chest { inventory }) => {
                let table = lua.create_table()?;
                table.set("type", "chest")?;
                let items = lua.create_table()?;
                for (i, slot) in inventory.iter().enumerate() {
                    if let Some(item) = slot {
                        let item_table = lua.create_table()?;
                        item_table.set("id", item.item_id)?;
                        item_table.set("name", pickaxe_data::item_id_to_name(item.item_id).unwrap_or("unknown"))?;
                        item_table.set("count", item.count)?;
                        items.set(i + 1, item_table)?;
                    }
                }
                table.set("items", items)?;
                Ok(Some(table))
            }
            Some(BlockEntity::Furnace { input, fuel, output, burn_time, cook_progress, .. }) => {
                let table = lua.create_table()?;
                table.set("type", "furnace")?;
                table.set("burn_time", *burn_time)?;
                table.set("cook_progress", *cook_progress)?;
                // ... add input/fuel/output items
                Ok(Some(table))
            }
            None => Ok(None),
        }
    })
})?)?;
```

**Step 2: Add container events to vanilla Lua mod**

In `lua/vanilla/init.lua`:

```lua
pickaxe.events.on("container_open", function(event)
    pickaxe.log(event.name .. " opened " .. event.block_type .. " at " .. event.x .. "," .. event.y .. "," .. event.z)
end, { priority = "MONITOR", mod_id = "pickaxe-vanilla" })

pickaxe.events.on("container_close", function(event)
    pickaxe.log(event.name .. " closed " .. event.block_type)
end, { priority = "MONITOR", mod_id = "pickaxe-vanilla" })
```

**Step 3: Full integration test**

Run: `cargo build && cargo test`

Manual test with MC 1.21.1 client:
1. `/give crafting_table 1` → place it → right-click → crafting GUI opens
2. Place planks in 2x2 → result shows crafting table
3. Take result → grid items consumed
4. Close crafting table → leftover items drop
5. `/give chest 1` → place it → right-click → chest GUI opens
6. Put items in chest → close → reopen → items persist
7. Break chest → items drop
8. `/give furnace 1` → place it → right-click → furnace GUI opens
9. Put cobblestone in input, coal in fuel → progress bar advances → stone appears in output
10. Close and reopen furnace → smelting continues
11. Restart server → chest and furnace contents persist

**Step 4: Commit**

```
feat(m11): add block entity Lua API and container events
```

---

## Tick Order After M11

1. `tick_keep_alive`
2. `tick_item_physics`
3. `tick_item_pickup` (every 4 ticks)
4. **`tick_furnaces`** (NEW)
5. `tick_entity_tracking`
6. `tick_entity_movement_broadcast`
7. `tick_world_time`
8. `tick_block_breaking`
9. `tick_hunger` (every tick)

## Key Reusable Code

- `write_slot()` / `read_slot()` in `codec.rs` — slot encoding for container packets
- `give_item_to_player()` in `tick.rs` — adding items to player inventory
- `spawn_item_entity()` in `tick.rs` — dropping items on block break / container close
- `broadcast_to_all()` / `broadcast_except()` in `tick.rs` — packet broadcasting
- `Inventory.set_slot()` / `find_slot_for_item()` in `ecs.rs` — inventory management

## Verification

1. `cargo build` — compiles
2. `cargo test` — all tests pass (including new recipe test)
3. Manual test with MC 1.21.1 client:
   - Craft sticks from planks on crafting table
   - Store items in chest, persist across server restart
   - Smelt cobblestone to stone in furnace
   - Shift-click moves items between container and inventory
   - Breaking containers drops their contents
