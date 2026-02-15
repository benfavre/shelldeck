use thiserror::Error;

#[derive(Error, Debug)]
pub enum ShellDeckError {
    #[error("SSH config parse error: {0}")]
    SshConfigParse(String),
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Config error: {0}")]
    Config(String),
    #[error("Keychain error: {0}")]
    Keychain(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, ShellDeckError>;
