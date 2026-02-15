# Pickaxe TODO

## Current Blocker
- [ ] **Fix chunk rendering (black ground/void)** — Chunks are sent and accepted by the client but render as void. Likely a chunk section serialization issue (palette encoding, heightmap format, or missing light data). Needs byte-level comparison against vanilla server output.

## Milestone 1 Remaining
- [ ] Fix chunk rendering so flat world is visible
- [ ] Send player entity metadata (arms not visible)
- [ ] Verify movement/chunk loading works once rendering is fixed
- [ ] Test with walking around the flat world

## Future
- [ ] Online mode (Mojang auth)
- [ ] hecs ECS integration (currently using PlayerHandle directly)
- [ ] Multi-player support (entity spawning, player list)
- [ ] Block breaking/placing
- [ ] Chat
