use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error(transparent)]
    Application(#[from] anyhow::Error),
}

impl AppError {
    pub const fn exit_code(&self) -> u8 {
        1
    }
}
