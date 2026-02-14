use bytes::{BufMut, BytesMut};

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
}

fn write_nbt_string(s: &str, buf: &mut BytesMut) {
    let bytes = s.as_bytes();
    buf.put_u16(bytes.len() as u16);
    buf.put_slice(bytes);
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
}
