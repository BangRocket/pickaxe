use crate::codec::{read_varint, varint_len, write_varint};
use aes::cipher::{BlockEncrypt, KeyInit};
use aes::Aes128;
use bytes::{Buf, BytesMut};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{Read as _, Write as _};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tracing::trace;

/// Manual AES-128-CFB8 cipher that supports streaming (byte-at-a-time).
/// MC protocol requires maintaining cipher state across multiple encrypt/decrypt calls.
struct Cfb8Cipher {
    cipher: Aes128,
    iv: [u8; 16],
}

impl Cfb8Cipher {
    fn new(key: &[u8; 16], iv: &[u8; 16]) -> Self {
        let cipher = Aes128::new(key.into());
        Self { cipher, iv: *iv }
    }

    fn encrypt(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            let mut block = aes::Block::from(self.iv);
            self.cipher.encrypt_block(&mut block);
            *byte ^= block[0];
            // Shift IV left by 1, append ciphertext byte
            self.iv.copy_within(1.., 0);
            self.iv[15] = *byte;
        }
    }

    fn decrypt(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            let mut block = aes::Block::from(self.iv);
            self.cipher.encrypt_block(&mut block);
            let ciphertext = *byte;
            *byte ^= block[0];
            // Shift IV left by 1, append original ciphertext byte
            self.iv.copy_within(1.., 0);
            self.iv[15] = ciphertext;
        }
    }
}

/// A framed Minecraft protocol connection with optional compression and encryption.
pub struct Connection {
    stream: Option<TcpStream>,
    read_buf: BytesMut,
    compression_threshold: Option<i32>,
    encryptor: Option<Cfb8Cipher>,
    decryptor: Option<Cfb8Cipher>,
}

impl Connection {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream: Some(stream),
            read_buf: BytesMut::with_capacity(4096),
            compression_threshold: None,
            encryptor: None,
            decryptor: None,
        }
    }

    /// Create a dummy connection (used as a placeholder after `into_split`).
    pub fn new_dummy() -> Self {
        // Create a dummy TCP stream by binding to a temporary address
        // This is only used as a placeholder and never actually read/written
        Self {
            stream: None,
            read_buf: BytesMut::new(),
            compression_threshold: None,
            encryptor: None,
            decryptor: None,
        }
    }

    /// Enable AES-CFB8 encryption with the given shared secret (16 bytes).
    /// In MC protocol, key == IV == shared secret.
    pub fn enable_encryption(&mut self, shared_secret: &[u8]) {
        let key: [u8; 16] = shared_secret
            .try_into()
            .expect("shared secret must be 16 bytes");
        self.encryptor = Some(Cfb8Cipher::new(&key, &key));
        self.decryptor = Some(Cfb8Cipher::new(&key, &key));
    }

    /// Enable zlib compression with the given threshold.
    pub fn enable_compression(&mut self, threshold: i32) {
        self.compression_threshold = Some(threshold);
    }

    /// Read a single packet frame, returning (packet_id, payload).
    pub async fn read_packet(&mut self) -> anyhow::Result<(i32, BytesMut)> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Connection has been split"))?;
        loop {
            if let Some(result) = try_parse_packet(&mut self.read_buf, self.compression_threshold)?
            {
                return Ok(result);
            }
            let mut tmp = [0u8; 4096];
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                return Err(anyhow::anyhow!("Connection closed"));
            }
            let data = &mut tmp[..n];
            if let Some(ref mut decryptor) = self.decryptor {
                decryptor.decrypt(data);
            }
            self.read_buf.extend_from_slice(data);
        }
    }

    /// Write a packet with the given ID and payload.
    pub async fn write_packet(&mut self, packet_id: i32, payload: &[u8]) -> anyhow::Result<()> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Connection has been split"))?;
        let frame = build_frame(
            packet_id,
            payload,
            self.compression_threshold,
            &mut self.encryptor,
        );
        stream.write_all(&frame).await?;
        Ok(())
    }

    pub fn peer_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.stream
            .as_ref()
            .map(|s| s.peer_addr())
            .unwrap_or(Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "dummy connection",
            )))
    }

    /// Split the connection into read and write halves for concurrent I/O.
    /// Compression and encryption state is transferred to each half.
    pub fn into_split(mut self) -> (ConnectionReader, ConnectionWriter) {
        let stream = self.stream.take().expect("cannot split a dummy connection");
        let (read_half, write_half) = stream.into_split();
        (
            ConnectionReader {
                stream: read_half,
                read_buf: self.read_buf,
                compression_threshold: self.compression_threshold,
                decryptor: self.decryptor,
            },
            ConnectionWriter {
                stream: write_half,
                compression_threshold: self.compression_threshold,
                encryptor: self.encryptor,
            },
        )
    }
}

/// Read half of a split connection.
pub struct ConnectionReader {
    stream: OwnedReadHalf,
    read_buf: BytesMut,
    compression_threshold: Option<i32>,
    decryptor: Option<Cfb8Cipher>,
}

impl ConnectionReader {
    pub async fn read_packet(&mut self) -> anyhow::Result<(i32, BytesMut)> {
        loop {
            if let Some(result) =
                try_parse_packet(&mut self.read_buf, self.compression_threshold)?
            {
                return Ok(result);
            }
            let mut tmp = [0u8; 4096];
            let n = self.stream.read(&mut tmp).await?;
            if n == 0 {
                return Err(anyhow::anyhow!("Connection closed"));
            }
            let data = &mut tmp[..n];
            if let Some(ref mut decryptor) = self.decryptor {
                decryptor.decrypt(data);
            }
            self.read_buf.extend_from_slice(data);
        }
    }
}

/// Write half of a split connection.
pub struct ConnectionWriter {
    stream: OwnedWriteHalf,
    compression_threshold: Option<i32>,
    encryptor: Option<Cfb8Cipher>,
}

impl ConnectionWriter {
    pub async fn write_packet(&mut self, packet_id: i32, payload: &[u8]) -> anyhow::Result<()> {
        let frame = build_frame(
            packet_id,
            payload,
            self.compression_threshold,
            &mut self.encryptor,
        );
        self.stream.write_all(&frame).await?;
        Ok(())
    }
}

// === Shared helpers ===

fn try_parse_packet(
    read_buf: &mut BytesMut,
    compression_threshold: Option<i32>,
) -> anyhow::Result<Option<(i32, BytesMut)>> {
    if read_buf.is_empty() {
        return Ok(None);
    }

    let mut peek = read_buf.clone();
    let length = match read_varint(&mut peek) {
        Ok(len) => len as usize,
        Err(_) => return Ok(None),
    };

    let varint_bytes = read_buf.len() - peek.len();

    if peek.remaining() < length {
        return Ok(None);
    }

    read_buf.advance(varint_bytes);
    let mut packet_data = read_buf.split_to(length);

    if let Some(_threshold) = compression_threshold {
        let data_length = read_varint(&mut packet_data)? as usize;
        if data_length > 0 {
            let mut decompressed = vec![0u8; data_length];
            let mut decoder = ZlibDecoder::new(&packet_data[..]);
            decoder.read_exact(&mut decompressed)?;
            packet_data = BytesMut::from(&decompressed[..]);
        }
    }

    let packet_id = read_varint(&mut packet_data)?;
    trace!(
        "Read packet id=0x{:02X} len={}",
        packet_id,
        packet_data.len()
    );

    Ok(Some((packet_id, packet_data)))
}

fn build_frame(
    packet_id: i32,
    payload: &[u8],
    compression_threshold: Option<i32>,
    encryptor: &mut Option<Cfb8Cipher>,
) -> BytesMut {
    let mut packet_buf = BytesMut::new();
    write_varint(&mut packet_buf, packet_id);
    packet_buf.extend_from_slice(payload);

    let mut frame = BytesMut::new();

    if let Some(threshold) = compression_threshold {
        let uncompressed_len = packet_buf.len() as i32;
        if uncompressed_len >= threshold {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            let _ = encoder.write_all(&packet_buf);
            let compressed = encoder.finish().unwrap_or_default();

            let data_length_size = varint_len(uncompressed_len);
            let total_length = data_length_size + compressed.len();
            write_varint(&mut frame, total_length as i32);
            write_varint(&mut frame, uncompressed_len);
            frame.extend_from_slice(&compressed);
        } else {
            let total_length = 1 + packet_buf.len();
            write_varint(&mut frame, total_length as i32);
            write_varint(&mut frame, 0);
            frame.extend_from_slice(&packet_buf);
        }
    } else {
        write_varint(&mut frame, packet_buf.len() as i32);
        frame.extend_from_slice(&packet_buf);
    }

    if let Some(ref mut enc) = encryptor {
        enc.encrypt(&mut frame);
    }

    frame
}
