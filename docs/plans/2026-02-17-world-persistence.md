# M10: World Persistence Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Persist chunks (Anvil .mca), player data (vanilla .dat), and world metadata (level.dat) so the world survives server restarts.

**Architecture:** New `pickaxe-region` crate for Anvil I/O. NBT read support added to `pickaxe-nbt`. WorldState gets a background saver task via mpsc channel. Write-through on block changes, periodic player saves, graceful shutdown.

**Tech Stack:** Rust, flate2 (DEFLATE/gzip), pickaxe-nbt, bytes, tokio mpsc

---

### Task 1: Add NBT Read/Parse Support

**Files:**
- Modify: `crates/pickaxe-nbt/src/nbt.rs`
- Modify: `crates/pickaxe-nbt/Cargo.toml`

The NBT crate currently only has write support. We need a full parser that can read named root tags from byte slices.

**Step 1: Write failing tests**

Add to `crates/pickaxe-nbt/src/nbt.rs` at end of `mod tests`:

```rust
#[test]
fn test_roundtrip_simple_compound() {
    let nbt = NbtValue::Compound(vec![
        ("name".into(), NbtValue::String("test".into())),
        ("value".into(), NbtValue::Int(42)),
        ("flag".into(), NbtValue::Byte(1)),
    ]);
    let mut buf = BytesMut::new();
    nbt.write_root_named("", &mut buf);
    let (name, parsed) = NbtValue::read_root_named(&buf).unwrap();
    assert_eq!(name, "");
    assert_eq!(parsed, nbt);
}

#[test]
fn test_roundtrip_nested() {
    let nbt = NbtValue::Compound(vec![
        ("pos".into(), NbtValue::List(vec![
            NbtValue::Double(1.0),
            NbtValue::Double(2.0),
            NbtValue::Double(3.0),
        ])),
        ("data".into(), NbtValue::LongArray(vec![100, 200, 300])),
        ("bytes".into(), NbtValue::ByteArray(vec![1, 2, 3])),
        ("ints".into(), NbtValue::IntArray(vec![10, 20])),
    ]);
    let mut buf = BytesMut::new();
    nbt.write_root_named("Level", &mut buf);
    let (name, parsed) = NbtValue::read_root_named(&buf).unwrap();
    assert_eq!(name, "Level");
    assert_eq!(parsed, nbt);
}

#[test]
fn test_roundtrip_empty_list() {
    let nbt = NbtValue::Compound(vec![
        ("empty".into(), NbtValue::List(vec![])),
    ]);
    let mut buf = BytesMut::new();
    nbt.write_root_named("", &mut buf);
    let (_, parsed) = NbtValue::read_root_named(&buf).unwrap();
    assert_eq!(parsed, nbt);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p pickaxe-nbt`
Expected: FAIL — `read_root_named` doesn't exist

**Step 3: Implement NBT reader**

Add a `read` module or add directly to `nbt.rs`. The reader needs:

```rust
use std::io::{self, Read, Cursor};

impl NbtValue {
    /// Read a named root tag from bytes. Returns (name, value).
    pub fn read_root_named(data: &[u8]) -> io::Result<(String, NbtValue)> {
        let mut cursor = Cursor::new(data);
        let tag_type = read_u8(&mut cursor)?;
        if tag_type != TAG_COMPOUND {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Root must be compound"));
        }
        let name = read_nbt_string(&mut cursor)?;
        let value = read_payload(&mut cursor, TAG_COMPOUND)?;
        Ok((name, value))
    }

    /// Read an unnamed root tag (network format).
    pub fn read_root_network(data: &[u8]) -> io::Result<NbtValue> {
        let mut cursor = Cursor::new(data);
        let tag_type = read_u8(&mut cursor)?;
        if tag_type != TAG_COMPOUND {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Root must be compound"));
        }
        read_payload(&mut cursor, TAG_COMPOUND)
    }
}

fn read_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut buf = [0u8; 1];
    r.read_exact(&mut buf)?;
    Ok(buf[0])
}

fn read_i8(r: &mut impl Read) -> io::Result<i8> {
    Ok(read_u8(r)? as i8)
}

fn read_i16(r: &mut impl Read) -> io::Result<i16> {
    let mut buf = [0u8; 2];
    r.read_exact(&mut buf)?;
    Ok(i16::from_be_bytes(buf))
}

fn read_u16(r: &mut impl Read) -> io::Result<u16> {
    let mut buf = [0u8; 2];
    r.read_exact(&mut buf)?;
    Ok(u16::from_be_bytes(buf))
}

fn read_i32(r: &mut impl Read) -> io::Result<i32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(i32::from_be_bytes(buf))
}

fn read_i64(r: &mut impl Read) -> io::Result<i64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(i64::from_be_bytes(buf))
}

fn read_f32(r: &mut impl Read) -> io::Result<f32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(f32::from_be_bytes(buf))
}

fn read_f64(r: &mut impl Read) -> io::Result<f64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(f64::from_be_bytes(buf))
}

fn read_nbt_string(r: &mut impl Read) -> io::Result<String> {
    let len = read_u16(r)? as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn read_payload(r: &mut impl Read, tag_type: u8) -> io::Result<NbtValue> {
    match tag_type {
        TAG_BYTE => Ok(NbtValue::Byte(read_i8(r)?)),
        TAG_SHORT => Ok(NbtValue::Short(read_i16(r)?)),
        TAG_INT => Ok(NbtValue::Int(read_i32(r)?)),
        TAG_LONG => Ok(NbtValue::Long(read_i64(r)?)),
        TAG_FLOAT => Ok(NbtValue::Float(read_f32(r)?)),
        TAG_DOUBLE => Ok(NbtValue::Double(read_f64(r)?)),
        TAG_BYTE_ARRAY => {
            let len = read_i32(r)? as usize;
            let mut data = vec![0i8; len];
            for v in &mut data { *v = read_i8(r)?; }
            Ok(NbtValue::ByteArray(data))
        }
        TAG_STRING => Ok(NbtValue::String(read_nbt_string(r)?)),
        TAG_LIST => {
            let elem_type = read_u8(r)?;
            let len = read_i32(r)? as usize;
            let mut items = Vec::with_capacity(len);
            for _ in 0..len {
                items.push(read_payload(r, elem_type)?);
            }
            Ok(NbtValue::List(items))
        }
        TAG_COMPOUND => {
            let mut entries = Vec::new();
            loop {
                let child_type = read_u8(r)?;
                if child_type == TAG_END { break; }
                let name = read_nbt_string(r)?;
                let value = read_payload(r, child_type)?;
                entries.push((name, value));
            }
            Ok(NbtValue::Compound(entries))
        }
        TAG_INT_ARRAY => {
            let len = read_i32(r)? as usize;
            let mut data = Vec::with_capacity(len);
            for _ in 0..len { data.push(read_i32(r)?); }
            Ok(NbtValue::IntArray(data))
        }
        TAG_LONG_ARRAY => {
            let len = read_i32(r)? as usize;
            let mut data = Vec::with_capacity(len);
            for _ in 0..len { data.push(read_i64(r)?); }
            Ok(NbtValue::LongArray(data))
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, format!("Unknown tag type {}", tag_type))),
    }
}
```

Also add a helper method for extracting fields from compounds:

```rust
impl NbtValue {
    /// Get a named field from a compound tag.
    pub fn get(&self, key: &str) -> Option<&NbtValue> {
        match self {
            NbtValue::Compound(entries) => {
                entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
            }
            _ => None,
        }
    }

    /// Get as i32.
    pub fn as_int(&self) -> Option<i32> {
        match self { NbtValue::Int(v) => Some(*v), _ => None }
    }

    /// Get as i64.
    pub fn as_long(&self) -> Option<i64> {
        match self { NbtValue::Long(v) => Some(*v), _ => None }
    }

    /// Get as f32.
    pub fn as_float(&self) -> Option<f32> {
        match self { NbtValue::Float(v) => Some(*v), _ => None }
    }

    /// Get as f64.
    pub fn as_double(&self) -> Option<f64> {
        match self { NbtValue::Double(v) => Some(*v), _ => None }
    }

    /// Get as i8.
    pub fn as_byte(&self) -> Option<i8> {
        match self { NbtValue::Byte(v) => Some(*v), _ => None }
    }

    /// Get as string.
    pub fn as_str(&self) -> Option<&str> {
        match self { NbtValue::String(v) => Some(v), _ => None }
    }

    /// Get as list.
    pub fn as_list(&self) -> Option<&[NbtValue]> {
        match self { NbtValue::List(v) => Some(v), _ => None }
    }

    /// Get as long array.
    pub fn as_long_array(&self) -> Option<&[i64]> {
        match self { NbtValue::LongArray(v) => Some(v), _ => None }
    }

    /// Get as byte array.
    pub fn as_byte_array(&self) -> Option<&[i8]> {
        match self { NbtValue::ByteArray(v) => Some(v), _ => None }
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p pickaxe-nbt`
Expected: All 5 tests PASS

**Step 5: Commit**

```bash
git add crates/pickaxe-nbt/
git commit -m "feat(m10): add NBT read/parse support with accessor methods"
```

---

### Task 2: Create `pickaxe-region` Crate — Region File I/O

**Files:**
- Create: `crates/pickaxe-region/Cargo.toml`
- Create: `crates/pickaxe-region/src/lib.rs`
- Create: `crates/pickaxe-region/src/region_file.rs`
- Modify: `Cargo.toml` (workspace root — add member + workspace dep)

This crate handles reading/writing Anvil `.mca` region files. It knows nothing about chunks or NBT structure — it reads/writes raw compressed byte blobs keyed by chunk coordinates.

**Step 1: Create crate skeleton**

`crates/pickaxe-region/Cargo.toml`:
```toml
[package]
name = "pickaxe-region"
edition.workspace = true
version.workspace = true

[dependencies]
flate2 = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
```

Add to workspace root `Cargo.toml`:
- In `[workspace] members`: add `"crates/pickaxe-region"`
- In `[workspace.dependencies]`: add `pickaxe-region = { path = "crates/pickaxe-region" }`

`crates/pickaxe-region/src/lib.rs`:
```rust
mod region_file;
pub use region_file::*;
```

**Step 2: Implement RegionFile**

`crates/pickaxe-region/src/region_file.rs`:

```rust
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

const SECTOR_BYTES: usize = 4096;
const HEADER_SECTORS: usize = 2;
const COMPRESSION_DEFLATE: u8 = 2;

/// A single .mca region file handle.
pub struct RegionFile {
    file: File,
    locations: [u32; 1024],
    timestamps: [u32; 1024],
    /// Tracks which sectors are in use.
    used_sectors: Vec<bool>,
}

impl RegionFile {
    /// Open or create a region file.
    pub fn open(path: &Path) -> io::Result<Self> {
        let exists = path.exists();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        let mut locations = [0u32; 1024];
        let mut timestamps = [0u32; 1024];

        if exists {
            let file_len = file.metadata()?.len();
            if file_len >= (HEADER_SECTORS * SECTOR_BYTES) as u64 {
                // Read location table
                file.seek(SeekFrom::Start(0))?;
                let mut loc_buf = [0u8; 4096];
                file.read_exact(&mut loc_buf)?;
                for i in 0..1024 {
                    locations[i] = u32::from_be_bytes([
                        loc_buf[i * 4],
                        loc_buf[i * 4 + 1],
                        loc_buf[i * 4 + 2],
                        loc_buf[i * 4 + 3],
                    ]);
                }
                // Read timestamp table
                let mut ts_buf = [0u8; 4096];
                file.read_exact(&mut ts_buf)?;
                for i in 0..1024 {
                    timestamps[i] = u32::from_be_bytes([
                        ts_buf[i * 4],
                        ts_buf[i * 4 + 1],
                        ts_buf[i * 4 + 2],
                        ts_buf[i * 4 + 3],
                    ]);
                }
            }
        } else {
            // Write empty header
            let zeros = [0u8; SECTOR_BYTES];
            file.write_all(&zeros)?; // locations
            file.write_all(&zeros)?; // timestamps
            file.flush()?;
        }

        // Build used sectors bitmap
        let file_len = file.metadata()?.len() as usize;
        let total_sectors = (file_len + SECTOR_BYTES - 1) / SECTOR_BYTES;
        let total_sectors = total_sectors.max(HEADER_SECTORS);
        let mut used_sectors = vec![false; total_sectors];
        used_sectors[0] = true; // header sector 0
        used_sectors[1] = true; // header sector 1

        for &loc in &locations {
            if loc != 0 {
                let sector = (loc >> 8) as usize;
                let count = (loc & 0xFF) as usize;
                for s in sector..sector + count {
                    if s < used_sectors.len() {
                        used_sectors[s] = true;
                    }
                }
            }
        }

        Ok(Self { file, locations, timestamps, used_sectors })
    }

    /// Read a chunk's compressed NBT data. Returns None if chunk not stored.
    pub fn read_chunk(&mut self, local_x: usize, local_z: usize) -> io::Result<Option<Vec<u8>>> {
        let index = local_x + local_z * 32;
        let loc = self.locations[index];
        if loc == 0 {
            return Ok(None);
        }

        let sector = (loc >> 8) as u64;
        let _count = (loc & 0xFF) as usize;

        self.file.seek(SeekFrom::Start(sector * SECTOR_BYTES as u64))?;

        // Read chunk header: 4-byte length + 1-byte compression
        let mut header = [0u8; 5];
        self.file.read_exact(&mut header)?;
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let compression = header[4];

        if length <= 1 {
            return Ok(None);
        }

        // Read compressed data
        let data_len = length - 1; // subtract compression byte
        let mut compressed = vec![0u8; data_len];
        self.file.read_exact(&mut compressed)?;

        // Decompress
        let decompressed = match compression {
            COMPRESSION_DEFLATE => {
                let mut decoder = DeflateDecoder::new(&compressed[..]);
                let mut out = Vec::new();
                decoder.read_to_end(&mut out)?;
                out
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unsupported compression type: {}", compression),
                ));
            }
        };

        Ok(Some(decompressed))
    }

    /// Write a chunk's raw NBT data (will be DEFLATE compressed).
    pub fn write_chunk(&mut self, local_x: usize, local_z: usize, nbt_bytes: &[u8]) -> io::Result<()> {
        let index = local_x + local_z * 32;

        // Compress
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(nbt_bytes)?;
        let compressed = encoder.finish()?;

        // Total bytes: 4 (length) + 1 (compression) + compressed data
        let total = 5 + compressed.len();
        let sectors_needed = (total + SECTOR_BYTES - 1) / SECTOR_BYTES;

        // Free old sectors
        let old_loc = self.locations[index];
        if old_loc != 0 {
            let old_sector = (old_loc >> 8) as usize;
            let old_count = (old_loc & 0xFF) as usize;
            for s in old_sector..old_sector + old_count {
                if s < self.used_sectors.len() {
                    self.used_sectors[s] = false;
                }
            }
        }

        // Allocate new sectors
        let new_sector = self.allocate_sectors(sectors_needed);

        // Write chunk data
        self.file.seek(SeekFrom::Start(new_sector as u64 * SECTOR_BYTES as u64))?;
        let length = (compressed.len() + 1) as u32; // +1 for compression byte
        self.file.write_all(&length.to_be_bytes())?;
        self.file.write_all(&[COMPRESSION_DEFLATE])?;
        self.file.write_all(&compressed)?;

        // Pad to sector boundary
        let written = 5 + compressed.len();
        let padding = sectors_needed * SECTOR_BYTES - written;
        if padding > 0 {
            self.file.write_all(&vec![0u8; padding])?;
        }

        // Update location table
        self.locations[index] = ((new_sector as u32) << 8) | (sectors_needed as u32 & 0xFF);

        // Update timestamp
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        self.timestamps[index] = now;

        // Write header back
        self.write_header()?;
        self.file.flush()?;

        Ok(())
    }

    /// Find contiguous free sectors, extending the file if needed.
    fn allocate_sectors(&mut self, count: usize) -> usize {
        // Search for contiguous free block
        let mut start = HEADER_SECTORS;
        while start + count <= self.used_sectors.len() {
            let mut found = true;
            for s in start..start + count {
                if self.used_sectors[s] {
                    found = false;
                    start = s + 1;
                    break;
                }
            }
            if found {
                for s in start..start + count {
                    self.used_sectors[s] = true;
                }
                return start;
            }
        }

        // Extend file
        let new_start = self.used_sectors.len();
        self.used_sectors.resize(new_start + count, true);
        new_start
    }

    fn write_header(&mut self) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut loc_buf = [0u8; 4096];
        for i in 0..1024 {
            let bytes = self.locations[i].to_be_bytes();
            loc_buf[i * 4..i * 4 + 4].copy_from_slice(&bytes);
        }
        self.file.write_all(&loc_buf)?;

        let mut ts_buf = [0u8; 4096];
        for i in 0..1024 {
            let bytes = self.timestamps[i].to_be_bytes();
            ts_buf[i * 4..i * 4 + 4].copy_from_slice(&bytes);
        }
        self.file.write_all(&ts_buf)?;
        Ok(())
    }
}

/// Manages a directory of region files.
pub struct RegionStorage {
    dir: PathBuf,
    cache: HashMap<(i32, i32), RegionFile>,
}

impl RegionStorage {
    pub fn new(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        Ok(Self { dir, cache: HashMap::new() })
    }

    /// Read a chunk's raw NBT bytes from disk. Returns None if not saved.
    pub fn read_chunk(&mut self, chunk_x: i32, chunk_z: i32) -> io::Result<Option<Vec<u8>>> {
        let (region_x, region_z, local_x, local_z) = Self::chunk_to_region(chunk_x, chunk_z);
        let region = self.get_or_open(region_x, region_z)?;
        region.read_chunk(local_x, local_z)
    }

    /// Write a chunk's raw NBT bytes to disk.
    pub fn write_chunk(&mut self, chunk_x: i32, chunk_z: i32, nbt_bytes: &[u8]) -> io::Result<()> {
        let (region_x, region_z, local_x, local_z) = Self::chunk_to_region(chunk_x, chunk_z);
        let region = self.get_or_open(region_x, region_z)?;
        region.write_chunk(local_x, local_z, nbt_bytes)
    }

    fn get_or_open(&mut self, region_x: i32, region_z: i32) -> io::Result<&mut RegionFile> {
        if !self.cache.contains_key(&(region_x, region_z)) {
            let path = self.dir.join(format!("r.{}.{}.mca", region_x, region_z));
            let region = RegionFile::open(&path)?;
            self.cache.insert((region_x, region_z), region);
        }
        Ok(self.cache.get_mut(&(region_x, region_z)).unwrap())
    }

    fn chunk_to_region(chunk_x: i32, chunk_z: i32) -> (i32, i32, usize, usize) {
        let region_x = chunk_x >> 5;
        let region_z = chunk_z >> 5;
        let local_x = (chunk_x & 31) as usize;
        let local_z = (chunk_z & 31) as usize;
        (region_x, region_z, local_x, local_z)
    }
}
```

**Step 3: Write tests**

Add to `crates/pickaxe-region/src/region_file.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_create_and_write_read_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = RegionStorage::new(dir.path().join("region")).unwrap();

        let data = b"Hello chunk NBT data";
        storage.write_chunk(0, 0, data).unwrap();

        let read_back = storage.read_chunk(0, 0).unwrap();
        assert_eq!(read_back, Some(data.to_vec()));
    }

    #[test]
    fn test_read_nonexistent_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = RegionStorage::new(dir.path().join("region")).unwrap();
        let result = storage.read_chunk(5, 5).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_overwrite_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = RegionStorage::new(dir.path().join("region")).unwrap();

        storage.write_chunk(3, 7, b"first version").unwrap();
        storage.write_chunk(3, 7, b"second version, which is longer").unwrap();

        let result = storage.read_chunk(3, 7).unwrap();
        assert_eq!(result, Some(b"second version, which is longer".to_vec()));
    }

    #[test]
    fn test_multiple_chunks_same_region() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = RegionStorage::new(dir.path().join("region")).unwrap();

        storage.write_chunk(0, 0, b"chunk_0_0").unwrap();
        storage.write_chunk(1, 0, b"chunk_1_0").unwrap();
        storage.write_chunk(0, 1, b"chunk_0_1").unwrap();

        assert_eq!(storage.read_chunk(0, 0).unwrap(), Some(b"chunk_0_0".to_vec()));
        assert_eq!(storage.read_chunk(1, 0).unwrap(), Some(b"chunk_1_0".to_vec()));
        assert_eq!(storage.read_chunk(0, 1).unwrap(), Some(b"chunk_0_1".to_vec()));
        assert_eq!(storage.read_chunk(2, 2).unwrap(), None);
    }

    #[test]
    fn test_different_regions() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = RegionStorage::new(dir.path().join("region")).unwrap();

        // Chunk (0,0) → region (0,0), chunk (32,0) → region (1,0)
        storage.write_chunk(0, 0, b"region_0_0").unwrap();
        storage.write_chunk(32, 0, b"region_1_0").unwrap();

        assert_eq!(storage.read_chunk(0, 0).unwrap(), Some(b"region_0_0".to_vec()));
        assert_eq!(storage.read_chunk(32, 0).unwrap(), Some(b"region_1_0".to_vec()));

        // Verify two .mca files exist
        let region_dir = dir.path().join("region");
        let files: Vec<_> = fs::read_dir(&region_dir).unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_reopen_region() {
        let dir = tempfile::tempdir().unwrap();
        let region_dir = dir.path().join("region");

        // Write with one storage instance
        {
            let mut storage = RegionStorage::new(region_dir.clone()).unwrap();
            storage.write_chunk(5, 10, b"persistent data").unwrap();
        }

        // Read with a new instance (simulates server restart)
        {
            let mut storage = RegionStorage::new(region_dir).unwrap();
            let result = storage.read_chunk(5, 10).unwrap();
            assert_eq!(result, Some(b"persistent data".to_vec()));
        }
    }
}
```

Add `tempfile` to dev-dependencies in `crates/pickaxe-region/Cargo.toml`:
```toml
[dev-dependencies]
tempfile = "3"
```

**Step 4: Run tests**

Run: `cargo test -p pickaxe-region`
Expected: All 6 tests PASS

**Step 5: Commit**

```bash
git add crates/pickaxe-region/ Cargo.toml
git commit -m "feat(m10): add pickaxe-region crate with Anvil .mca file I/O"
```

---

### Task 3: Chunk NBT Serialization (Chunk ↔ NBT)

**Files:**
- Modify: `crates/pickaxe-world/src/chunk.rs` — add `to_nbt()` and `from_nbt()` methods
- Modify: `crates/pickaxe-world/Cargo.toml` — add `pickaxe-data` if not present

Chunk sections use a palette with block state IDs internally, but the Anvil NBT format stores block names (e.g. `"minecraft:stone"`) in the palette with optional Properties compound for block states. We need to convert between state IDs and name+properties.

For our flat world, all blocks are simple (no properties variants matter), so the initial implementation maps state ID → name via `pickaxe_data::block_state_to_name()` and name → state ID via `pickaxe_data::block_name_to_default_state()`.

**Step 1: Write failing tests**

Add to `crates/pickaxe-world/src/chunk.rs` tests module:

```rust
#[test]
fn test_chunk_nbt_roundtrip() {
    let chunk = generate_flat_chunk();
    let nbt = chunk.to_nbt(0, 0, 1000);
    let restored = Chunk::from_nbt(&nbt).unwrap();

    // Verify blocks match
    assert_eq!(restored.get_block(0, -64, 0), BEDROCK);
    assert_eq!(restored.get_block(0, -63, 0), DIRT);
    assert_eq!(restored.get_block(0, -62, 0), DIRT);
    assert_eq!(restored.get_block(0, -61, 0), GRASS_BLOCK);
    assert_eq!(restored.get_block(0, -60, 0), AIR);
    assert_eq!(restored.get_block(8, -64, 8), BEDROCK);
}

#[test]
fn test_chunk_nbt_roundtrip_after_mutation() {
    let mut chunk = generate_flat_chunk();
    chunk.set_block(5, -61, 5, STONE);
    chunk.set_block(10, -60, 10, DIRT);

    let nbt = chunk.to_nbt(3, -2, 500);
    let restored = Chunk::from_nbt(&nbt).unwrap();

    assert_eq!(restored.get_block(5, -61, 5), STONE);
    assert_eq!(restored.get_block(10, -60, 10), DIRT);
    assert_eq!(restored.get_block(0, -61, 0), GRASS_BLOCK); // unchanged
}

#[test]
fn test_chunk_nbt_empty_sections() {
    let chunk = Chunk::new(); // all air
    let nbt = chunk.to_nbt(0, 0, 0);
    let restored = Chunk::from_nbt(&nbt).unwrap();
    assert_eq!(restored.get_block(0, 0, 0), AIR);
}
```

**Step 2: Implement `to_nbt()` and `from_nbt()`**

Add to `Chunk` impl in `crates/pickaxe-world/src/chunk.rs`:

```rust
use pickaxe_nbt::{NbtValue, nbt_compound, nbt_list};

const DATA_VERSION: i32 = 3955; // MC 1.21.1

impl Chunk {
    /// Serialize this chunk to Anvil NBT format.
    pub fn to_nbt(&self, chunk_x: i32, chunk_z: i32, last_update: i64) -> NbtValue {
        let mut sections_list = Vec::new();

        for (i, section) in self.sections.iter().enumerate() {
            let section_y = (i as i32) + (MIN_Y / 16); // -4 to 19
            let mut section_nbt_entries = vec![
                ("Y".into(), NbtValue::Byte(section_y as i8)),
            ];

            // Build palette NBT: list of {Name: "minecraft:stone", Properties: {...}}
            let mut palette_nbt = Vec::new();
            // Ensure we have the flat blocks array for reading
            // We need to iterate all 4096 blocks to build the actual palette
            let blocks = self.get_section_blocks(i);
            let mut palette_map: Vec<i32> = Vec::new();
            let mut idx_map = std::collections::HashMap::new();
            let mut indices = [0u16; 4096];
            for (bi, &state_id) in blocks.iter().enumerate() {
                let idx = *idx_map.entry(state_id).or_insert_with(|| {
                    let idx = palette_map.len();
                    palette_map.push(state_id);
                    idx
                });
                indices[bi] = idx as u16;
            }

            for &state_id in &palette_map {
                let name = pickaxe_data::block_state_to_name(state_id)
                    .unwrap_or("minecraft:air");
                let full_name = if name.contains(':') {
                    name.to_string()
                } else {
                    format!("minecraft:{}", name)
                };
                palette_nbt.push(nbt_compound! {
                    "Name" => NbtValue::String(full_name)
                });
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

            section_nbt_entries.push((
                "block_states".into(),
                NbtValue::Compound(block_states_entries),
            ));

            sections_list.push(NbtValue::Compound(section_nbt_entries));
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

    /// Helper: get a section's blocks as a flat array.
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

    /// Deserialize a chunk from Anvil NBT.
    pub fn from_nbt(nbt: &NbtValue) -> Option<Self> {
        let sections_nbt = nbt.get("sections")?.as_list()?;
        let mut chunk = Chunk::new();

        for section_nbt in sections_nbt {
            let y = section_nbt.get("Y")?.as_byte()?;
            let section_idx = (y as i32 - (MIN_Y / 16)) as usize;
            if section_idx >= SECTION_COUNT {
                continue;
            }

            let block_states = match section_nbt.get("block_states") {
                Some(bs) => bs,
                None => continue, // no block data = air
            };

            let palette_nbt = block_states.get("palette")?.as_list()?;
            if palette_nbt.is_empty() {
                continue;
            }

            // Build palette: map NBT names to state IDs
            let mut palette_ids: Vec<i32> = Vec::new();
            for entry in palette_nbt {
                let name = entry.get("Name")?.as_str()?;
                let short_name = name.strip_prefix("minecraft:").unwrap_or(name);
                let state_id = pickaxe_data::block_name_to_default_state(short_name)
                    .unwrap_or(0);
                palette_ids.push(state_id);
            }

            if palette_ids.len() == 1 {
                // Single-valued section
                chunk.sections[section_idx] = ChunkSection::single_value(palette_ids[0]);
            } else {
                // Multi-valued: read packed data
                let data = block_states.get("data")?.as_long_array()?;
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
}
```

**Step 3: Run tests**

Run: `cargo test -p pickaxe-world`
Expected: All tests PASS (existing 8 + 3 new)

**Step 4: Commit**

```bash
git add crates/pickaxe-world/
git commit -m "feat(m10): add Chunk to_nbt/from_nbt for Anvil serialization"
```

---

### Task 4: Background Saver Task and WorldState Integration

**Files:**
- Modify: `crates/pickaxe-server/src/tick.rs` — WorldState gets RegionStorage + save channel
- Modify: `crates/pickaxe-server/src/main.rs` — spawn saver task, pass world_dir
- Modify: `crates/pickaxe-server/src/config.rs` — add `world_dir` config field
- Modify: `crates/pickaxe-server/Cargo.toml` — add `pickaxe-region` dependency

**Step 1: Add config field**

In `crates/pickaxe-server/src/config.rs`, add to `ServerConfig`:

```rust
#[serde(default = "default_world_dir")]
pub world_dir: String,
```

And:
```rust
fn default_world_dir() -> String { "world".to_string() }
```

Include it in the `Default` impl.

**Step 2: Define SaveOp and saver task**

Add to `crates/pickaxe-server/src/tick.rs` after imports:

```rust
use pickaxe_region::RegionStorage;

/// Operations queued for the background saver task.
pub enum SaveOp {
    /// Save a chunk. (chunk_x, chunk_z, serialized NBT bytes)
    Chunk(i32, i32, Vec<u8>),
    /// Save player data. (uuid, serialized gzip NBT bytes)
    Player(uuid::Uuid, Vec<u8>),
    /// Save level.dat. (serialized gzip NBT bytes)
    LevelDat(Vec<u8>),
    /// Shutdown: flush all pending writes, then stop.
    Shutdown(tokio::sync::oneshot::Sender<()>),
}

/// Runs on a background Tokio blocking task. Processes SaveOps sequentially.
pub fn run_saver_task(
    mut rx: mpsc::UnboundedReceiver<SaveOp>,
    world_dir: PathBuf,
) {
    let region_dir = world_dir.join("region");
    let playerdata_dir = world_dir.join("playerdata");
    let _ = std::fs::create_dir_all(&region_dir);
    let _ = std::fs::create_dir_all(&playerdata_dir);

    let mut region_storage = match RegionStorage::new(region_dir) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to init region storage: {}", e);
            return;
        }
    };

    while let Some(op) = rx.blocking_recv() {
        match op {
            SaveOp::Chunk(cx, cz, data) => {
                if let Err(e) = region_storage.write_chunk(cx, cz, &data) {
                    tracing::error!("Failed to save chunk ({}, {}): {}", cx, cz, e);
                }
            }
            SaveOp::Player(uuid, data) => {
                let path = playerdata_dir.join(format!("{}.dat", uuid));
                let tmp_path = playerdata_dir.join(format!("{}.dat.tmp", uuid));
                if let Err(e) = std::fs::write(&tmp_path, &data) {
                    tracing::error!("Failed to write player data {}: {}", uuid, e);
                } else if let Err(e) = std::fs::rename(&tmp_path, &path) {
                    tracing::error!("Failed to rename player data {}: {}", uuid, e);
                }
            }
            SaveOp::LevelDat(data) => {
                let path = world_dir.join("level.dat");
                let tmp_path = world_dir.join("level.dat.tmp");
                if let Err(e) = std::fs::write(&tmp_path, &data) {
                    tracing::error!("Failed to write level.dat: {}", e);
                } else if let Err(e) = std::fs::rename(&tmp_path, &path) {
                    tracing::error!("Failed to rename level.dat: {}", e);
                }
            }
            SaveOp::Shutdown(done) => {
                tracing::info!("Saver task shutting down");
                let _ = done.send(());
                return;
            }
        }
    }
}
```

**Step 3: Modify WorldState**

Change `WorldState` to include RegionStorage for loading and a save channel for writing:

```rust
pub struct WorldState {
    chunks: HashMap<ChunkPos, Chunk>,
    pub world_age: i64,
    pub time_of_day: i64,
    pub tick_count: u64,
    region_storage: RegionStorage,
    save_tx: mpsc::UnboundedSender<SaveOp>,
}
```

Update `WorldState::new()` to accept these parameters. Update `get_chunk_packet()`, `set_block()`, and `get_block()` to:
1. Load from region storage on cache miss (before generating)
2. Send `SaveOp::Chunk` after every `set_block()`

The `ensure_chunk()` helper loads-or-generates:

```rust
fn ensure_chunk(&mut self, pos: ChunkPos) -> &mut Chunk {
    if !self.chunks.contains_key(&pos) {
        // Try loading from disk
        if let Ok(Some(nbt_bytes)) = self.region_storage.read_chunk(pos.x, pos.z) {
            if let Ok((_, nbt)) = NbtValue::read_root_named(&nbt_bytes) {
                if let Some(chunk) = Chunk::from_nbt(&nbt) {
                    self.chunks.insert(pos, chunk);
                    return self.chunks.get_mut(&pos).unwrap();
                }
            }
        }
        // Generate and save
        let chunk = generate_flat_chunk();
        self.chunks.insert(pos, chunk);
        self.queue_chunk_save(pos);
    }
    self.chunks.get_mut(&pos).unwrap()
}
```

In `set_block()`, after mutating the chunk, call `self.queue_chunk_save(chunk_pos)`:

```rust
fn queue_chunk_save(&self, pos: ChunkPos) {
    if let Some(chunk) = self.chunks.get(&pos) {
        let nbt = chunk.to_nbt(pos.x, pos.z, self.world_age);
        let mut buf = BytesMut::new();
        nbt.write_root_named("", &mut buf);
        let _ = self.save_tx.send(SaveOp::Chunk(pos.x, pos.z, buf.to_vec()));
    }
}
```

**Step 4: Update main.rs**

In `main()`:
1. Create save channel: `let (save_tx, save_rx) = mpsc::unbounded_channel::<SaveOp>();`
2. Create RegionStorage for WorldState: `RegionStorage::new(world_dir.join("region"))?`
3. Spawn saver task: `tokio::task::spawn_blocking(move || run_saver_task(save_rx, world_dir));`
4. Pass `save_tx` and `region_storage` to `run_tick_loop()`

**Step 5: Update run_tick_loop signature**

Add `world_dir: PathBuf`, `save_tx: mpsc::UnboundedSender<SaveOp>` parameters. Create `WorldState` with the new parameters.

**Step 6: Build and verify**

Run: `cargo build`
Expected: Compiles. The write-through save path is now wired up.

**Step 7: Commit**

```bash
git add crates/pickaxe-server/ crates/pickaxe-region/
git commit -m "feat(m10): wire up write-through chunk saving with background saver"
```

---

### Task 5: Player Data Persistence

**Files:**
- Modify: `crates/pickaxe-server/src/tick.rs` — save/load player data on join/leave

**Step 1: Implement player data serialization**

Add helper functions to tick.rs:

```rust
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

/// Serialize a player's state to gzip-compressed NBT bytes.
fn serialize_player_data(world: &World, entity: hecs::Entity) -> Option<Vec<u8>> {
    let pos = world.get::<&Position>(entity).ok()?;
    let rot = world.get::<&Rotation>(entity).ok()?;
    let on_ground = world.get::<&OnGround>(entity).ok()?;
    let health = world.get::<&Health>(entity).ok()?;
    let food = world.get::<&FoodData>(entity).ok()?;
    let fall_dist = world.get::<&FallDistance>(entity).ok()?;
    let inv = world.get::<&Inventory>(entity).ok()?;
    let held = world.get::<&HeldSlot>(entity).ok()?;
    let gm = world.get::<&PlayerGameMode>(entity).ok()?;

    // Build inventory list
    let mut inv_list = Vec::new();
    for (i, slot) in inv.slots.iter().enumerate() {
        if let Some(item) = slot {
            let slot_byte = match i {
                0..=8 => i as i8,           // hotbar → slots 0-8 in NBT (MC maps hotbar to 0-8)
                // MC NBT: 0-8=hotbar, 9-35=main inventory, 100-103=armor, -106=offhand
                // Our ECS: 0=craft out, 1-4=craft, 5-8=armor, 9-35=main, 36-44=hotbar, 45=offhand
                36..=44 => (i - 36) as i8,  // hotbar slots
                9..=35 => i as i8,          // main inventory
                5..=8 => (100 + i - 5) as i8, // armor: ECS 5-8 → NBT 100-103
                45 => -106i8,               // offhand
                _ => continue,
            };
            let item_name = pickaxe_data::item_id_to_name(item.item_id)
                .unwrap_or("minecraft:air");
            let full_name = if item_name.contains(':') {
                item_name.to_string()
            } else {
                format!("minecraft:{}", item_name)
            };
            inv_list.push(nbt_compound! {
                "Slot" => NbtValue::Byte(slot_byte),
                "id" => NbtValue::String(full_name),
                "count" => NbtValue::Byte(item.count)
            });
        }
    }

    let game_type = match gm.0 {
        GameMode::Survival => 0,
        GameMode::Creative => 1,
        GameMode::Adventure => 2,
        GameMode::Spectator => 3,
    };

    let nbt = nbt_compound! {
        "DataVersion" => NbtValue::Int(3955),
        "Pos" => nbt_list![
            NbtValue::Double(pos.0.x),
            NbtValue::Double(pos.0.y),
            NbtValue::Double(pos.0.z)
        ],
        "Rotation" => nbt_list![
            NbtValue::Float(rot.yaw),
            NbtValue::Float(rot.pitch)
        ],
        "OnGround" => NbtValue::Byte(if on_ground.0 { 1 } else { 0 }),
        "Health" => NbtValue::Float(health.current),
        "FallDistance" => NbtValue::Float(fall_dist.0),
        "foodLevel" => NbtValue::Int(food.food_level),
        "foodSaturationLevel" => NbtValue::Float(food.saturation),
        "foodExhaustionLevel" => NbtValue::Float(food.exhaustion),
        "Inventory" => NbtValue::List(inv_list),
        "SelectedItemSlot" => NbtValue::Int(held.0 as i32),
        "playerGameType" => NbtValue::Int(game_type),
        "Dimension" => NbtValue::String("minecraft:overworld".into())
    };

    let mut buf = BytesMut::new();
    nbt.write_root_named("", &mut buf);

    // Gzip compress
    let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&buf).ok()?;
    Some(encoder.finish().ok()?)
}

/// Load player data from gzip-compressed NBT bytes. Returns position, rotation, health, food, inventory, game mode.
fn deserialize_player_data(data: &[u8]) -> Option<PlayerSaveData> {
    let mut decoder = GzDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed).ok()?;

    let (_, nbt) = NbtValue::read_root_named(&decompressed).ok()?;

    let pos_list = nbt.get("Pos")?.as_list()?;
    let x = pos_list.get(0)?.as_double()?;
    let y = pos_list.get(1)?.as_double()?;
    let z = pos_list.get(2)?.as_double()?;

    let rot_list = nbt.get("Rotation")?.as_list()?;
    let yaw = rot_list.get(0)?.as_float()?;
    let pitch = rot_list.get(1)?.as_float()?;

    let health = nbt.get("Health")?.as_float()?;
    let food_level = nbt.get("foodLevel")?.as_int()?;
    let saturation = nbt.get("foodSaturationLevel")?.as_float()?;
    let exhaustion = nbt.get("foodExhaustionLevel").and_then(|v| v.as_float()).unwrap_or(0.0);
    let fall_distance = nbt.get("FallDistance").and_then(|v| v.as_float()).unwrap_or(0.0);
    let held_slot = nbt.get("SelectedItemSlot").and_then(|v| v.as_int()).unwrap_or(0) as u8;
    let game_type = nbt.get("playerGameType").and_then(|v| v.as_int()).unwrap_or(0);

    let game_mode = match game_type {
        0 => GameMode::Survival,
        1 => GameMode::Creative,
        2 => GameMode::Adventure,
        3 => GameMode::Spectator,
        _ => GameMode::Survival,
    };

    // Parse inventory
    let mut slots: [Option<ItemStack>; 46] = std::array::from_fn(|_| None);
    if let Some(inv_list) = nbt.get("Inventory").and_then(|v| v.as_list()) {
        for item_nbt in inv_list {
            let nbt_slot = item_nbt.get("Slot")?.as_byte()?;
            let id_str = item_nbt.get("id")?.as_str()?;
            let count = item_nbt.get("count")?.as_byte()?;

            let short_name = id_str.strip_prefix("minecraft:").unwrap_or(id_str);
            let item_id = pickaxe_data::item_name_to_id(short_name).unwrap_or(0);

            // Map NBT slot to ECS slot
            let ecs_slot = match nbt_slot {
                0..=8 => Some(36 + nbt_slot as usize),   // hotbar
                9..=35 => Some(nbt_slot as usize),        // main inventory
                100..=103 => Some(5 + (nbt_slot - 100) as usize), // armor
                -106 => Some(45),                          // offhand
                _ => None,
            };

            if let Some(slot_idx) = ecs_slot {
                if slot_idx < 46 {
                    slots[slot_idx] = Some(ItemStack { item_id, count });
                }
            }
        }
    }

    Some(PlayerSaveData {
        position: Vec3d::new(x, y, z),
        yaw, pitch, health, food_level, saturation, exhaustion,
        fall_distance, held_slot, game_mode, slots,
    })
}

struct PlayerSaveData {
    position: Vec3d,
    yaw: f32,
    pitch: f32,
    health: f32,
    food_level: i32,
    saturation: f32,
    exhaustion: f32,
    fall_distance: f32,
    held_slot: u8,
    game_mode: GameMode,
    slots: [Option<ItemStack>; 46],
}
```

**Step 2: Hook into handle_disconnect**

In `handle_disconnect()`, before `world.despawn(entity)`, serialize and save:

```rust
// Save player data before despawning
if let Some(data) = serialize_player_data(world, entity) {
    if let Some(uuid) = player_uuid {
        let _ = world_state.save_tx.send(SaveOp::Player(uuid, data));
    }
}
```

This requires `world_state` to be passed to `handle_disconnect()` (it already is based on the explored code).

**Step 3: Hook into handle_new_player**

In `handle_new_player()`, try loading player data from disk before spawning:

```rust
// Try to load saved player data
let player_data_path = PathBuf::from(&config.world_dir)
    .join("playerdata")
    .join(format!("{}.dat", profile.uuid));
let saved = if player_data_path.exists() {
    std::fs::read(&player_data_path).ok()
        .and_then(|data| deserialize_player_data(&data))
} else {
    None
};

let (spawn_pos, yaw, pitch) = if let Some(ref s) = saved {
    (s.position, s.yaw, s.pitch)
} else {
    (Vec3d::new(0.5, -59.0, 0.5), 0.0, 0.0)
};
```

Then use `saved` when creating the ECS entity to restore health, food, inventory, game mode, etc. instead of defaults.

**Step 4: Add periodic player save (every 1200 ticks = 60 seconds)**

In the tick loop, after `tick_block_breaking`:

```rust
// Auto-save player data every 60 seconds
if tick_count % 1200 == 0 && tick_count > 0 {
    save_all_players(&world, &world_state.save_tx);
}
```

```rust
fn save_all_players(world: &World, save_tx: &mpsc::UnboundedSender<SaveOp>) {
    for (entity, profile) in world.query::<&Profile>().iter() {
        if let Some(data) = serialize_player_data(world, entity) {
            let _ = save_tx.send(SaveOp::Player(profile.0.uuid, data));
        }
    }
}
```

**Step 5: Build and verify**

Run: `cargo build`
Expected: Compiles

**Step 6: Commit**

```bash
git add crates/pickaxe-server/
git commit -m "feat(m10): add player data save/load with inventory persistence"
```

---

### Task 6: Level.dat Persistence

**Files:**
- Modify: `crates/pickaxe-server/src/tick.rs` — load/save level.dat

**Step 1: Implement level.dat serialization/deserialization**

```rust
fn serialize_level_dat(world_state: &WorldState, config: &ServerConfig) -> Vec<u8> {
    let nbt = nbt_compound! {
        "DataVersion" => NbtValue::Int(3955),
        "Data" => nbt_compound! {
            "LevelName" => NbtValue::String("Pickaxe World".into()),
            "SpawnX" => NbtValue::Int(0),
            "SpawnY" => NbtValue::Int(-59),
            "SpawnZ" => NbtValue::Int(0),
            "Time" => NbtValue::Long(world_state.world_age),
            "DayTime" => NbtValue::Long(world_state.time_of_day),
            "GameType" => NbtValue::Int(0),
            "Difficulty" => NbtValue::Byte(2),
            "hardcore" => NbtValue::Byte(0),
            "allowCommands" => NbtValue::Byte(1),
            "Version" => nbt_compound! {
                "Name" => NbtValue::String("1.21.1".into()),
                "Id" => NbtValue::Int(767)
            }
        }
    };

    let mut buf = BytesMut::new();
    nbt.write_root_named("", &mut buf);

    let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(&buf);
    encoder.finish().unwrap_or_default()
}

fn load_level_dat(path: &Path) -> Option<(i64, i64)> {
    let data = std::fs::read(path).ok()?;
    let mut decoder = GzDecoder::new(&data[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed).ok()?;

    let (_, nbt) = NbtValue::read_root_named(&decompressed).ok()?;
    let data_nbt = nbt.get("Data")?;
    let world_age = data_nbt.get("Time")?.as_long()?;
    let time_of_day = data_nbt.get("DayTime")?.as_long()?;
    Some((world_age, time_of_day))
}
```

**Step 2: Load on startup**

In `run_tick_loop()`, after creating `WorldState`:

```rust
let level_dat_path = PathBuf::from(&config.world_dir).join("level.dat");
if let Some((world_age, time_of_day)) = load_level_dat(&level_dat_path) {
    world_state.world_age = world_age;
    world_state.time_of_day = time_of_day;
    info!("Loaded level.dat: world_age={}, time_of_day={}", world_age, time_of_day);
}
```

**Step 3: Save on shutdown and periodically**

Save level.dat every 1200 ticks alongside player saves:

```rust
if tick_count % 1200 == 0 && tick_count > 0 {
    save_all_players(&world, &world_state.save_tx);
    let level_data = serialize_level_dat(&world_state, &config);
    let _ = world_state.save_tx.send(SaveOp::LevelDat(level_data));
}
```

**Step 4: Build and verify**

Run: `cargo build`
Expected: Compiles

**Step 5: Commit**

```bash
git add crates/pickaxe-server/
git commit -m "feat(m10): add level.dat load/save for world time persistence"
```

---

### Task 7: Graceful Shutdown

**Files:**
- Modify: `crates/pickaxe-server/src/main.rs` — add SIGINT/SIGTERM handler
- Modify: `crates/pickaxe-server/src/tick.rs` — save all on shutdown signal

**Step 1: Add shutdown channel**

In `main.rs`, create a shutdown signal:

```rust
let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

// Handle Ctrl+C
let ctrlc_tx = shutdown_tx.clone();
tokio::spawn(async move {
    let _ = tokio::signal::ctrl_c().await;
    info!("Received shutdown signal");
    let _ = ctrlc_tx.send(true);
});
```

Pass `shutdown_rx` to `run_tick_loop()`.

**Step 2: Check shutdown in tick loop**

In the main loop of `run_tick_loop()`, at the start of each tick:

```rust
if *shutdown_rx.borrow() {
    info!("Shutting down...");
    // Save all players
    save_all_players(&world, &world_state.save_tx);
    // Save level.dat
    let level_data = serialize_level_dat(&world_state, &config);
    let _ = world_state.save_tx.send(SaveOp::LevelDat(level_data));
    // Signal saver to flush and stop
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let _ = world_state.save_tx.send(SaveOp::Shutdown(done_tx));
    let _ = done_rx.await;
    info!("World saved. Goodbye!");
    return;
}
```

**Step 3: Build and test manually**

Run: `cargo run`
- Start server, let it run for a few seconds
- Press Ctrl+C
- Expected: "Shutting down...", "World saved. Goodbye!" in logs
- Verify `world/` directory exists with `region/`, `playerdata/`, `level.dat`

**Step 4: Commit**

```bash
git add crates/pickaxe-server/
git commit -m "feat(m10): add graceful shutdown with world save"
```

---

### Task 8: Integration Testing and Verification

**Step 1: Full build and test**

```bash
cargo build && cargo test
```

Expected: All tests pass, no compile errors.

**Step 2: Manual test plan**

1. `cargo run` — start server
2. Join with MC 1.21.1 client
3. Break a few blocks, place some blocks
4. Ctrl+C to stop server
5. Check `world/region/r.0.0.mca` exists (and has nonzero size)
6. Check `world/level.dat` exists
7. Check `world/playerdata/` has a `.dat` file
8. `cargo run` — restart server
9. Join again — verify:
   - Broken blocks are still broken
   - Placed blocks are still placed
   - Player position is restored
   - Inventory is restored
   - Health/food are restored
   - World time continues from where it left off

**Step 3: Final commit**

```bash
git add -A
git commit -m "feat(m10): complete world persistence — chunks, players, level.dat"
```

---

## File Summary

| File | Action | Purpose |
|------|--------|---------|
| `crates/pickaxe-nbt/src/nbt.rs` | Modify | Add NBT read/parse + accessor methods |
| `crates/pickaxe-region/Cargo.toml` | Create | New crate for Anvil I/O |
| `crates/pickaxe-region/src/lib.rs` | Create | Module exports |
| `crates/pickaxe-region/src/region_file.rs` | Create | RegionFile + RegionStorage |
| `crates/pickaxe-world/src/chunk.rs` | Modify | Add `to_nbt()` / `from_nbt()` |
| `crates/pickaxe-server/src/config.rs` | Modify | Add `world_dir` field |
| `crates/pickaxe-server/src/tick.rs` | Modify | WorldState integration, save/load, saver task |
| `crates/pickaxe-server/src/main.rs` | Modify | Spawn saver, shutdown handler |
| `crates/pickaxe-server/Cargo.toml` | Modify | Add `pickaxe-region` dep |
| `Cargo.toml` | Modify | Add workspace member + dep |

## Commit Order

1. `feat(m10): add NBT read/parse support with accessor methods`
2. `feat(m10): add pickaxe-region crate with Anvil .mca file I/O`
3. `feat(m10): add Chunk to_nbt/from_nbt for Anvil serialization`
4. `feat(m10): wire up write-through chunk saving with background saver`
5. `feat(m10): add player data save/load with inventory persistence`
6. `feat(m10): add level.dat load/save for world time persistence`
7. `feat(m10): add graceful shutdown with world save`
8. `feat(m10): complete world persistence — chunks, players, level.dat`
