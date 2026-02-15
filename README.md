# Pickaxe

A Minecraft Java Edition compatible server written in Rust with Lua scripting.

All game behavior lives in Lua mods — Rust handles networking, protocol, chunk storage, and the Lua runtime. Vanilla Minecraft itself is a Lua mod (`lua/vanilla/`), loaded through the same system as third-party mods.

## Status

Early development. Currently supports:

- MC 1.21/1.21.1 clients (protocol 767)
- Offline-mode authentication
- Flat world generation with proper sky lighting
- Player movement and chunk streaming
- Server-side block breaking and placing (creative mode)
- Lua scripting with event system (`server_start`, `player_join`, `player_move`, `block_break`, `block_place`)

## Building

Requires Rust (stable) and LuaJIT.

```bash
cargo build
cargo test
cargo run        # starts server on 0.0.0.0:25565
```

## Connecting

1. Start the server with `cargo run`
2. Add `localhost` to your MC 1.21.1 server list
3. Join in offline mode

## Project Structure

```
crates/
  pickaxe-types/             # BlockPos, ChunkPos, Vec3d, GameProfile, TextComponent
  pickaxe-nbt/               # NBT serialization
  pickaxe-protocol-core/     # VarInt codec, Connection, InternalPacket, ProtocolAdapter trait
  pickaxe-protocol-v1_21/    # MC 1.21.x protocol adapter (encode/decode + registry data)
  pickaxe-world/             # Chunk sections, palette encoding, flat world generator
  pickaxe-events/            # EventBus with priority ordering
  pickaxe-scripting/         # mlua/LuaJIT runtime, mod loader, sandbox
  pickaxe-data/              # Stub for block/item codegen from MC data
  pickaxe-server/            # Main binary — TCP listener, state machine, play loop
lua/
  core/                      # Core Lua API (event registration, logging)
  vanilla/                   # Vanilla game behavior mod
```

## License

All rights reserved. See [LICENSE](LICENSE).
