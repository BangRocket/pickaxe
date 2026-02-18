use bytes::{BufMut, BytesMut};
use pickaxe_nbt::{nbt_compound, NbtValue};
use pickaxe_protocol_core::{write_varint, ChunkLightData, InternalPacket};
use std::collections::HashMap;

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
    /// Flat block state array for mutation. Populated on first set_block call.
    /// Layout: [y * 256 + z * 16 + x] = state_id
    blocks: Option<Box<[i32; 4096]>>,
}

impl ChunkSection {
    /// Create an empty (all air) section.
    pub fn empty() -> Self {
        Self {
            block_count: 0,
            palette: vec![0], // air
            block_data: None,
            bits_per_entry: 0,
            blocks: None,
        }
    }

    /// Create a single-value section (all blocks are the same state ID).
    pub fn single_value(state_id: i32) -> Self {
        Self {
            block_count: if state_id == 0 { 0 } else { 4096 },
            palette: vec![state_id],
            block_data: None,
            bits_per_entry: 0,
            blocks: None,
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
            blocks: None,
        }
    }

    /// Get a single block's state ID.
    pub fn get_block(&self, x: usize, y: usize, z: usize) -> i32 {
        let index = y * 256 + z * 16 + x;
        // If we have a flat blocks array, use it directly
        if let Some(ref blocks) = self.blocks {
            return blocks[index];
        }
        // Otherwise decode from palette
        if self.palette.len() == 1 {
            return self.palette[0];
        }
        if let Some(ref data) = self.block_data {
            let entries_per_long = 64 / self.bits_per_entry as usize;
            let long_index = index / entries_per_long;
            let bit_index = (index % entries_per_long) * self.bits_per_entry as usize;
            let mask = (1u64 << self.bits_per_entry) - 1;
            let palette_idx = ((data[long_index] as u64 >> bit_index) & mask) as usize;
            self.palette.get(palette_idx).copied().unwrap_or(0)
        } else {
            0
        }
    }

    /// Set a single block in this section. Returns the old state ID.
    /// Ensures the flat blocks array is populated, then re-encodes the palette.
    pub fn set_block(&mut self, x: usize, y: usize, z: usize, state_id: i32) -> i32 {
        self.ensure_blocks_array();
        let index = y * 256 + z * 16 + x;
        let blocks = self.blocks.as_mut().unwrap();
        let old = blocks[index];
        blocks[index] = state_id;
        self.rebuild_palette();
        old
    }

    /// Populate the flat blocks array from the palette encoding.
    fn ensure_blocks_array(&mut self) {
        if self.blocks.is_some() {
            return;
        }
        let mut blocks = Box::new([0i32; 4096]);
        if self.palette.len() == 1 {
            let val = self.palette[0];
            if val != 0 {
                blocks.fill(val);
            }
        } else if let Some(ref data) = self.block_data {
            let entries_per_long = 64 / self.bits_per_entry as usize;
            let mask = (1u64 << self.bits_per_entry) - 1;
            for i in 0..4096 {
                let long_index = i / entries_per_long;
                let bit_index = (i % entries_per_long) * self.bits_per_entry as usize;
                let palette_idx = ((data[long_index] as u64 >> bit_index) & mask) as usize;
                blocks[i] = self.palette.get(palette_idx).copied().unwrap_or(0);
            }
        }
        self.blocks = Some(blocks);
    }

    /// Rebuild palette and packed data from the flat blocks array.
    fn rebuild_palette(&mut self) {
        let blocks = self.blocks.as_ref().unwrap();

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

        self.block_count = block_count;

        if palette.len() == 1 {
            self.palette = palette;
            self.block_data = None;
            self.bits_per_entry = 0;
            return;
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

        self.palette = palette;
        self.block_data = Some(data);
        self.bits_per_entry = bits_per_entry;
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

    /// Get block state at chunk-local coordinates.
    /// local_x/local_z: 0..15, world_y: absolute world Y coordinate.
    pub fn get_block(&self, local_x: usize, world_y: i32, local_z: usize) -> i32 {
        let section_idx = ((world_y - MIN_Y) / 16) as usize;
        if section_idx >= SECTION_COUNT {
            return 0;
        }
        let local_y = ((world_y - MIN_Y) % 16) as usize;
        self.sections[section_idx].get_block(local_x, local_y, local_z)
    }

    /// Set block state at chunk-local coordinates. Returns the old state ID.
    /// local_x/local_z: 0..15, world_y: absolute world Y coordinate.
    pub fn set_block(&mut self, local_x: usize, world_y: i32, local_z: usize, state_id: i32) -> i32 {
        let section_idx = ((world_y - MIN_Y) / 16) as usize;
        if section_idx >= SECTION_COUNT {
            return 0;
        }
        let local_y = ((world_y - MIN_Y) % 16) as usize;
        self.sections[section_idx].set_block(local_x, local_y, local_z, state_id)
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
                        let block_state = self.sections[section_idx].get_block(x, local_y, z);
                        if block_state != 0 {
                            let world_y = MIN_Y + (section_idx as i32) * 16 + local_y as i32;
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

    /// Serialize this chunk to Anvil NBT format.
    pub fn to_nbt(&self, chunk_x: i32, chunk_z: i32, last_update: i64) -> NbtValue {
        const DATA_VERSION: i32 = 3955; // MC 1.21.1

        let mut sections_list = Vec::new();

        for (i, _section) in self.sections.iter().enumerate() {
            let section_y = (i as i32) + (MIN_Y / 16); // -4 to 19

            // Build palette NBT from actual block data
            let blocks = self.get_section_blocks(i);
            let mut palette_map: Vec<i32> = Vec::new();
            let mut idx_map = HashMap::new();
            let mut indices = [0u16; 4096];
            for (bi, &state_id) in blocks.iter().enumerate() {
                let idx = *idx_map.entry(state_id).or_insert_with(|| {
                    let idx = palette_map.len();
                    palette_map.push(state_id);
                    idx
                });
                indices[bi] = idx as u16;
            }

            let mut palette_nbt = Vec::new();
            for &state_id in &palette_map {
                if let Some((name, props)) = pickaxe_data::block_state_to_properties(state_id) {
                    let full_name = if name.contains(':') {
                        name.to_string()
                    } else {
                        format!("minecraft:{}", name)
                    };
                    if props.is_empty() {
                        palette_nbt.push(nbt_compound! {
                            "Name" => NbtValue::String(full_name)
                        });
                    } else {
                        let prop_entries: Vec<(String, NbtValue)> = props.iter()
                            .map(|(k, v)| (k.to_string(), NbtValue::String(v.to_string())))
                            .collect();
                        palette_nbt.push(NbtValue::Compound(vec![
                            ("Name".into(), NbtValue::String(full_name)),
                            ("Properties".into(), NbtValue::Compound(prop_entries)),
                        ]));
                    }
                } else {
                    palette_nbt.push(nbt_compound! {
                        "Name" => NbtValue::String("minecraft:air".into())
                    });
                }
            }

            let mut block_states_entries = vec![
                ("palette".into(), NbtValue::List(palette_nbt)),
            ];

            // Only write data array if palette has >1 entry
            if palette_map.len() > 1 {
                let bits = std::cmp::max(4, (palette_map.len() as f64).log2().ceil() as u32);
                let entries_per_long = 64 / bits as usize;
                let longs_needed = (4096 + entries_per_long - 1) / entries_per_long;
                let mask = (1u64 << bits) - 1;
                let mut data = vec![0i64; longs_needed];
                for (bi, &idx) in indices.iter().enumerate() {
                    let long_index = bi / entries_per_long;
                    let bit_index = (bi % entries_per_long) * bits as usize;
                    data[long_index] |= ((idx as u64 & mask) << bit_index) as i64;
                }
                block_states_entries.push(("data".into(), NbtValue::LongArray(data)));
            }

            sections_list.push(NbtValue::Compound(vec![
                ("Y".into(), NbtValue::Byte(section_y as i8)),
                ("block_states".into(), NbtValue::Compound(block_states_entries)),
            ]));
        }

        let heightmap = self.compute_heightmap();

        nbt_compound! {
            "DataVersion" => NbtValue::Int(DATA_VERSION),
            "xPos" => NbtValue::Int(chunk_x),
            "zPos" => NbtValue::Int(chunk_z),
            "yPos" => NbtValue::Int(MIN_Y / 16),
            "Status" => NbtValue::String("full".into()),
            "LastUpdate" => NbtValue::Long(last_update),
            "sections" => NbtValue::List(sections_list),
            "Heightmaps" => nbt_compound! {
                "MOTION_BLOCKING" => NbtValue::LongArray(heightmap)
            }
        }
    }

    /// Deserialize a chunk from Anvil NBT.
    pub fn from_nbt(nbt: &NbtValue) -> Option<Self> {
        let sections_nbt = nbt.get("sections")?.as_list()?;
        let mut chunk = Chunk::new();

        for section_nbt in sections_nbt {
            // Use continue on parse failures so one bad section doesn't discard the whole chunk
            let y = match section_nbt.get("Y").and_then(|v| v.as_byte()) {
                Some(y) => y,
                None => continue,
            };
            let section_idx = (y as i32 - (MIN_Y / 16)) as usize;
            if section_idx >= SECTION_COUNT {
                continue;
            }

            let block_states = match section_nbt.get("block_states") {
                Some(bs) => bs,
                None => continue,
            };

            let palette_nbt = match block_states.get("palette").and_then(|v| v.as_list()) {
                Some(p) if !p.is_empty() => p,
                _ => continue,
            };

            // Build palette: map NBT names + properties to state IDs
            let mut palette_ids: Vec<i32> = Vec::new();
            for entry in palette_nbt {
                let name = match entry.get("Name").and_then(|v| v.as_str()) {
                    Some(n) => n,
                    None => continue,
                };
                let short_name = name.strip_prefix("minecraft:").unwrap_or(name);
                let state_id = if let Some(props_nbt) = entry.get("Properties") {
                    // Extract property key-value pairs from NBT compound
                    if let NbtValue::Compound(entries) = props_nbt {
                        let props: Vec<(&str, &str)> = entries.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.as_str(), s)))
                            .collect();
                        pickaxe_data::block_name_with_properties_to_state(short_name, &props)
                            .unwrap_or_else(|| pickaxe_data::block_name_to_default_state(short_name).unwrap_or(0))
                    } else {
                        pickaxe_data::block_name_to_default_state(short_name).unwrap_or(0)
                    }
                } else {
                    pickaxe_data::block_name_to_default_state(short_name).unwrap_or(0)
                };
                palette_ids.push(state_id);
            }

            if palette_ids.len() == 1 {
                // Single-valued section
                chunk.sections[section_idx] = ChunkSection::single_value(palette_ids[0]);
            } else if let Some(data) = block_states.get("data").and_then(|v| v.as_long_array()) {
                // Multi-valued: read packed data
                let bits = std::cmp::max(4, (palette_ids.len() as f64).log2().ceil() as u32);
                let entries_per_long = 64 / bits as usize;
                let mask = (1u64 << bits) - 1;

                let mut blocks = [0i32; 4096];
                for i in 0..4096 {
                    let long_index = i / entries_per_long;
                    let bit_index = (i % entries_per_long) * bits as usize;
                    if long_index < data.len() {
                        let palette_idx = ((data[long_index] as u64 >> bit_index) & mask) as usize;
                        blocks[i] = palette_ids.get(palette_idx).copied().unwrap_or(0);
                    }
                }
                chunk.sections[section_idx] = ChunkSection::from_blocks(&blocks);
            }
        }

        Some(chunk)
    }

    /// Helper: get a section's blocks as a flat array (Y * 256 + Z * 16 + X ordering).
    fn get_section_blocks(&self, section_idx: usize) -> [i32; 4096] {
        let section = &self.sections[section_idx];
        let mut blocks = [0i32; 4096];
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    blocks[y * 256 + z * 16 + x] = section.get_block(x, y, z);
                }
            }
        }
        blocks
    }

    /// Build the full chunk data + light packet.
    pub fn to_packet(&self, chunk_x: i32, chunk_z: i32) -> InternalPacket {
        let data = self.serialize_sections();
        let heightmap_data = self.compute_heightmap();

        let heightmaps = nbt_compound! {
            "MOTION_BLOCKING" => NbtValue::LongArray(heightmap_data)
        };

        // Sky light: 24 block sections + 2 boundary sections = 26 sections (bits 0..25)
        // Set all bits in the sky light mask to indicate all sections have sky light
        let sky_light_mask = vec![0x03FFFFFFi64]; // bits 0..25 set (26 sections)
        // Full sky light = 15 for every block: 4 bits per block, 2 blocks per byte = 0xFF
        let full_sky_light = vec![0xFFu8; 2048];
        let sky_light_arrays: Vec<Vec<u8>> = (0..26).map(|_| full_sky_light.clone()).collect();

        InternalPacket::ChunkDataAndUpdateLight {
            chunk_x,
            chunk_z,
            heightmaps,
            data,
            block_entities: Vec::new(),
            light_data: ChunkLightData {
                sky_light_mask,
                block_light_mask: vec![0i64],
                empty_sky_light_mask: vec![0i64],
                empty_block_light_mask: vec![0i64],
                sky_light_arrays,
                block_light_arrays: Vec::new(),
            },
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
    fn test_flat_section_encoding() {
        use crate::generator::*;
        let chunk = generate_flat_chunk();
        let section = &chunk.sections[0];
        // New layout: bedrock(1) + stone+ores(10) + dirt(2) + grass(1) = 14 layers
        // At least bedrock, stone, dirt, grass, air + ores in palette
        assert!(section.palette.len() >= 4);
        assert!(section.block_count > 1024); // more blocks than the old 4-layer layout

        let mut buf = BytesMut::new();
        section.write_to(&mut buf);
        let block_count_val = i16::from_be_bytes([buf[0], buf[1]]);
        assert!(block_count_val > 1024);
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

    #[test]
    fn test_section_get_block() {
        let section = ChunkSection::single_value(1); // all stone
        assert_eq!(section.get_block(0, 0, 0), 1);
        assert_eq!(section.get_block(15, 15, 15), 1);
    }

    #[test]
    fn test_section_set_block() {
        let mut section = ChunkSection::single_value(1); // all stone
        let old = section.set_block(5, 5, 5, 0); // set to air
        assert_eq!(old, 1);
        assert_eq!(section.get_block(5, 5, 5), 0);
        assert_eq!(section.get_block(0, 0, 0), 1); // other blocks unchanged
        assert_eq!(section.block_count, 4095);
    }

    #[test]
    fn test_chunk_get_set_block() {
        use crate::generator::*;
        let mut chunk = generate_flat_chunk();
        // Bedrock at y=-64
        assert_eq!(chunk.get_block(0, -64, 0), BEDROCK);
        // Grass at y=-51
        assert_eq!(chunk.get_block(0, -51, 0), GRASS_BLOCK);
        // Air at y=-50
        assert_eq!(chunk.get_block(0, -50, 0), AIR);
        // Dirt at y=-52
        assert_eq!(chunk.get_block(0, -52, 0), DIRT);

        // Break the grass block
        let old = chunk.set_block(0, -51, 0, AIR);
        assert_eq!(old, GRASS_BLOCK);
        assert_eq!(chunk.get_block(0, -51, 0), AIR);

        // Place stone at y=-50
        chunk.set_block(0, -50, 0, STONE);
        assert_eq!(chunk.get_block(0, -50, 0), STONE);
    }

    #[test]
    fn test_section_roundtrip_after_mutation() {
        use crate::generator::*;
        let mut chunk = generate_flat_chunk();
        // Break a block and verify serialization still works
        chunk.set_block(8, -51, 8, AIR);
        let data = chunk.serialize_sections();
        assert!(!data.is_empty());
    }

    #[test]
    fn test_chunk_nbt_roundtrip() {
        use crate::generator::*;
        let chunk = generate_flat_chunk();
        let nbt = chunk.to_nbt(0, 0, 1000);
        let restored = Chunk::from_nbt(&nbt).unwrap();
        assert_eq!(restored.get_block(0, -64, 0), BEDROCK);
        // y=-63 is stone (may have ore at 0,0 due to hash, check stone layer exists)
        let block_63 = restored.get_block(0, -63, 0);
        assert_ne!(block_63, AIR, "y=-63 should not be air");
        assert_eq!(restored.get_block(0, -52, 0), DIRT);
        assert_eq!(restored.get_block(0, -51, 0), GRASS_BLOCK);
        assert_eq!(restored.get_block(0, -50, 0), AIR);
        assert_eq!(restored.get_block(8, -64, 8), BEDROCK);
    }

    #[test]
    fn test_chunk_nbt_roundtrip_after_mutation() {
        use crate::generator::*;
        let mut chunk = generate_flat_chunk();
        chunk.set_block(5, -51, 5, STONE);
        chunk.set_block(10, -50, 10, DIRT);
        let nbt = chunk.to_nbt(3, -2, 500);
        let restored = Chunk::from_nbt(&nbt).unwrap();
        assert_eq!(restored.get_block(5, -51, 5), STONE);
        assert_eq!(restored.get_block(10, -50, 10), DIRT);
        assert_eq!(restored.get_block(0, -51, 0), GRASS_BLOCK);
    }

    #[test]
    fn test_chunk_nbt_empty_sections() {
        use crate::generator::AIR;
        let chunk = Chunk::new();
        let nbt = chunk.to_nbt(0, 0, 0);
        let restored = Chunk::from_nbt(&nbt).unwrap();
        assert_eq!(restored.get_block(0, 0, 0), AIR);
    }
}
