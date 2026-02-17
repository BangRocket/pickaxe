# M10: World Persistence & Saves — Design

## Summary

Add persistent world storage so chunks, player data, and world metadata survive server restarts. Uses vanilla-compatible formats: Anvil `.mca` region files for chunks, gzip-compressed NBT `.dat` files for players, and `level.dat` for world metadata.

## Architecture

Three persistence targets:

| What | Format | When Saved | Location |
|------|--------|------------|----------|
| Chunks | Anvil `.mca` region files (DEFLATE NBT) | Write-through on block change + shutdown | `world/region/r.X.Z.mca` |
| Player data | Vanilla `.dat` (gzip NBT) | On disconnect + every 60s + shutdown | `world/playerdata/{UUID}.dat` |
| Level metadata | `level.dat` (gzip NBT) | On startup (create if missing) + shutdown | `world/level.dat` |

New crate `pickaxe-region` handles Anvil I/O. Write-through saves use a background Tokio task via mpsc channel to avoid blocking the tick loop.

## Component 1: Region File I/O (`pickaxe-region`)

### Core Types

```
RegionFile
  file: File
  locations: [u32; 1024]        // sector offsets (header bytes 0-4095)
  timestamps: [u32; 1024]       // last-modified (header bytes 4096-8191)
  used_sectors: BitSet           // sector allocation bitmap

RegionStorage
  dir: PathBuf                   // world/region/
  cache: HashMap<(i32, i32), RegionFile>
```

### Anvil Format

- Region file = `r.{regionX}.{regionZ}.mca`
- 8KB header: 4KB location table + 4KB timestamp table (1024 entries each, big-endian)
- Location entry: `(sectorNumber << 8) | numSectors`
- Chunk record: 4-byte length + 1-byte compression type + compressed NBT
- Compression type 2 = DEFLATE (MC default)
- Sector size: 4096 bytes, sectors 0-1 reserved for header
- Coordinate mapping: `region = chunk >> 5`, `local = chunk & 31`, `index = localX + localZ * 32`

### Operations

- `RegionStorage::read_chunk(cx, cz) -> Option<NbtValue>`
- `RegionStorage::write_chunk(cx, cz, nbt: &NbtValue)`
- `RegionFile::new(path)` — create or open, parse header
- `RegionFile::close()` — flush and close

## Component 2: Chunk NBT Serialization

### Chunk-to-NBT (write)

```
{
  xPos: INT, zPos: INT, yPos: INT (-4),
  DataVersion: INT (3955),
  Status: STRING ("full"),
  LastUpdate: LONG (world_age),
  sections: LIST[
    {
      Y: BYTE,
      block_states: { palette: LIST[{Name: STRING, Properties: COMPOUND}], data: LONG_ARRAY },
      SkyLight: BYTE_ARRAY (2048 bytes)
    }
  ],
  Heightmaps: { MOTION_BLOCKING: LONG_ARRAY }
}
```

Omitted (not needed yet): biomes (single-biome flat world), block entities, structures, carving masks, blending data.

### NBT-to-Chunk (read)

Parse `sections` list, reconstruct palette and block data per section. Missing sections default to air. Must handle both single-entry palettes (no `data` array) and multi-entry palettes.

## Component 3: WorldState Integration

### Changes

```rust
WorldState {
    chunks: HashMap<ChunkPos, Chunk>,        // in-memory cache
    region_storage: RegionStorage,            // disk-backed storage
    save_tx: mpsc::UnboundedSender<SaveOp>,  // async saver channel
}

enum SaveOp {
    Chunk(ChunkPos, Vec<u8>),  // pre-serialized compressed chunk data
    Player(Uuid, Vec<u8>),     // pre-serialized compressed player data
    LevelDat(Vec<u8>),         // pre-serialized level.dat
    Shutdown,                  // flush + stop
}
```

### Load Path

1. Check HashMap cache → hit? return
2. `region_storage.read_chunk(cx, cz)` → found? deserialize to `Chunk`, cache, return
3. Not on disk → generate flat chunk, cache, queue save

### Save Path (write-through)

- `set_block()` serializes chunk to NBT bytes on main thread, sends `SaveOp::Chunk` to saver
- Background `tokio::spawn` task reads from channel, writes to region files
- Serialization (fast, CPU) happens on main thread; I/O (slow) on background task

### Shutdown

Send `SaveOp::Shutdown`. Saver task processes remaining queue, flushes, closes files.

## Component 4: Player Data Persistence

### NBT Structure (minimum viable subset)

```
{
  DataVersion: INT (3955),
  Pos: LIST[DOUBLE x3],
  Rotation: LIST[FLOAT x2],
  OnGround: BYTE,
  Health: FLOAT,
  FallDistance: FLOAT,
  foodLevel: INT,
  foodSaturationLevel: FLOAT,
  foodExhaustionLevel: FLOAT,
  Inventory: LIST[{ Slot: BYTE, id: STRING, count: BYTE }],
  SelectedItemSlot: INT,
  playerGameType: INT,
  Dimension: STRING,
  abilities: {
    invulnerable: BYTE, flying: BYTE, mayfly: BYTE,
    instabuild: BYTE, mayBuild: BYTE,
    flySpeed: FLOAT (0.05), walkSpeed: FLOAT (0.1)
  }
}
```

### Timing

- Save on player disconnect (immediate, on main thread)
- Save every 60 seconds for all online players (via saver task)
- Save all on shutdown

### Load

On player join: if `world/playerdata/{UUID}.dat` exists, restore state. Otherwise use defaults (spawn point, 20 HP, full food, empty inventory).

### File Operations

- Path: `world/playerdata/{UUID}.dat`
- Write to temp file first, then rename (atomic)
- Gzip compressed NBT

## Component 5: Level.dat

### NBT Structure

```
{
  DataVersion: INT (3955),
  Data: {
    LevelName: STRING,
    SpawnX: INT, SpawnY: INT, SpawnZ: INT,
    Time: LONG (world_age),
    DayTime: LONG (time_of_day),
    GameType: INT,
    Difficulty: BYTE,
    hardcore: BYTE (0),
    allowCommands: BYTE (1),
    Version: { Name: STRING ("1.21.1"), Id: INT (767) }
  }
}
```

### Timing

- On first startup: create with defaults
- On startup: load and apply (spawn point, world time, etc.)
- On shutdown: save current state

## Decisions

- **DEFLATE compression** for region files (MC default, type 2)
- **Gzip compression** for player data and level.dat (vanilla standard)
- **No external chunk files** (.mcc) — chunks will stay under the 256-sector limit
- **No chunk unloading from cache** in M10 — all accessed chunks stay in memory. Eviction can be added later.
- **DataVersion 3955** — MC 1.21.1's data version number
- **Flat world only** — no world gen beyond existing flat generator
