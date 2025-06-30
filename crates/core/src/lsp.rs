use client::LspClient;
use notification_handler::NotificationHandler;
use server::LspServer;

use crate::{logging::Logger, state::State, task::Task};

pub mod client;
pub mod completer;
pub mod notification_handler;
mod payload;
mod response;
pub mod server;

/// Create instances of the LspClient and LspServer.
pub fn build_lsp(state: State, logger: Logger) -> (LspClient, LspServer, NotificationHandler) {
    let (lsp_server, channels) = LspServer::new(logger.clone());
    let lsp_client = LspClient::new(
        channels.req_tx,
        channels.req_output_tx,
        state.clone(),
        logger.clone(),
    );

    let notification_handler = NotificationHandler::new(state, logger, channels.notif_rx);

    (lsp_client, lsp_server, notification_handler)
}
