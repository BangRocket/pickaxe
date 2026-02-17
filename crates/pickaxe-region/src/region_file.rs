use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const SECTOR_BYTES: usize = 4096;
const HEADER_SECTORS: usize = 2;
const COMPRESSION_DEFLATE: u8 = 2;

/// A single .mca region file handle.
pub struct RegionFile {
    file: File,
    locations: [u32; 1024],
    timestamps: [u32; 1024],
    used_sectors: Vec<bool>,
}

impl RegionFile {
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
            let zeros = [0u8; SECTOR_BYTES];
            file.write_all(&zeros)?;
            file.write_all(&zeros)?;
            file.flush()?;
        }

        let file_len = file.metadata()?.len() as usize;
        let total_sectors = (file_len + SECTOR_BYTES - 1) / SECTOR_BYTES;
        let total_sectors = total_sectors.max(HEADER_SECTORS);
        let mut used_sectors = vec![false; total_sectors];
        used_sectors[0] = true;
        used_sectors[1] = true;

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

        Ok(Self {
            file,
            locations,
            timestamps,
            used_sectors,
        })
    }

    pub fn read_chunk(&mut self, local_x: usize, local_z: usize) -> io::Result<Option<Vec<u8>>> {
        let index = local_x + local_z * 32;
        let loc = self.locations[index];
        if loc == 0 {
            return Ok(None);
        }

        let sector = (loc >> 8) as u64;
        let _count = (loc & 0xFF) as usize;

        self.file
            .seek(SeekFrom::Start(sector * SECTOR_BYTES as u64))?;

        let mut header = [0u8; 5];
        self.file.read_exact(&mut header)?;
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let compression = header[4];

        if length <= 1 {
            return Ok(None);
        }

        let data_len = length - 1;
        let mut compressed = vec![0u8; data_len];
        self.file.read_exact(&mut compressed)?;

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

    pub fn write_chunk(
        &mut self,
        local_x: usize,
        local_z: usize,
        nbt_bytes: &[u8],
    ) -> io::Result<()> {
        let index = local_x + local_z * 32;

        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(nbt_bytes)?;
        let compressed = encoder.finish()?;

        let total = 5 + compressed.len();
        let sectors_needed = (total + SECTOR_BYTES - 1) / SECTOR_BYTES;

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

        let new_sector = self.allocate_sectors(sectors_needed);

        self.file
            .seek(SeekFrom::Start(new_sector as u64 * SECTOR_BYTES as u64))?;
        let length = (compressed.len() + 1) as u32;
        self.file.write_all(&length.to_be_bytes())?;
        self.file.write_all(&[COMPRESSION_DEFLATE])?;
        self.file.write_all(&compressed)?;

        let written = 5 + compressed.len();
        let padding = sectors_needed * SECTOR_BYTES - written;
        if padding > 0 {
            self.file.write_all(&vec![0u8; padding])?;
        }

        self.locations[index] = ((new_sector as u32) << 8) | (sectors_needed as u32 & 0xFF);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        self.timestamps[index] = now;

        self.write_header()?;
        self.file.flush()?;

        Ok(())
    }

    fn allocate_sectors(&mut self, count: usize) -> usize {
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
        Ok(Self {
            dir,
            cache: HashMap::new(),
        })
    }

    pub fn read_chunk(&mut self, chunk_x: i32, chunk_z: i32) -> io::Result<Option<Vec<u8>>> {
        let (region_x, region_z, local_x, local_z) = Self::chunk_to_region(chunk_x, chunk_z);
        let region = self.get_or_open(region_x, region_z)?;
        region.read_chunk(local_x, local_z)
    }

    pub fn write_chunk(
        &mut self,
        chunk_x: i32,
        chunk_z: i32,
        nbt_bytes: &[u8],
    ) -> io::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        storage
            .write_chunk(3, 7, b"second version, which is longer")
            .unwrap();
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
        assert_eq!(
            storage.read_chunk(0, 0).unwrap(),
            Some(b"chunk_0_0".to_vec())
        );
        assert_eq!(
            storage.read_chunk(1, 0).unwrap(),
            Some(b"chunk_1_0".to_vec())
        );
        assert_eq!(
            storage.read_chunk(0, 1).unwrap(),
            Some(b"chunk_0_1".to_vec())
        );
        assert_eq!(storage.read_chunk(2, 2).unwrap(), None);
    }

    #[test]
    fn test_different_regions() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = RegionStorage::new(dir.path().join("region")).unwrap();
        storage.write_chunk(0, 0, b"region_0_0").unwrap();
        storage.write_chunk(32, 0, b"region_1_0").unwrap();
        assert_eq!(
            storage.read_chunk(0, 0).unwrap(),
            Some(b"region_0_0".to_vec())
        );
        assert_eq!(
            storage.read_chunk(32, 0).unwrap(),
            Some(b"region_1_0".to_vec())
        );
        let region_dir = dir.path().join("region");
        let files: Vec<_> = fs::read_dir(&region_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_reopen_region() {
        let dir = tempfile::tempdir().unwrap();
        let region_dir = dir.path().join("region");
        {
            let mut storage = RegionStorage::new(region_dir.clone()).unwrap();
            storage.write_chunk(5, 10, b"persistent data").unwrap();
        }
        {
            let mut storage = RegionStorage::new(region_dir).unwrap();
            let result = storage.read_chunk(5, 10).unwrap();
            assert_eq!(result, Some(b"persistent data".to_vec()));
        }
    }
}
