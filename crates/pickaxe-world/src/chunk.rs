use bytes::{BufMut, BytesMut};
use pickaxe_nbt::{nbt_compound, NbtValue};
use pickaxe_protocol_core::{write_varint, ChunkLightData, InternalPacket};

/// Total number of sections in a chunk (from y=-64 to y=320, 384 blocks / 16 = 24 sections).
pub const SECTION_COUNT: usize = 24;
/// Minimum Y coordinate.
pub const MIN_Y: i32 = -64;

/// A 16x16x16 chunk section.
#[derive(Clone)]
pub struct ChunkSection {
    /// Block count (non-air) for the section.
    pub block_count: i16,
    /// Block state palette. Index 0 is always the default (air = 0).
    pub palette: Vec<i32>,
    /// If palette has 1 entry: single-valued (no data array needed).
    /// If palette has >1 entry: indices into palette, packed into longs.
    pub block_data: Option<Vec<i64>>,
    /// Bits per entry for the block data.
    pub bits_per_entry: u8,
}

impl ChunkSection {
    /// Create an empty (all air) section.
    pub fn empty() -> Self {
        Self {
            block_count: 0,
            palette: vec![0], // air
            block_data: None,
            bits_per_entry: 0,
        }
    }

    /// Create a single-value section (all blocks are the same state ID).
    pub fn single_value(state_id: i32) -> Self {
        Self {
            block_count: if state_id == 0 { 0 } else { 4096 },
            palette: vec![state_id],
            block_data: None,
            bits_per_entry: 0,
        }
    }

    /// Create a section with a mixed palette. blocks is [y][z][x] = 16*16*16 = 4096 entries.
    pub fn from_blocks(blocks: &[i32; 4096]) -> Self {
        let mut palette = Vec::new();
        let mut palette_map = std::collections::HashMap::new();
        let mut indices = [0u16; 4096];
        let mut block_count: i16 = 0;

        for (i, &state_id) in blocks.iter().enumerate() {
            if state_id != 0 {
                block_count += 1;
            }
            let idx = *palette_map.entry(state_id).or_insert_with(|| {
                let idx = palette.len();
                palette.push(state_id);
                idx
            });
            indices[i] = idx as u16;
        }

        if palette.len() == 1 {
            return Self::single_value(palette[0]);
        }

        let bits_per_entry = std::cmp::max(4, (palette.len() as f64).log2().ceil() as u8);
        let entries_per_long = 64 / bits_per_entry as usize;
        let longs_needed = (4096 + entries_per_long - 1) / entries_per_long;
        let mask = (1u64 << bits_per_entry) - 1;

        let mut data = vec![0i64; longs_needed];
        for (i, &idx) in indices.iter().enumerate() {
            let long_index = i / entries_per_long;
            let bit_index = (i % entries_per_long) * bits_per_entry as usize;
            data[long_index] |= ((idx as u64 & mask) << bit_index) as i64;
        }

        Self {
            block_count,
            palette,
            block_data: Some(data),
            bits_per_entry,
        }
    }

    /// Serialize this section for the chunk data packet.
    pub fn write_to(&self, buf: &mut BytesMut) {
        buf.put_i16(self.block_count);

        // Block states — paletted container
        buf.put_u8(self.bits_per_entry);

        if self.bits_per_entry == 0 {
            // Single-valued: write the single palette entry, then 0 longs
            write_varint(buf, self.palette[0]);
            write_varint(buf, 0); // data array length = 0
        } else {
            // Indirect palette
            write_varint(buf, self.palette.len() as i32);
            for &entry in &self.palette {
                write_varint(buf, entry);
            }
            if let Some(ref data) = self.block_data {
                write_varint(buf, data.len() as i32);
                for &long in data {
                    buf.put_i64(long);
                }
            }
        }

        // Biomes — single-valued (plains = 0)
        buf.put_u8(0); // bits per entry = 0 (single value)
        write_varint(buf, 0); // palette entry: biome ID 0 (plains)
        write_varint(buf, 0); // data array length = 0
    }
}

/// A full chunk column (24 sections).
pub struct Chunk {
    pub sections: Vec<ChunkSection>,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            sections: (0..SECTION_COUNT).map(|_| ChunkSection::empty()).collect(),
        }
    }

    /// Serialize all sections into the chunk data byte array.
    pub fn serialize_sections(&self) -> Vec<u8> {
        let mut buf = BytesMut::new();
        for section in &self.sections {
            section.write_to(&mut buf);
        }
        buf.to_vec()
    }

    /// Build a heightmap for MOTION_BLOCKING.
    /// Returns packed long array (256 entries, 9 bits each for 384 height range).
    pub fn compute_heightmap(&self) -> Vec<i64> {
        let mut heights = [0u16; 256]; // 16x16

        // Scan from top to bottom for each column
        for x in 0..16 {
            for z in 0..16 {
                let col_idx = z * 16 + x;
                'scan: for section_idx in (0..SECTION_COUNT).rev() {
                    for local_y in (0..16).rev() {
                        let section = &self.sections[section_idx];
                        let block_state = self.get_block_state(section, x, local_y, z);
                        if block_state != 0 {
                            // World Y = MIN_Y + section_idx * 16 + local_y
                            let world_y = MIN_Y + (section_idx as i32) * 16 + local_y as i32;
                            // Heightmap value = world_y - MIN_Y + 1 (1-indexed from bottom)
                            heights[col_idx] = (world_y - MIN_Y + 1) as u16;
                            break 'scan;
                        }
                    }
                }
            }
        }

        // Pack into longs: 9 bits per entry (for 384 range), 7 entries per long (7*9=63 bits)
        let bits_per_entry = 9;
        let entries_per_long = 64 / bits_per_entry;
        let longs_needed = (256 + entries_per_long - 1) / entries_per_long; // 37 longs
        let mask = (1u64 << bits_per_entry) - 1;

        let mut packed = vec![0i64; longs_needed];
        for (i, &h) in heights.iter().enumerate() {
            let long_index = i / entries_per_long;
            let bit_index = (i % entries_per_long) * bits_per_entry;
            packed[long_index] |= ((h as u64 & mask) << bit_index) as i64;
        }

        packed
    }

    fn get_block_state(&self, section: &ChunkSection, x: usize, y: usize, z: usize) -> i32 {
        if section.palette.len() == 1 {
            return section.palette[0];
        }
        if let Some(ref data) = section.block_data {
            let index = y * 256 + z * 16 + x;
            let entries_per_long = 64 / section.bits_per_entry as usize;
            let long_index = index / entries_per_long;
            let bit_index = (index % entries_per_long) * section.bits_per_entry as usize;
            let mask = (1u64 << section.bits_per_entry) - 1;
            let palette_idx = ((data[long_index] as u64 >> bit_index) & mask) as usize;
            section.palette.get(palette_idx).copied().unwrap_or(0)
        } else {
            0
        }
    }

    /// Build the full chunk data + light packet.
    pub fn to_packet(&self, chunk_x: i32, chunk_z: i32) -> InternalPacket {
        let data = self.serialize_sections();
        let heightmap_data = self.compute_heightmap();

        let heightmaps = nbt_compound! {
            "MOTION_BLOCKING" => NbtValue::LongArray(heightmap_data)
        };

        InternalPacket::ChunkDataAndUpdateLight {
            chunk_x,
            chunk_z,
            heightmaps,
            data,
            block_entities: Vec::new(),
            light_data: ChunkLightData::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_section_serialize() {
        let section = ChunkSection::empty();
        let mut buf = BytesMut::new();
        section.write_to(&mut buf);
        // Should have block_count(2) + bits_per_entry(1) + palette_varint + data_len_varint + biome data
        assert!(buf.len() > 0);
    }

    #[test]
    fn test_single_value_section() {
        let section = ChunkSection::single_value(1); // stone
        assert_eq!(section.block_count, 4096);
        assert_eq!(section.bits_per_entry, 0);
    }

    #[test]
    fn test_heightmap_packing() {
        let mut chunk = Chunk::new();
        // Set section 4 (y=-64+64=0..15 → but we want the first non-empty)
        // Actually section index = (world_y - MIN_Y) / 16
        // For flat world: bedrock at y=-64 → section 0, local_y=0
        chunk.sections[0] = ChunkSection::single_value(1); // bedrock
        let heightmap = chunk.compute_heightmap();
        assert_eq!(heightmap.len(), 37); // ceil(256/7) = 37
    }
}
