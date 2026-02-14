use crate::chunk::{Chunk, ChunkSection};

/// Block state IDs for the flat world.
/// These need to match the MC 1.21 block state registry.
/// In MC 1.21: air=0, stone=1, granite=2, ... bedrock=33, ... grass_block=8/9, dirt=10
/// The exact IDs come from the block state palette.
/// For MC 1.21.1 (data version 3955):
///   bedrock = state ID 33
///   stone = state ID 1
///   dirt = state ID 10
///   grass_block[snowy=false] = state ID 9
///   air = state ID 0
pub const AIR: i32 = 0;
pub const STONE: i32 = 1;
pub const GRASS_BLOCK: i32 = 9; // grass_block[snowy=false]
pub const DIRT: i32 = 10;
pub const BEDROCK: i32 = 33;

/// Generates a flat world chunk (superflat default: bedrock, 2 dirt, grass_block).
/// Layer layout (matching vanilla superflat "Classic Flat"):
///   y = -64: bedrock
///   y = -63: dirt
///   y = -62: dirt
///   y = -61: grass_block
///   y = -60 and above: air
pub fn generate_flat_chunk() -> Chunk {
    let mut chunk = Chunk::new();

    // Section 0: y = -64 to -49
    // y=-64 (local 0): bedrock, y=-63 (local 1): dirt, y=-62 (local 2): dirt, y=-61 (local 3): grass_block
    // y=-60 to -49 (local 4-15): air
    let mut blocks = [AIR; 4096];
    for x in 0..16 {
        for z in 0..16 {
            let idx = |y: usize| y * 256 + z * 16 + x;
            blocks[idx(0)] = BEDROCK;     // y = -64
            blocks[idx(1)] = DIRT;        // y = -63
            blocks[idx(2)] = DIRT;        // y = -62
            blocks[idx(3)] = GRASS_BLOCK; // y = -61
        }
    }
    chunk.sections[0] = ChunkSection::from_blocks(&blocks);

    // All other sections remain empty (air)
    chunk
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flat_chunk_generation() {
        let chunk = generate_flat_chunk();
        assert_eq!(chunk.sections.len(), 24);
        // Section 0 should have blocks
        assert!(chunk.sections[0].block_count > 0);
        // Section 1+ should be empty
        assert_eq!(chunk.sections[1].block_count, 0);
    }

    #[test]
    fn test_flat_chunk_serializes() {
        let chunk = generate_flat_chunk();
        let data = chunk.serialize_sections();
        assert!(!data.is_empty());
    }

    #[test]
    fn test_flat_chunk_heightmap() {
        let chunk = generate_flat_chunk();
        let heightmap = chunk.compute_heightmap();
        // All columns should have height at y=-61 (grass_block)
        // Heightmap value = world_y - MIN_Y + 1 = -61 - (-64) + 1 = 4
        // Check first entry
        let first_value = heightmap[0] & 0x1FF; // 9-bit mask
        assert_eq!(first_value, 4, "Expected heightmap value 4 for grass at y=-61");
    }
}
