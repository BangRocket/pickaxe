use crate::{ConnectionState, InternalPacket};
use anyhow::Result;
use bytes::BytesMut;

/// Trait for version-specific protocol adapters.
/// Each supported MC version implements this trait.
pub trait ProtocolAdapter: Send + Sync {
    /// The protocol version number this adapter handles.
    fn protocol_version(&self) -> i32;

    /// Decode a raw packet from wire format into an InternalPacket.
    fn decode_packet(
        &self,
        state: ConnectionState,
        id: i32,
        data: &mut BytesMut,
    ) -> Result<InternalPacket>;

    /// Encode an InternalPacket into wire format bytes.
    fn encode_packet(
        &self,
        state: ConnectionState,
        packet: &InternalPacket,
    ) -> Result<BytesMut>;

    /// Get the registry data packets for the Configuration state.
    fn registry_data(&self) -> Vec<InternalPacket>;
}
