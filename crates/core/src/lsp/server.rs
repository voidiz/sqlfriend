use std::{future::Future, pin::Pin, process::Stdio};

use anyhow::Context;
use jsonrpsee_types::{Notification, Response};
use serde_json::Value;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command},
    sync::broadcast,
};

use crate::lsp::response::read_body;
use crate::{logging::Logger, task};

use super::Task;

#[derive(Debug, Clone)]
pub enum CommunicationProtocol {
    Stdio {
        /// Path to LSP binary.
        cmd: String,

        /// Arguments for LSP binary.
        args: Vec<String>,
    },
}

/// All channels used by LspServer.
pub struct ServerChannels {
    /// Used to receive requests from the LSP client. This is a sender so we can create new
    /// receivers every time the LSP server is spawned.
    req_tx: broadcast::Sender<String>,

    /// Used to return LSP request responses to the LSP client.
    req_output_tx: broadcast::Sender<Vec<u8>>,

    /// Used to return LSP notifications to notification handler.
    notif_tx: broadcast::Sender<Vec<u8>>,
}

/// All channels used when interfacing with LspServer.
pub struct ClientChannels {
    /// Used to send requests to the LSP server.
    pub req_tx: broadcast::Sender<String>,

    /// Used to receive LSP request responses from the LSP server. This is a sender so we can
    /// create new receivers every time a LspClient is created.
    pub req_output_tx: broadcast::Sender<Vec<u8>>,

    /// Used to receive LSP notifications from the LSP server.
    pub notif_rx: broadcast::Receiver<Vec<u8>>,
}

pub struct LspServer {
    /// Used to log messages.
    logger: Logger,

    /// All channels used by the server.
    channels: ServerChannels,
}

impl LspServer {
    pub fn new(logger: Logger) -> (Self, ClientChannels) {
        // The receiver will be created when the LSP server is spawned.
        let (req_tx, _) = broadcast::channel(10);

        // The receiver will be created when a LspClient is created.
        let (req_output_tx, _) = broadcast::channel(10);

        let (notif_tx, notif_rx) = broadcast::channel(10);

        (
            LspServer {
                logger,
                channels: ServerChannels {
                    req_tx: req_tx.clone(),
                    req_output_tx: req_output_tx.clone(),
                    notif_tx,
                },
            },
            ClientChannels {
                req_tx,
                req_output_tx,
                notif_rx,
            },
        )
    }

    pub fn init(
        &mut self,
        protocol: CommunicationProtocol,
        broadcast_tx: &mut broadcast::Sender<task::BroadcastMessage>,
    ) -> anyhow::Result<Vec<Task>> {
        match protocol {
            CommunicationProtocol::Stdio { cmd, args } => self.init_stdio(cmd, args, broadcast_tx),
        }
    }

    /// Initializes the LSP server using stdio commmunication.
    /// Futures for [stdin, stdout, stderr] tasks are returned.
    /// Will not start until futures are awaited.
    fn init_stdio(
        &self,
        cmd: String,
        args: Vec<String>,
        broadcast_tx: &mut broadcast::Sender<task::BroadcastMessage>,
    ) -> anyhow::Result<Vec<Task>> {
        let mut child = Command::new(&cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn LSP server `{cmd}`"))?;

        let child_stdin = child
            .stdin
            .take()
            .expect("stdin shouldn't be taken anywhere else");
        let stdin_task = self.new_stdin_sender(child_stdin, broadcast_tx.subscribe());

        let child_stdout = child
            .stdout
            .take()
            .expect("stdout shouldn't be taken anywhere else");
        let stdout_task = self.new_stdout_reader(child_stdout, broadcast_tx.subscribe());

        let child_stderr = child
            .stderr
            .take()
            .expect("stderr shouldn't be taken anywhere else");
        let stderr_task = self.new_stderr_reader(child_stderr, broadcast_tx.subscribe());

        let process_task = self.new_process_manager(child, broadcast_tx.subscribe());

        Ok(vec![
            Box::pin(stdin_task),
            Box::pin(stdout_task),
            Box::pin(stderr_task),
            Box::pin(process_task),
        ])
    }

    /// Task that forwards messages from child_stdout to the output channel.
    fn new_stdout_reader(
        &self,
        child_stdout: ChildStdout,
        mut broadcast_rx: broadcast::Receiver<task::BroadcastMessage>,
    ) -> impl Future<Output = anyhow::Result<()>> {
        let logger_stdout = self.logger.clone();
        let req_output_tx = self.channels.req_output_tx.clone();
        let notif_tx = self.channels.notif_tx.clone();

        async move {
            let mut stdout: Pin<Box<dyn AsyncBufRead + Send>> =
                Box::pin(BufReader::new(child_stdout));

            loop {
                tokio::select! {
                    body = read_body(&mut stdout) => {
                        let body = body?;
                        let body_str = String::from_utf8_lossy(&body);
                        logger_stdout.debug(&format!("server stdout: {body_str}"))?;

                        // TODO: Figure out why an untagged enum doesn't work here
                        if serde_json::from_slice::<Response<Value>>(&body).is_ok() {
                            req_output_tx.send(body)?;
                            continue;
                        }

                        if serde_json::from_slice::<Notification<Value>>(&body).is_ok() {
                            notif_tx.send(body)?;
                            continue;
                        }

                        logger_stdout
                            .error(&format!("failed to deserialize server message: {body_str}"))?;
                    }
                    msg = broadcast_rx.recv() => {
                        match msg {
                            Ok(task::BroadcastMessage::KillLsp) => {
                                return Ok(());
                            }
                            Err(e) => anyhow::bail!(e)
                        }
                    }
                }
            }
        }
    }

    /// Task that forwards messages from the input channel to child_stdin.
    fn new_stdin_sender(
        &self,
        mut child_stdin: ChildStdin,
        mut broadcast_rx: broadcast::Receiver<task::BroadcastMessage>,
    ) -> impl Future<Output = anyhow::Result<()>> {
        let mut input_rx = self.channels.req_tx.subscribe();
        let logger_stdin = self.logger.clone();

        async move {
            loop {
                tokio::select! {
                    input = input_rx.recv() => {
                        let input = input?;
                        logger_stdin.debug(&format!("server stdin: {input}"))?;
                        child_stdin.write_all(input.as_bytes()).await?;
                    }
                    msg = broadcast_rx.recv() => {
                        match msg {
                            Ok(task::BroadcastMessage::KillLsp) => {
                                return Ok(());
                            }
                            Err(e) => anyhow::bail!(e)
                        }
                    }
                }
            }
        }
    }

    /// Task that forwards messages from child_stderr to the logger.
    fn new_stderr_reader(
        &self,
        child_stderr: ChildStderr,
        mut broadcast_rx: broadcast::Receiver<task::BroadcastMessage>,
    ) -> impl Future<Output = anyhow::Result<()>> {
        let logger_stderr = self.logger.clone();

        async move {
            let stderr = BufReader::new(child_stderr);
            let mut lines = stderr.lines();
            loop {
                tokio::select! {
                    line = lines.next_line() => {
                        let line = line?;
                        let line = match line {
                            Some(l) => l,
                            None => return Ok(())
                        };
                        logger_stderr.debug(&format!("server stderr: {line}"))?;
                    }
                    msg = broadcast_rx.recv() => {
                        match msg {
                            Ok(task::BroadcastMessage::KillLsp) => {
                                return Ok(());
                            }
                            Err(e) => anyhow::bail!(e)
                        }
                    }
                }
            }
        }
    }

    /// Task used to manage LSP server process.
    fn new_process_manager(
        &self,
        mut child: Child,
        mut broadcast_rx: broadcast::Receiver<task::BroadcastMessage>,
    ) -> impl Future<Output = anyhow::Result<()>> {
        async move {
            // If we add more broadcast messages in the future, this loop is necessary.
            #[allow(clippy::never_loop)]
            loop {
                tokio::select! {
                    msg = broadcast_rx.recv() => {
                        match msg {
                            Ok(task::BroadcastMessage::KillLsp) => {
                                child.kill().await?;
                                return Ok(());
                            }
                            Err(e) => anyhow::bail!(e),
                        }
                    }
                }
            }
        }
    }
}
