/// The state of a Minecraft protocol connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Handshaking,
    Status,
    Login,
    Configuration,
    Play,
}

impl ConnectionState {
    pub fn from_handshake_next(next: i32) -> Option<Self> {
        match next {
            1 => Some(ConnectionState::Status),
            2 => Some(ConnectionState::Login),
            _ => None,
        }
    }
}
