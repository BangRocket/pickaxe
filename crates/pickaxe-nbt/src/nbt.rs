use bytes::{BufMut, BytesMut};
use std::io::{self, Cursor, Read};

/// NBT tag type IDs.
pub const TAG_END: u8 = 0;
pub const TAG_BYTE: u8 = 1;
pub const TAG_SHORT: u8 = 2;
pub const TAG_INT: u8 = 3;
pub const TAG_LONG: u8 = 4;
pub const TAG_FLOAT: u8 = 5;
pub const TAG_DOUBLE: u8 = 6;
pub const TAG_BYTE_ARRAY: u8 = 7;
pub const TAG_STRING: u8 = 8;
pub const TAG_LIST: u8 = 9;
pub const TAG_COMPOUND: u8 = 10;
pub const TAG_INT_ARRAY: u8 = 11;
pub const TAG_LONG_ARRAY: u8 = 12;

/// An NBT value.
#[derive(Debug, Clone, PartialEq)]
pub enum NbtValue {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<NbtValue>),
    Compound(Vec<(String, NbtValue)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl NbtValue {
    pub fn tag_id(&self) -> u8 {
        match self {
            NbtValue::Byte(_) => TAG_BYTE,
            NbtValue::Short(_) => TAG_SHORT,
            NbtValue::Int(_) => TAG_INT,
            NbtValue::Long(_) => TAG_LONG,
            NbtValue::Float(_) => TAG_FLOAT,
            NbtValue::Double(_) => TAG_DOUBLE,
            NbtValue::ByteArray(_) => TAG_BYTE_ARRAY,
            NbtValue::String(_) => TAG_STRING,
            NbtValue::List(_) => TAG_LIST,
            NbtValue::Compound(_) => TAG_COMPOUND,
            NbtValue::IntArray(_) => TAG_INT_ARRAY,
            NbtValue::LongArray(_) => TAG_LONG_ARRAY,
        }
    }

    /// Write this value as a root compound tag (with empty name) for network protocol.
    pub fn write_root_network(&self, buf: &mut BytesMut) {
        // Network NBT in 1.20.2+: root compound tag with type byte, but NO name
        buf.put_u8(self.tag_id());
        self.write_payload(buf);
    }

    /// Write this value as a full named root tag (for files).
    pub fn write_root_named(&self, name: &str, buf: &mut BytesMut) {
        buf.put_u8(self.tag_id());
        write_nbt_string(name, buf);
        self.write_payload(buf);
    }

    /// Write just the payload (no tag type or name).
    pub fn write_payload(&self, buf: &mut BytesMut) {
        match self {
            NbtValue::Byte(v) => buf.put_i8(*v),
            NbtValue::Short(v) => buf.put_i16(*v),
            NbtValue::Int(v) => buf.put_i32(*v),
            NbtValue::Long(v) => buf.put_i64(*v),
            NbtValue::Float(v) => buf.put_f32(*v),
            NbtValue::Double(v) => buf.put_f64(*v),
            NbtValue::ByteArray(v) => {
                buf.put_i32(v.len() as i32);
                for b in v {
                    buf.put_i8(*b);
                }
            }
            NbtValue::String(v) => {
                write_nbt_string(v, buf);
            }
            NbtValue::List(v) => {
                if v.is_empty() {
                    buf.put_u8(TAG_END);
                    buf.put_i32(0);
                } else {
                    buf.put_u8(v[0].tag_id());
                    buf.put_i32(v.len() as i32);
                    for item in v {
                        item.write_payload(buf);
                    }
                }
            }
            NbtValue::Compound(entries) => {
                for (name, value) in entries {
                    buf.put_u8(value.tag_id());
                    write_nbt_string(name, buf);
                    value.write_payload(buf);
                }
                buf.put_u8(TAG_END);
            }
            NbtValue::IntArray(v) => {
                buf.put_i32(v.len() as i32);
                for i in v {
                    buf.put_i32(*i);
                }
            }
            NbtValue::LongArray(v) => {
                buf.put_i32(v.len() as i32);
                for l in v {
                    buf.put_i64(*l);
                }
            }
        }
    }

    /// Read a named root tag from bytes. Returns (name, value).
    pub fn read_root_named(data: &[u8]) -> io::Result<(String, NbtValue)> {
        let mut cursor = Cursor::new(data);
        let tag_type = read_u8(&mut cursor)?;
        if tag_type != TAG_COMPOUND {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Root must be compound",
            ));
        }
        let name = read_nbt_string_r(&mut cursor)?;
        let value = read_payload(&mut cursor, TAG_COMPOUND)?;
        Ok((name, value))
    }

    /// Read an unnamed root tag (network format).
    pub fn read_root_network(data: &[u8]) -> io::Result<NbtValue> {
        let mut cursor = Cursor::new(data);
        let tag_type = read_u8(&mut cursor)?;
        if tag_type != TAG_COMPOUND {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Root must be compound",
            ));
        }
        read_payload(&mut cursor, TAG_COMPOUND)
    }

    /// Get a named field from a compound tag.
    pub fn get(&self, key: &str) -> Option<&NbtValue> {
        match self {
            NbtValue::Compound(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Get as i8.
    pub fn as_byte(&self) -> Option<i8> {
        match self {
            NbtValue::Byte(v) => Some(*v),
            _ => None,
        }
    }

    /// Get as i16.
    pub fn as_short(&self) -> Option<i16> {
        match self {
            NbtValue::Short(v) => Some(*v),
            _ => None,
        }
    }

    /// Get as i32.
    pub fn as_int(&self) -> Option<i32> {
        match self {
            NbtValue::Int(v) => Some(*v),
            _ => None,
        }
    }

    /// Get as i64.
    pub fn as_long(&self) -> Option<i64> {
        match self {
            NbtValue::Long(v) => Some(*v),
            _ => None,
        }
    }

    /// Get as f32.
    pub fn as_float(&self) -> Option<f32> {
        match self {
            NbtValue::Float(v) => Some(*v),
            _ => None,
        }
    }

    /// Get as f64.
    pub fn as_double(&self) -> Option<f64> {
        match self {
            NbtValue::Double(v) => Some(*v),
            _ => None,
        }
    }

    /// Get as string slice.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            NbtValue::String(v) => Some(v),
            _ => None,
        }
    }

    /// Get as list slice.
    pub fn as_list(&self) -> Option<&[NbtValue]> {
        match self {
            NbtValue::List(v) => Some(v),
            _ => None,
        }
    }

    /// Get as byte array slice.
    pub fn as_byte_array(&self) -> Option<&[i8]> {
        match self {
            NbtValue::ByteArray(v) => Some(v),
            _ => None,
        }
    }

    /// Get as int array slice.
    pub fn as_int_array(&self) -> Option<&[i32]> {
        match self {
            NbtValue::IntArray(v) => Some(v),
            _ => None,
        }
    }

    /// Get as long array slice.
    pub fn as_long_array(&self) -> Option<&[i64]> {
        match self {
            NbtValue::LongArray(v) => Some(v),
            _ => None,
        }
    }
}

fn write_nbt_string(s: &str, buf: &mut BytesMut) {
    let bytes = s.as_bytes();
    buf.put_u16(bytes.len() as u16);
    buf.put_slice(bytes);
}

// --- NBT Reader helpers ---

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

fn read_nbt_string_r(r: &mut impl Read) -> io::Result<String> {
    let len = read_u16(r)? as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn read_length(r: &mut impl Read) -> io::Result<usize> {
    let raw = read_i32(r)?;
    if raw < 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Negative length"));
    }
    Ok(raw as usize)
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
            let len = read_length(r)?;
            let mut data = vec![0i8; len];
            for v in &mut data {
                *v = read_i8(r)?;
            }
            Ok(NbtValue::ByteArray(data))
        }
        TAG_STRING => Ok(NbtValue::String(read_nbt_string_r(r)?)),
        TAG_LIST => {
            let elem_type = read_u8(r)?;
            let len = read_length(r)?;
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
                if child_type == TAG_END {
                    break;
                }
                let name = read_nbt_string_r(r)?;
                let value = read_payload(r, child_type)?;
                entries.push((name, value));
            }
            Ok(NbtValue::Compound(entries))
        }
        TAG_INT_ARRAY => {
            let len = read_length(r)?;
            let mut data = Vec::with_capacity(len);
            for _ in 0..len {
                data.push(read_i32(r)?);
            }
            Ok(NbtValue::IntArray(data))
        }
        TAG_LONG_ARRAY => {
            let len = read_length(r)?;
            let mut data = Vec::with_capacity(len);
            for _ in 0..len {
                data.push(read_i64(r)?);
            }
            Ok(NbtValue::LongArray(data))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Unknown tag type {}", tag_type),
        )),
    }
}

/// Helper macro for building compound tags.
#[macro_export]
macro_rules! nbt_compound {
    ($($key:expr => $val:expr),* $(,)?) => {
        $crate::NbtValue::Compound(vec![
            $(($key.into(), $val)),*
        ])
    };
}

/// Helper macro for building list tags.
#[macro_export]
macro_rules! nbt_list {
    ($($val:expr),* $(,)?) => {
        $crate::NbtValue::List(vec![$($val),*])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_simple_compound() {
        let nbt = NbtValue::Compound(vec![
            ("name".into(), NbtValue::String("test".into())),
            ("value".into(), NbtValue::Int(42)),
        ]);
        let mut buf = BytesMut::new();
        nbt.write_root_network(&mut buf);
        // Should start with TAG_COMPOUND (10)
        assert_eq!(buf[0], TAG_COMPOUND);
    }

    #[test]
    fn test_long_array() {
        let nbt = NbtValue::LongArray(vec![1, 2, 3]);
        let mut buf = BytesMut::new();
        nbt.write_payload(&mut buf);
        // 4 bytes length (3) + 3 * 8 bytes = 28 bytes
        assert_eq!(buf.len(), 28);
    }

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
            (
                "pos".into(),
                NbtValue::List(vec![
                    NbtValue::Double(1.0),
                    NbtValue::Double(2.0),
                    NbtValue::Double(3.0),
                ]),
            ),
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
        let nbt = NbtValue::Compound(vec![("empty".into(), NbtValue::List(vec![]))]);
        let mut buf = BytesMut::new();
        nbt.write_root_named("", &mut buf);
        let (_, parsed) = NbtValue::read_root_named(&buf).unwrap();
        assert_eq!(parsed, nbt);
    }
}
