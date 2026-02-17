use bytes::{Buf, BufMut, BytesMut};
use pickaxe_types::ItemStack;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("VarInt too big")]
    VarIntTooBig,
    #[error("Not enough data")]
    NotEnoughData,
    #[error("String too long: {0} > {1}")]
    StringTooLong(usize, usize),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type CodecResult<T> = Result<T, CodecError>;

/// Read a VarInt from the buffer.
pub fn read_varint(buf: &mut BytesMut) -> CodecResult<i32> {
    let mut result: i32 = 0;
    let mut shift: u32 = 0;
    loop {
        if !buf.has_remaining() {
            return Err(CodecError::NotEnoughData);
        }
        let byte = buf.get_u8();
        result |= ((byte & 0x7F) as i32) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 32 {
            return Err(CodecError::VarIntTooBig);
        }
    }
}

/// Write a VarInt to the buffer.
pub fn write_varint(buf: &mut BytesMut, mut value: i32) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value = ((value as u32) >> 7) as i32;
        if value != 0 {
            byte |= 0x80;
        }
        buf.put_u8(byte);
        if value == 0 {
            break;
        }
    }
}

/// Calculate the byte length of a VarInt.
pub fn varint_len(value: i32) -> usize {
    let mut val = value as u32;
    let mut len = 0;
    loop {
        len += 1;
        val >>= 7;
        if val == 0 {
            break;
        }
    }
    len
}

/// Write a VarInt to a Vec<u8>.
pub fn write_varint_vec(buf: &mut Vec<u8>, mut value: i32) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value = ((value as u32) >> 7) as i32;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Read a VarLong from the buffer.
pub fn read_varlong(buf: &mut BytesMut) -> CodecResult<i64> {
    let mut result: i64 = 0;
    let mut shift: u32 = 0;
    loop {
        if !buf.has_remaining() {
            return Err(CodecError::NotEnoughData);
        }
        let byte = buf.get_u8();
        result |= ((byte & 0x7F) as i64) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err(CodecError::VarIntTooBig);
        }
    }
}

/// Read a protocol string (varint-prefixed UTF-8).
pub fn read_string(buf: &mut BytesMut, max_len: usize) -> CodecResult<String> {
    let len = read_varint(buf)? as usize;
    if len > max_len * 4 {
        return Err(CodecError::StringTooLong(len, max_len));
    }
    if buf.remaining() < len {
        return Err(CodecError::NotEnoughData);
    }
    let bytes = buf.split_to(len);
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Write a protocol string.
pub fn write_string(buf: &mut BytesMut, s: &str) {
    write_varint(buf, s.len() as i32);
    buf.put_slice(s.as_bytes());
}

/// Safe fixed-size reads that return CodecError::NotEnoughData instead of panicking.

pub fn read_u8(buf: &mut BytesMut) -> CodecResult<u8> {
    if buf.remaining() < 1 { return Err(CodecError::NotEnoughData); }
    Ok(buf.get_u8())
}

pub fn read_i8(buf: &mut BytesMut) -> CodecResult<i8> {
    if buf.remaining() < 1 { return Err(CodecError::NotEnoughData); }
    Ok(buf.get_i8())
}

pub fn read_u16(buf: &mut BytesMut) -> CodecResult<u16> {
    if buf.remaining() < 2 { return Err(CodecError::NotEnoughData); }
    Ok(buf.get_u16())
}

pub fn read_i16(buf: &mut BytesMut) -> CodecResult<i16> {
    if buf.remaining() < 2 { return Err(CodecError::NotEnoughData); }
    Ok(buf.get_i16())
}

pub fn read_i32_raw(buf: &mut BytesMut) -> CodecResult<i32> {
    if buf.remaining() < 4 { return Err(CodecError::NotEnoughData); }
    Ok(buf.get_i32())
}

pub fn read_f32(buf: &mut BytesMut) -> CodecResult<f32> {
    if buf.remaining() < 4 { return Err(CodecError::NotEnoughData); }
    Ok(buf.get_f32())
}

pub fn read_i64(buf: &mut BytesMut) -> CodecResult<i64> {
    if buf.remaining() < 8 { return Err(CodecError::NotEnoughData); }
    Ok(buf.get_i64())
}

pub fn read_u64(buf: &mut BytesMut) -> CodecResult<u64> {
    if buf.remaining() < 8 { return Err(CodecError::NotEnoughData); }
    Ok(buf.get_u64())
}

pub fn read_f64(buf: &mut BytesMut) -> CodecResult<f64> {
    if buf.remaining() < 8 { return Err(CodecError::NotEnoughData); }
    Ok(buf.get_f64())
}

pub fn read_bytes(buf: &mut BytesMut, count: usize) -> CodecResult<Vec<u8>> {
    if buf.remaining() < count { return Err(CodecError::NotEnoughData); }
    let mut out = vec![0u8; count];
    buf.copy_to_slice(&mut out);
    Ok(out)
}

/// Read a UUID (128 bits, big endian).
pub fn read_uuid(buf: &mut BytesMut) -> CodecResult<Uuid> {
    if buf.remaining() < 16 {
        return Err(CodecError::NotEnoughData);
    }
    let mut bytes = [0u8; 16];
    buf.copy_to_slice(&mut bytes);
    Ok(Uuid::from_bytes(bytes))
}

/// Write a UUID.
pub fn write_uuid(buf: &mut BytesMut, uuid: &Uuid) {
    buf.put_slice(uuid.as_bytes());
}

/// Read a byte array with varint length prefix.
pub fn read_byte_array(buf: &mut BytesMut) -> CodecResult<Vec<u8>> {
    let len = read_varint(buf)? as usize;
    if buf.remaining() < len {
        return Err(CodecError::NotEnoughData);
    }
    let bytes = buf.split_to(len);
    Ok(bytes.to_vec())
}

/// Write a byte array with varint length prefix.
pub fn write_byte_array(buf: &mut BytesMut, data: &[u8]) {
    write_varint(buf, data.len() as i32);
    buf.put_slice(data);
}

/// Read a Slot from the wire (1.21.1 component-based format).
/// Returns None for empty slots (item_count == 0).
pub fn read_slot(buf: &mut BytesMut) -> CodecResult<Option<ItemStack>> {
    let item_count = read_varint(buf)?;
    if item_count <= 0 {
        return Ok(None);
    }
    let item_id = read_varint(buf)?;
    let add_count = read_varint(buf)?;
    let remove_count = read_varint(buf)?;
    // Skip component data — we don't handle components yet.
    // For basic items (no enchantments/custom data), counts are 0.
    // When components are present, we must consume all remaining data for this slot
    // to avoid corrupting subsequent reads. Since component data has variable-length
    // encoding and we can't parse it, we consume all remaining bytes in the packet.
    if add_count > 0 || remove_count > 0 {
        tracing::debug!("Slot has {} added, {} removed components — skipping {} remaining bytes", add_count, remove_count, buf.remaining());
        buf.advance(buf.remaining());
    }
    Ok(Some(ItemStack::new(item_id, item_count as i8)))
}

/// Write a Slot to the wire (1.21.1 component-based format).
pub fn write_slot(buf: &mut BytesMut, slot: &Option<ItemStack>) {
    match slot {
        None => {
            write_varint(buf, 0); // item_count = 0 = empty
        }
        Some(item) => {
            write_varint(buf, item.count as i32);
            write_varint(buf, item.item_id);
            write_varint(buf, 0); // no added components
            write_varint(buf, 0); // no removed components
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_varint_roundtrip() {
        let test_cases = vec![
            (0, vec![0x00]),
            (1, vec![0x01]),
            (127, vec![0x7F]),
            (128, vec![0x80, 0x01]),
            (255, vec![0xFF, 0x01]),
            (25565, vec![0xDD, 0xC7, 0x01]),
            (2097151, vec![0xFF, 0xFF, 0x7F]),
            (-1, vec![0xFF, 0xFF, 0xFF, 0xFF, 0x0F]),
        ];

        for (value, expected_bytes) in test_cases {
            // Test write
            let mut buf = BytesMut::new();
            write_varint(&mut buf, value);
            assert_eq!(
                buf.to_vec(),
                expected_bytes,
                "write_varint({}) failed",
                value
            );

            // Test read
            let mut buf = BytesMut::from(&expected_bytes[..]);
            let result = read_varint(&mut buf).unwrap();
            assert_eq!(result, value, "read_varint for {} failed", value);
        }
    }

    #[test]
    fn test_varint_len() {
        assert_eq!(varint_len(0), 1);
        assert_eq!(varint_len(127), 1);
        assert_eq!(varint_len(128), 2);
        assert_eq!(varint_len(25565), 3);
        assert_eq!(varint_len(-1), 5);
    }

    #[test]
    fn test_string_roundtrip() {
        let test_str = "Hello, Minecraft!";
        let mut buf = BytesMut::new();
        write_string(&mut buf, test_str);
        let result = read_string(&mut buf, 32767).unwrap();
        assert_eq!(result, test_str);
    }

    #[test]
    fn test_uuid_roundtrip() {
        let uuid = Uuid::new_v4();
        let mut buf = BytesMut::new();
        write_uuid(&mut buf, &uuid);
        let result = read_uuid(&mut buf).unwrap();
        assert_eq!(result, uuid);
    }
}
