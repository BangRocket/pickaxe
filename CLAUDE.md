# Pickaxe — Claude Code Project Notes

## What is this?
A Minecraft Java Edition compatible server written in Rust with Lua scripting. All game behavior lives in Lua mods; Rust handles networking, protocol, chunks, and the Lua runtime.

## Build & Run
```bash
cargo build          # build all crates
cargo test           # run all 15 unit tests
cargo run            # start server on 0.0.0.0:25565
```

## Crate Structure
```
crates/
  pickaxe-types/           # BlockPos, ChunkPos, Vec3d, GameProfile, TextComponent
  pickaxe-nbt/             # NBT serialization
  pickaxe-protocol-core/   # VarInt codec, Connection (encryption/compression), InternalPacket, ProtocolAdapter trait
  pickaxe-protocol-v1_21/  # MC 1.21.x adapter (protocol 767) — encode/decode + registry data
  pickaxe-world/           # Chunk sections, palette encoding, flat world generator
  pickaxe-events/          # EventBus with priority ordering
  pickaxe-scripting/       # mlua/LuaJIT runtime, mod loader, sandbox
  pickaxe-data/            # Stub for future block/item codegen from MC data JSONs
  pickaxe-server/          # Main binary — TCP listener, connection state machine, play loop
```

## Protocol Version
Currently targets MC 1.21/1.21.1 (protocol version 767). Do NOT use protocol 768+ features (e.g., sea_level in JoinGame).

## Key Conventions
- Block state IDs must match MC 1.21.1 exactly (source: PrismarineJS minecraft-data `data/pc/1.21.1/blocks.json`)
- Registry data must include ALL vanilla entries (client crashes on missing damage types, etc.)
- Packet IDs are version-specific — always verify against PrismarineJS or Pumpkin MC source
- Lua VM is NOT Send — must stay on main thread, use mpsc channels from async tasks
- mlua errors need `.map_err(|e| anyhow!("{}", e))` — not Send+Sync in LuaJIT mode

## Testing with MC Client
1. `cargo run` to start server
2. Add `localhost` in MC 1.21.1 server list
3. Join with offline mode (server config: `online_mode = false`)
4. Debug errors in `~/.local/share/PrismLauncher/instances/1.21.1/minecraft/debug/`
