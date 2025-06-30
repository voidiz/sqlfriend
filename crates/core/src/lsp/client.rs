use std::{str::FromStr, sync::Arc, time::Duration};

use anyhow::{anyhow, Context};
use jsonrpsee_types::{response::Success, RequestSer, Response};
use lsp_types::{CompletionResponse, InitializeResult, Url};
use serde::Deserialize;
use serde_json::Value;
use tokio::{
    sync::{broadcast, RwLock},
    task::JoinHandle,
    time::timeout,
};

use crate::{
    config::{self, Connection},
    logging::Logger,
    lsp::payload::{self, LspPayload},
    state::State,
};

#[derive(Clone)]
pub struct LspClient {
    /// Used to send requests to the LSP server.
    req_tx: broadcast::Sender<String>,

    /// Used to receive LSP request responses from the LSP server. Sender is passed so that a
    /// receiver can be created for each instance of the LspClient.
    req_output_tx: broadcast::Sender<Vec<u8>>,

    /// Document URI placeholder used to identify the REPL input.
    document_uri: &'static Url,

    /// Shared application state.
    state: State,

    /// Used to log messages.
    logger: Logger,

    /// True if initialized.
    initialized: Arc<RwLock<bool>>,
}

impl LspClient {
    // Timeout for blocking requests.
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

    pub fn new(
        req_tx: broadcast::Sender<String>,
        req_output_tx: broadcast::Sender<Vec<u8>>,
        state: State,
        logger: Logger,
    ) -> Self {
        LspClient {
            req_tx,
            req_output_tx,
            document_uri: Box::leak(Box::new(
                Url::from_str("repl:///repl").expect("uri should be valid"),
            )),
            state,
            logger,
            initialized: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn is_initialized(&self) -> bool {
        let initialized = self.initialized.read().await;
        *initialized
    }

    /// Inform the LSP server that the text file (REPL input) changed.
    pub async fn on_change(&self, text: &str) -> anyhow::Result<()> {
        // We don't need to change the version number since we sync
        // the full input every time
        let change_request = payload::did_change(self.document_uri.clone(), 1, text)?;
        *self.state.lsp_text.lock().await = text.to_string();
        self.send_payload(change_request).await
    }

    /// Request the LSP server for auto completion at the given cursor position.
    /// Blocks until a request is received or until it times out.
    ///
    /// Some LSP servers such as sqls don't seem to conform to the specification
    /// fully, so we need to do some manual parsing.
    pub async fn request_completion(&self, line: u32, offset: u32) -> anyhow::Result<Vec<String>> {
        let completion_request = payload::completion(self.document_uri.clone(), line, offset)?;

        let res = self
            .send_blocking_request::<Value>(completion_request)
            .await?;

        // Try parsing using lsp_types
        let response = serde_json::from_value::<CompletionResponse>(res.clone());
        if let Ok(res) = response {
            let items = match res {
                CompletionResponse::Array(arr) => arr,
                CompletionResponse::List(list) => list.items,
            };

            return Ok(items.into_iter().map(|item| item.label).collect());
        }

        // Fall back to manual parsing
        let items: Option<Vec<String>> = (|| {
            // Assume that it is an array of CompletionItem.
            let items = res
                .as_array()?
                .iter()
                .map(|item| item.get("label")?.as_str());

            if items.clone().any(|item| item.is_none()) {
                return None;
            }

            Some(items.filter_map(|item| Some(item?.to_string())).collect())
        })();

        items.ok_or(anyhow!("failed parsing completion response: {:?}", res))
    }

    /// Initialize the LSP server with the given connection.
    pub async fn init_lsp_server(
        &self,
        server_type: &config::LspServerType,
        connection: Connection,
    ) -> anyhow::Result<()> {
        // Prepare the language server, need to wait for the initialize to return before we
        // continue
        let init_options = server_type.to_initialization_options(connection)?;
        let init_payload = payload::initialize(init_options)?;
        let _ = self
            .send_blocking_request::<InitializeResult>(init_payload)
            .await?;

        // Acknowledge that we've received the initialize response. Used by
        // postgres-language-server to read the configuration file and connect to the database.
        let initialized_payload = payload::initialized()?;
        self.send_payload(initialized_payload).await?;

        // Create our "document" (in reality it's just the current input in the REPL)
        let open_payload = payload::did_open(self.document_uri.clone(), "")?;
        self.send_payload(open_payload).await?;

        let mut initialized = self.initialized.write().await;
        *initialized = true;

        Ok(())
    }

    /// Shortcut to get the logger.
    pub fn get_logger(&self) -> &Logger {
        &self.logger
    }

    /// Helper that makes a async LSP request that resolves when the
    /// response is retrieved, or times out.
    ///
    /// T is the type of expected result payload.
    async fn send_blocking_request<T: Send + Sync + Clone + for<'de> Deserialize<'de> + 'static>(
        &self,
        mut req: RequestSer<'static>,
    ) -> anyhow::Result<T> {
        let req_payload = req.to_payload()?;

        // Wait for response
        let mut output_rx = self.req_output_tx.subscribe();
        let error_message = format!(
            "received no response for blocking request: {}",
            &req_payload
        );
        let res_payload: JoinHandle<anyhow::Result<T>> = tokio::spawn(async move {
            let res = loop {
                let body = timeout(Self::REQUEST_TIMEOUT, output_rx.recv())
                    .await
                    .with_context(|| error_message.clone())??;

                // Find the body with the corresponding ID. We defer the parsing of the payload
                // until later so that we can return an error if the ID is matching but the payload
                // has an unexpected structure.
                if let Ok(res) = serde_json::from_slice::<Response<Value>>(&body) {
                    if req.id == res.id {
                        break res.into_owned();
                    }
                }
            };

            let payload = Success::try_from(res)?;
            serde_json::from_value(payload.result)
                .map_err(anyhow::Error::from)
                .with_context(|| "failed to deserialize server response")
        });

        self.req_tx.send(req_payload)?;

        res_payload.await?
    }

    /// Helper that sends an LSP payload to the server without waiting for a response.
    async fn send_payload(&self, mut req: impl LspPayload) -> anyhow::Result<()> {
        let req_payload = req.to_payload()?;
        self.req_tx.send(req_payload)?;
        Ok(())
    }
}
