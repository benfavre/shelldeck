use thiserror::Error;

#[derive(Error, Debug)]
pub enum TerminalError {
    #[error("PTY error: {0}")]
    Pty(String),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Session closed")]
    SessionClosed,
    #[error("Resize failed: {0}")]
    Resize(String),
}

pub type Result<T> = std::result::Result<T, TerminalError>;
