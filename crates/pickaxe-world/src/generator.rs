use crate::chunk::{Chunk, ChunkSection};

/// Block state IDs for the flat world.
/// For MC 1.21.1 (data version 3955):
pub const AIR: i32 = 0;
pub const STONE: i32 = 1;
pub const GRASS_BLOCK: i32 = 9; // grass_block[snowy=false]
pub const DIRT: i32 = 10;
pub const BEDROCK: i32 = 79;

// Ore block state IDs (MC 1.21.1, from PrismarineJS blocks.json via codegen)
pub const COAL_ORE: i32 = 127;
pub const IRON_ORE: i32 = 125;
pub const COPPER_ORE: i32 = 22942;
pub const GOLD_ORE: i32 = 123;
pub const LAPIS_ORE: i32 = 520;
pub const REDSTONE_ORE: i32 = 5735; // lit=false
pub const DIAMOND_ORE: i32 = 4274;
pub const EMERALD_ORE: i32 = 7511;
pub const GRAVEL: i32 = 118;

/// Surface Y level (grass_block). Players spawn 1 block above this.
pub const SURFACE_Y: i32 = -51;

/// Generates a flat world chunk with ores.
/// Layer layout:
///   y = -64: bedrock
///   y = -63 to -54: stone (10 layers, with ore veins)
///   y = -53: dirt
///   y = -52: dirt
///   y = -51: grass_block
///   y = -50 and above: air
///
/// If chunk_x/chunk_z are provided, ores are seeded deterministically.
pub fn generate_flat_chunk() -> Chunk {
    generate_flat_chunk_at(0, 0)
}

pub fn generate_flat_chunk_at(chunk_x: i32, chunk_z: i32) -> Chunk {
    let mut chunk = Chunk::new();

    // Section 0: y = -64 to -49
    let mut blocks = [AIR; 4096];
    for x in 0..16 {
        for z in 0..16 {
            let idx = |y: usize| y * 256 + z * 16 + x;
            blocks[idx(0)] = BEDROCK;      // y = -64
            // y = -63 to -54 (local 1-10): stone
            for ly in 1..=10 {
                blocks[idx(ly)] = STONE;
            }
            blocks[idx(11)] = DIRT;        // y = -53
            blocks[idx(12)] = DIRT;        // y = -52
            blocks[idx(13)] = GRASS_BLOCK; // y = -51
        }
    }

    // Place ore veins using a simple deterministic hash
    place_ores(&mut blocks, chunk_x, chunk_z);

    chunk.sections[0] = ChunkSection::from_blocks(&blocks);

    chunk
}

/// Simple hash function for deterministic ore placement.
fn ore_hash(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = seed;
    h = h.wrapping_mul(31).wrapping_add(x as u32);
    h = h.wrapping_mul(31).wrapping_add(y as u32);
    h = h.wrapping_mul(31).wrapping_add(z as u32);
    h ^= h >> 16;
    h = h.wrapping_mul(0x45d9f3b);
    h ^= h >> 16;
    h
}

/// Ore distribution configuration: (block_id, min_local_y, max_local_y, chance_per_1000, vein_size)
/// Local Y 1 = world Y -63, local Y 10 = world Y -54
const ORE_CONFIG: &[(i32, usize, usize, u32, u32)] = &[
    // Coal: common throughout, veins of 3-5
    (COAL_ORE,     1, 10, 80, 4),
    // Iron: common in lower layers, veins of 2-4
    (IRON_ORE,     1,  8, 60, 3),
    // Copper: mid layers, veins of 3-4
    (COPPER_ORE,   3, 10, 50, 3),
    // Gold: deep only, veins of 2-3
    (GOLD_ORE,     1,  5, 30, 2),
    // Lapis: mid-deep, veins of 2-3
    (LAPIS_ORE,    1,  6, 25, 2),
    // Redstone: deep, veins of 2-4
    (REDSTONE_ORE, 1,  4, 35, 3),
    // Diamond: very deep, veins of 1-2
    (DIAMOND_ORE,  1,  3, 15, 2),
    // Emerald: rare, single blocks
    (EMERALD_ORE,  1,  6,  8, 1),
    // Gravel: pockets throughout
    (GRAVEL,       1, 10, 40, 3),
];

fn place_ores(blocks: &mut [i32; 4096], chunk_x: i32, chunk_z: i32) {
    let chunk_seed = ore_hash(chunk_x, chunk_z, 0, 0xDEAD_BEEF);

    for (ore_id, min_y, max_y, chance, vein_size) in ORE_CONFIG {
        let mut seed = chunk_seed.wrapping_add(*ore_id as u32);

        for local_y in *min_y..=*max_y {
            for x in 0..16_usize {
                for z in 0..16_usize {
                    seed = ore_hash(x as i32, local_y as i32, z as i32, seed);
                    // Check if this position should have an ore
                    if (seed % 1000) < *chance {
                        let idx = local_y * 256 + z * 16 + x;
                        // Only replace stone
                        if blocks[idx] == STONE {
                            blocks[idx] = *ore_id;
                        }

                        // Extend vein to adjacent blocks
                        if *vein_size > 1 {
                            let directions: &[(i32, i32, i32)] = &[
                                (1, 0, 0), (-1, 0, 0),
                                (0, 1, 0), (0, -1, 0),
                                (0, 0, 1), (0, 0, -1),
                            ];
                            let mut extend_seed = seed;
                            for (i, (dx, dy, dz)) in directions.iter().enumerate() {
                                if i as u32 >= *vein_size - 1 {
                                    break;
                                }
                                extend_seed = ore_hash(x as i32 + dx, local_y as i32 + dy, z as i32 + dz, extend_seed);
                                if extend_seed % 2 == 0 {
                                    let nx = x as i32 + dx;
                                    let ny = local_y as i32 + dy;
                                    let nz = z as i32 + dz;
                                    if nx >= 0 && nx < 16 && ny >= 1 && ny <= 10 && nz >= 0 && nz < 16 {
                                        let nidx = ny as usize * 256 + nz as usize * 16 + nx as usize;
                                        if blocks[nidx] == STONE {
                                            blocks[nidx] = *ore_id;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
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
        // All columns should have height at y=-51 (grass_block)
        // Heightmap value = world_y - MIN_Y + 1 = -51 - (-64) + 1 = 14
        let first_value = heightmap[0] & 0x1FF; // 9-bit mask
        assert_eq!(first_value, 14, "Expected heightmap value 14 for grass at y=-51");
    }

    #[test]
    fn test_flat_chunk_has_ores() {
        let chunk = generate_flat_chunk_at(5, 3);
        // Count non-stone, non-bedrock, non-dirt, non-grass, non-air blocks
        let section = &chunk.sections[0];
        let mut ore_count = 0;
        for y in 1..=10 {
            for z in 0..16 {
                for x in 0..16 {
                    let state = section.get_block(x, y, z);
                    if state != STONE && state != AIR {
                        ore_count += 1;
                    }
                }
            }
        }
        // Should have some ores (at least a few dozen per chunk)
        assert!(ore_count > 10, "Expected ores in chunk, found {}", ore_count);
    }

    #[test]
    fn test_ore_placement_deterministic() {
        let chunk1 = generate_flat_chunk_at(7, 11);
        let chunk2 = generate_flat_chunk_at(7, 11);
        // Same coordinates should produce identical chunks
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    assert_eq!(
                        chunk1.sections[0].get_block(x, y, z),
                        chunk2.sections[0].get_block(x, y, z),
                        "Mismatch at ({}, {}, {})", x, y, z
                    );
                }
            }
        }
    }

    #[test]
    fn test_different_chunks_different_ores() {
        let chunk1 = generate_flat_chunk_at(0, 0);
        let chunk2 = generate_flat_chunk_at(100, 100);
        // Different coordinates should produce different ore patterns
        let mut differences = 0;
        for y in 1..=10 {
            for z in 0..16 {
                for x in 0..16 {
                    if chunk1.sections[0].get_block(x, y, z) != chunk2.sections[0].get_block(x, y, z) {
                        differences += 1;
                    }
                }
            }
        }
        assert!(differences > 0, "Expected different ore patterns for different chunks");
    }
}
