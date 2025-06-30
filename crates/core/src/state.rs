use std::sync::Arc;

use tokio::sync::Mutex;

/// State contains shared application state.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// Current text reported to LSP
    pub lsp_text: Arc<Mutex<String>>,
}
