use thiserror::Error;

#[derive(Error, Debug)]
pub enum SshError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Authentication failed: {0}")]
    AuthFailed(String),
    #[error("Channel error: {0}")]
    Channel(String),
    #[error("Tunnel error: {0}")]
    Tunnel(String),
    #[error("Session closed")]
    SessionClosed,
    #[error("Port already in use: {0}")]
    PortInUse(u16),
    #[error("Timeout")]
    Timeout,
    #[error("Key error: {0}")]
    Key(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Russh error: {0}")]
    Russh(String),
}

pub type Result<T> = std::result::Result<T, SshError>;
