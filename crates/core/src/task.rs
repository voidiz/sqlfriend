use std::{future::Future, pin::Pin};

use anyhow::bail;
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinSet,
};

use crate::{
    config::{self, Connection},
    logging::Logger,
    lsp::{client::LspClient, server::LspServer},
};

pub type Task = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;

#[derive(Debug, Clone)]
pub enum Command {
    /// Start the LSP server with the given settings and connection. Any existing server is killed.
    SpawnLsp(config::LspServerType, Connection),
}

#[derive(Debug, Clone)]
pub enum BroadcastMessage {
    /// Kill LSP server tasks.
    KillLsp,
}

/// TaskManager is responsible for executing and stopping tasks.
pub struct TaskManager {
    /// JoinSet for all running tasks.
    pub set: JoinSet<anyhow::Result<()>>,

    logger: Logger,

    /// Channel used to receive task commands.
    command_tx: mpsc::Sender<Command>,
    command_rx: mpsc::Receiver<Command>,

    /// Broadcast channel for all tasks.
    broadcast_tx: broadcast::Sender<BroadcastMessage>,

    /// Used to spawn the LSP server.
    lsp_server: LspServer,

    /// Used to initialize the LSP server.
    lsp_client: LspClient,
}

impl TaskManager {
    pub fn new(logger: Logger, lsp_server: LspServer, lsp_client: LspClient) -> Self {
        let set = JoinSet::new();

        let (command_tx, command_rx) = mpsc::channel(1);

        // Receivers will be created when tasks are spawned.
        let (broadcast_tx, _) = broadcast::channel(1);

        Self {
            logger,
            set,
            command_tx,
            command_rx,
            broadcast_tx,
            lsp_server,
            lsp_client,
        }
    }

    pub fn get_command_tx(&self) -> mpsc::Sender<Command> {
        self.command_tx.clone()
    }

    /// Start the task manager and await all tasks.
    pub async fn run(mut self) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                command = self.command_rx.recv() => {
                    self.handle_command(command).await;
                }
                Some(result) = self.set.join_next() => {
                    self.handle_task(result)?;
                }
                else => {
                    return Ok(());
                }
            }
        }
    }

    /// Kill any existing LSP servers, spawn the one given by `protocol` and connect to
    /// `connection`.
    pub async fn spawn_lsp(
        &mut self,
        server_type: config::LspServerType,
        connection: Connection,
    ) -> anyhow::Result<()> {
        if self.broadcast_tx.send(BroadcastMessage::KillLsp).is_err() {
            self.logger
                .debug("no existing LSP server running, skipping shutdown")?;
        }

        let protocol = match server_type {
            config::LspServerType::Sqls | config::LspServerType::SqlLs => {
                server_type.to_stdio_cmd(std::iter::empty())
            }
            config::LspServerType::PgTools => {
                let config_path = connection.clone().to_postgres_ls_config_file()?;
                server_type.to_stdio_cmd([format!("--config-path={config_path}")])
            }
        };

        let tasks = self.lsp_server.init(protocol, &mut self.broadcast_tx)?;
        for task in tasks {
            self.set.spawn(task);
        }

        self.lsp_client
            .init_lsp_server(&server_type, connection.clone())
            .await?;

        self.logger
            .standard(&format!("Connected to {}.", connection.name))?;

        Ok(())
    }

    /// Handle the result of joining a task in the JoinSet.
    fn handle_task(
        &mut self,
        result: Result<Result<(), anyhow::Error>, tokio::task::JoinError>,
    ) -> anyhow::Result<()> {
        match result {
            Ok(Ok(_)) => {
                // TODO: need to handle lsp task finishing with success, should broadcast KillLsp
                // self.set.abort_all();
                Ok(())
            }
            Ok(Err(e)) => {
                self.set.abort_all();
                bail!(e)
            }
            Err(e) => {
                self.set.abort_all();
                bail!(e)
            }
        }
    }

    /// Handle an incoming command.
    async fn handle_command(&mut self, command: Option<Command>) {
        let command = match command {
            Some(cmd) => cmd,
            None => return,
        };

        let result = match command {
            Command::SpawnLsp(server_type, connection) => {
                self.spawn_lsp(server_type, connection).await
            }
        };

        if let Err(e) = result {
            self.logger
                .error(&format!("task manager command failed: {e:#}"))
                .unwrap();
        }
    }
}

#[derive(Clone)]
pub struct TaskController {
    /// Channel used to inform the TaskManager.
    command_tx: mpsc::Sender<Command>,
}

/// TaskController is responsible for sending commands to the TaskManager.
impl TaskController {
    pub fn new(command_tx: mpsc::Sender<Command>) -> Self {
        Self { command_tx }
    }

    /// Execute the given command in the TaskManager.
    pub async fn execute(&self, command: Command) -> anyhow::Result<()> {
        self.command_tx.send(command).await?;
        Ok(())
    }
}
