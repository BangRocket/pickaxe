# Pickaxe: Minecraft Java Edition Server in Rust + Lua

## Context

Build a Minecraft Java Edition compatible server from scratch. The goals are:
- **Performance**: Rust engine, competitive with Pumpkin (~100MB RAM, fast startup)
- **Scripting-first**: All game behavior (vanilla included) lives in Lua scripts, not compiled Rust
- **Moddable**: Event bus + override system so multiple mods coexist without manual merging
- **Version-agnostic**: Protocol adapters make MC version support pluggable

The first milestone is: a player can connect with a real MC client, authenticate, join, see a flat world, and walk around.

---

## Architecture

### Core Principle
Rust handles: networking, protocol parsing, chunk storage/serialization, the Lua runtime, ECS, and the event system.
Lua handles: all game rules, block behavior, entity AI, crafting — everything that makes Minecraft *Minecraft*.
Vanilla Minecraft is itself a Lua mod (`pickaxe-vanilla`), loaded through the same mod system as third-party mods.

### Crate Structure (Cargo workspace)

```
pickaxe/
  crates/
    pickaxe-types/           # BlockPos, ChunkPos, Vec3d, Uuid, Identifier, TextComponent
    pickaxe-nbt/             # NBT read/write with serde support
    pickaxe-data/            # Build-time codegen from MC data JSONs → block/item/entity enums
    pickaxe-protocol-core/   # ProtocolAdapter trait, InternalPacket enum, Connection, VarInt codec
    pickaxe-protocol-v1_21/  # Wire format adapter for MC 1.21.x
    pickaxe-world/           # Chunk storage, palette encoding, FlatWorldGenerator
    pickaxe-events/          # EventBus with priorities + OverrideRegistry
    pickaxe-scripting/       # mlua/LuaJIT runtime, Lua-Rust bridge (pickaxe.* API), mod loader
    pickaxe-server/          # Main binary: TCP listener, tick loop, hecs ECS world
  lua/
    core/                    # pickaxe-core Lua module (bridge utilities)
    vanilla/                 # pickaxe-vanilla mod (vanilla game behavior)
  data/
    minecraft/               # Extracted MC data JSONs (blocks.json, items.json, etc.)
  config/
    server.toml              # Server configuration
```

### Key Architectural Decisions

- **hecs for ECS** (not bevy_ecs): Pure data structure, no scheduler. Gives full control over Lua/ECS interleaving in the tick loop.
- **Single LuaJIT VM**: Each mod gets an isolated environment table (sandbox) within one VM. Proven pattern from Minetest, Factorio, Roblox.
- **InternalPacket enum**: Closed set of version-independent packet representations. Protocol adapters map wire bytes ↔ this enum.
- **Offline mode first**: Skip Mojang auth during development (togglable). Online mode added but optional.

### Tick Loop (20 TPS)

1. **Network receive** (Tokio async) → decode packets, queue as events
2. **Event dispatch** (Lua thread) → Lua handlers run, read/write ECS via bridge
3. **Systems** (Rust) → physics, chunk streaming, entity updates
4. **Network send** (Tokio async) → diff ECS state, build outbound packets, send

### Mod System

**Events (primary API):**
- Priorities: LOWEST → LOW → NORMAL → HIGH → HIGHEST → MONITOR
- Vanilla mod registers at NORMAL; other mods hook before/after
- Events can be cancelled (if cancellable); MONITOR is read-only
- Lua API: `pickaxe.events.on("player_move", callback, { priority = "HIGH" })`

**Overrides (secondary API):**
- Replace a named function entirely: `pickaxe.override("block.calculate_drops", fn)`
- `pickaxe.call_original()` chains to the replaced function
- Last-writer-wins with a warning log

**Mod manifest** (`pickaxe.toml`):
```toml
[mod]
id = "my-mod"
name = "My Mod"
version = "1.0.0"
[mod.entrypoint]
main = "init.lua"
[mod.dependencies]
vanilla = ">=0.1.0"
[mod.load_order]
after = ["vanilla"]
```

### Protocol Abstraction

```rust
trait ProtocolAdapter: Send + Sync {
    fn protocol_version(&self) -> i32;
    fn decode_packet(&self, state: ConnectionState, id: i32, data: &mut BytesMut) -> Result<InternalPacket>;
    fn encode_packet(&self, state: ConnectionState, packet: &InternalPacket) -> Result<BytesMut>;
    fn registry_data(&self) -> Vec<InternalPacket>;
}
```

Version selection happens at handshake time. Adding MC 1.20.4 = write `pickaxe-protocol-v1_20_4` crate, register it. No server logic changes.

### Key External Crates

| Crate | Purpose |
|-------|---------|
| `mlua` (features: luajit, vendored, serialize) | Lua bindings |
| `hecs` | ECS |
| `tokio` (features: full) | Async runtime |
| `bytes` | BytesMut for zero-copy packet building |
| `rsa` | RSA for login encryption |
| `aes` + `cfb8` | AES-CFB8 stream cipher |
| `flate2` | Zlib compression |
| `tracing` + `tracing-subscriber` | Structured logging |
| `reqwest` | Mojang auth HTTP calls |
| `serde` + `toml` | Config/manifest parsing |
| `uuid` | UUID handling |
| `sha1` | Server ID hash for auth |
| `num-bigint` | MC's negative hex hash format |

---

## Implementation Plan — Milestone 1

### Phase 1: Project Skeleton

**Files to create:**
- `Cargo.toml` — workspace definition with all member crates
- `crates/pickaxe-server/Cargo.toml` + `src/main.rs` — Tokio runtime, TCP listener on port 25565
- `crates/pickaxe-server/src/config.rs` — TOML config loading
- `config/server.toml` — default config (port, max_players, motd, online_mode)
- Stub `Cargo.toml` + `src/lib.rs` for all other crates

**Verification:** `cargo run` starts, logs "Listening on 0.0.0.0:25565", accepts TCP connections.

### Phase 2: Foundation Types + Protocol Core

**pickaxe-types** (`crates/pickaxe-types/src/lib.rs`):
- `BlockPos`, `ChunkPos`, `Vec3d`, `Uuid`, `Identifier`, `GameProfile`, `TextComponent`
- `GameMode` enum, `Hand` enum

**pickaxe-nbt** (`crates/pickaxe-nbt/src/lib.rs`):
- NBT writer (compound, list, string, int, byte, etc.)
- Serde serializer (or wrap an existing crate like `valence_nbt`)

**pickaxe-protocol-core**:
- `src/codec.rs` — VarInt/VarLong read/write, string read/write, position encoding
- `src/state.rs` — `ConnectionState` enum (Handshaking, Status, Login, Configuration, Play)
- `src/packets.rs` — `InternalPacket` enum (~20 variants for milestone 1)
- `src/adapter.rs` — `ProtocolAdapter` trait
- `src/connection.rs` — `Connection` struct: TCP framing, compression, encryption layers

**Verification:** Unit tests for VarInt encoding/decoding, packet framing.

### Phase 3: Protocol Adapter (1.21.x) + Status/Login

**pickaxe-protocol-v1_21**:
- `src/handshake.rs` — Handshake packet decode
- `src/status.rs` — Status Request/Response, Ping/Pong (server list)
- `src/login.rs` — Login Start, Encryption Request/Response, Login Success, Set Compression
- `src/configuration.rs` — Registry Data, Known Packs, Finish Configuration
- `src/play.rs` — Join Game, Synchronize Player Position, Chunk Data, Keep Alive, Set Center Chunk, Game Event, Player Position/Rotation packets
- `assets/registries/` — NBT registry data for 1.21 (dimension types, biomes, damage types)

**pickaxe-server**:
- `src/network.rs` — Connection accept loop, spawn per-connection Tokio task
- `src/player.rs` — Player connection state machine: Handshake → Login → Configuration → Play
- RSA key generation at startup, AES-CFB8 after encryption handshake
- Optional Mojang session authentication (toggled by `online_mode` config)
- Zlib compression after Set Compression

**Verification:** Server appears in MC server list with MOTD. Client completes login through Configuration state.

### Phase 4: World + Chunk Serialization

**pickaxe-data**:
- `build.rs` — read `data/minecraft/blocks.json`, generate `BlockKind` enum with state IDs
- `src/lib.rs` — re-export generated types
- `data/minecraft/blocks.json` — extract from vanilla jar (or copy from Pumpkin's data)

**pickaxe-world**:
- `src/chunk.rs` — `Chunk` (24 sections, -64 to 320), `ChunkSection` (16x16x16), palette-based block storage
- `src/generator.rs` — `FlatWorldGenerator`: bedrock → stone → dirt → grass_block
- `src/heightmap.rs` — MOTION_BLOCKING heightmap (long array encoding)
- Chunk → protocol serialization (palette + packed long array + heightmap NBT)

**Verification:** Generate a flat chunk, serialize it, verify byte layout against protocol spec.

### Phase 5: Play State — Join the World

**pickaxe-server**:
- After Configuration, transition to Play state
- ECS setup: spawn Player entity with Position, Rotation, Player, PlayerConnection, ChunkView components
- Send: Join Game → Synchronize Player Position → Set Center Chunk → Chunk Data (radius 8) → Game Event (start waiting for chunks)
- Keep-alive loop: send every 15s, kick on 30s timeout

**Verification:** Client exits "Loading Terrain" screen, sees flat world, can look around.

### Phase 6: Movement

**pickaxe-server**:
- Handle Player Position, Player Position And Rotation, Player Rotation packets (serverbound)
- Update Position/Rotation/OnGround components in ECS
- Chunk view tracking: when player crosses chunk boundary, calculate new visible set, send new chunks, unload old ones
- Send Set Center Chunk on chunk boundary crossing

**Verification:** Player walks around flat world, new chunks load seamlessly as they move.

### Phase 7: Lua Scripting Foundation

**pickaxe-events** (`crates/pickaxe-events/src/lib.rs`):
- `EventBus` — HashMap<event_name, Vec<Listener>> sorted by priority
- `Priority` enum, `EventResult` (Continue/Cancel)
- `OverrideRegistry` — HashMap<function_name, OverrideEntry>

**pickaxe-scripting**:
- `src/runtime.rs` — Initialize mlua with LuaJIT, set up global `pickaxe` table
- `src/bridge.rs` — Expose `pickaxe.events`, `pickaxe.log`, `pickaxe.world`, `pickaxe.players` to Lua
- `src/mod_loader.rs` — Discover `pickaxe.toml` manifests, topological sort, load in order
- `src/sandbox.rs` — Per-mod environment table creation, scoped `require`

**lua/core/**:
- `pickaxe.toml` + `init.lua` — Base utilities

**lua/vanilla/**:
- `pickaxe.toml` + `init.lua` — Register event listeners for ServerStart, PlayerJoin, PlayerMove
- Log messages proving events fire correctly

**Integration in pickaxe-server:**
- Initialize Lua runtime after config load, before accepting connections
- Fire `ServerStartEvent` after startup
- Fire `PlayerJoinEvent` when player enters Play state
- Fire `PlayerMoveEvent` when position packets arrive

**Verification:** Start server, connect with MC client. Console shows Lua log messages for ServerStart, PlayerJoin, and PlayerMove events.

---

## End-to-End Verification

1. `cargo build` compiles all crates without errors
2. `cargo test` passes unit tests (VarInt, NBT, chunk serialization, event bus)
3. `cargo run` starts server on port 25565
4. Open Minecraft 1.21.x, add server `localhost`
5. Server appears in server list with MOTD and player count
6. Click Join → client completes login → sees flat world
7. Walk around → chunks load/unload correctly
8. Server console shows Lua event log messages (join, move)
9. Memory usage stays under ~150MB with one player
