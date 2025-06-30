use thiserror::Error;

use crate::config;

// TODO: Get rid of the map_errs
#[derive(Error, Debug)]
pub enum SqlFriendError {
    #[error("invalid connection name: `{0}`")]
    InvalidConnectionName(String),

    #[error("invalid command usage: `{0}`")]
    InvalidCommandUsage(String),

    #[error("invalid command: `{0}`")]
    InvalidCommand(String),

    #[error("invalid LSP server: `{0}`, expected one of {1:?}")]
    InvalidLspServer(String, Vec<config::LspServerType>),

    #[error(transparent)]
    Unknown(#[from] anyhow::Error),
}
