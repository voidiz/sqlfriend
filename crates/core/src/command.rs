use std::{collections::HashMap, sync::LazyLock};

use anyhow::{anyhow, bail};

use crate::{
    config::{self, get_config},
    db_client::DbClient,
    error::SqlFriendError,
    logging::Logger,
    lsp::client::LspClient,
    task::TaskController,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command<'a> {
    pub description: &'a str,
    pub usage: &'a str,
}

#[macro_export]
macro_rules! command_prefix {
    () => {
        "/"
    };
}
pub use command_prefix;

macro_rules! input {
    ($prompt:expr) => {
        dialoguer::Input::new()
            .with_prompt($prompt)
            .interact_text()
            .map_err(|err| anyhow::anyhow!(err))?
    };
}

macro_rules! input_optional {
    ($prompt:expr) => {
        dialoguer::Input::new()
            .with_prompt($prompt)
            .allow_empty(true)
            .interact_text()
            .map(|input: String| if input.is_empty() { None } else { Some(input) })
            .map_err(|err| anyhow!(err))?
    };
}

pub static COMMANDS: LazyLock<HashMap<&str, Command>> = LazyLock::new(|| {
    HashMap::from([
        (
            "help",
            Command {
                description: "Display a list of available commands.",
                usage: concat!(command_prefix!(), "help"),
            },
        ),
        (
            "list",
            Command {
                description: "List all saved connections.",
                usage: concat!(command_prefix!(), "list"),
            },
        ),
        (
            "use",
            Command {
                description: "Change the active connection.",
                usage: concat!(command_prefix!(), "use <connection_name>"),
            },
        ),
        (
            "add",
            Command {
                description: "Add a new connection.",
                usage: concat!(command_prefix!(), "add"),
            },
        ),
        (
            "delete",
            Command {
                description: "Delete a saved connection.",
                usage: concat!(command_prefix!(), "delete <connection_name>"),
            },
        ),
        (
            "set_lsp_server",
            Command {
                description:
                    "Set the LSP server (Sqls, SqlLs, or PgTools). Should be available in $PATH.",
                usage: concat!(command_prefix!(), "set_lsp_server <lsp_server>"),
            },
        ),
    ])
});

/// Returns true if the given string looks like a command.
pub fn is_maybe_command(line: &str) -> bool {
    line.starts_with(command_prefix!())
}

/// Parse and execute the given command (line).
pub async fn handle_command(
    task_controller: &TaskController,
    db_client: &DbClient,
    lsp_client: &LspClient,
    line: &str,
) -> anyhow::Result<()> {
    let tokens = line.split(" ").collect::<Vec<_>>();
    let &cmd = tokens
        .first()
        .ok_or(anyhow!("an empty string is not a command"))?;

    if !cmd.starts_with(command_prefix!()) {
        bail!("`{line}` is not a command")
    }

    let prefix_length = command_prefix!().len();
    let stripped_cmd = &cmd[prefix_length..];
    let args = &tokens[1..];

    let cmd_result = match stripped_cmd {
        "list" => handle_list(lsp_client.get_logger()),
        "use" => handle_use(task_controller, db_client, lsp_client, args).await,
        "add" => handle_add(lsp_client.get_logger(), args),
        "delete" => handle_delete(lsp_client.get_logger(), args),
        "set_lsp_server" => {
            handle_set_lsp_server(task_controller, db_client, lsp_client, args).await
        }
        "help" => handle_help(lsp_client.get_logger()),
        _ => Err(SqlFriendError::InvalidCommand(cmd.to_string())),
    };

    match cmd_result {
        Ok(()) => (),
        Err(e) => lsp_client.get_logger().error(&e.to_string())?,
    };

    Ok(())
}

fn handle_help(logger: &Logger) -> Result<(), SqlFriendError> {
    let mut output_lines = COMMANDS
        .values()
        .map(|cmd| format!("\t{:35} - {}", cmd.usage, cmd.description))
        .collect::<Vec<_>>();

    output_lines.sort_unstable();
    logger.standard(&output_lines.join("\n"))?;

    Ok(())
}

fn handle_list(logger: &Logger) -> Result<(), SqlFriendError> {
    let config = get_config()?;
    let connections = config.get_connections();
    let output = connections
        .iter()
        .map(|connection| format!("{}: {:?}", connection.name, connection.settings))
        .collect::<Vec<_>>()
        .join("\n");

    logger.standard(&output)?;
    Ok(())
}

async fn handle_use(
    task_controller: &TaskController,
    db_client: &DbClient,
    lsp_client: &LspClient,
    args: &[&str],
) -> Result<(), SqlFriendError> {
    if args.len() != 1 {
        let use_usage = COMMANDS
            .get("use")
            .ok_or(anyhow!("internal error: use command doesn't exist"))?;

        return Err(SqlFriendError::InvalidCommandUsage(
            use_usage.usage.to_string(),
        ));
    }

    let mut config = get_config()?;
    let name = args[0];
    config.set_current_connection(name)?;

    let connection = config
        .get_current_connection()
        .ok_or(anyhow!("internal error: current connection not set"))?;

    connection
        .connect(task_controller, db_client, lsp_client)
        .await?;

    Ok(())
}

fn handle_add(logger: &Logger, args: &[&str]) -> Result<(), SqlFriendError> {
    if !args.is_empty() {
        let add_usage = COMMANDS
            .get("add")
            .ok_or(anyhow!("internal error: add command doesn't exist"))?;

        return Err(SqlFriendError::InvalidCommandUsage(
            add_usage.usage.to_string(),
        ));
    }

    let databases = vec!["postgres", "mysql", "sqlite"];
    let database_index = dialoguer::Select::new()
        .with_prompt("Choose a database type")
        .items(&databases)
        .interact()
        .map_err(|err| anyhow!(err))?;

    let name: String = dialoguer::Input::new()
        .with_prompt("Specify a name")
        .interact_text()
        .map_err(|err| anyhow!(err))?;

    let connection = match databases[database_index] {
        "postgres" => {
            let host = input!("Hostname");
            let port = input_optional!("Port (leave empty if none)");
            let user = input_optional!("Username (leave empty if none)");
            let password = input_optional!("Password (leave empty if none)");
            let database = input_optional!("Database (leave empty if none)");

            config::Connection {
                name: name.clone(),
                settings: config::ConnectionSettings::Postgres {
                    host,
                    port,
                    user,
                    password,
                    database,
                },
            }
        }
        "mysql" => {
            let host = input!("Hostname");
            let port = input_optional!("Port (leave empty if none)");
            let user = input_optional!("Username (leave empty if none)");
            let password = input_optional!("Password (leave empty if none)");
            let database = input_optional!("Database (leave empty if none)");

            config::Connection {
                name: name.clone(),
                settings: config::ConnectionSettings::MySql {
                    host,
                    port,
                    user,
                    password,
                    database,
                },
            }
        }
        "sqlite" => {
            let path = dialoguer::Input::new()
                .with_prompt("Path to database file")
                .interact_text()
                .map_err(|err| anyhow!(err))?;
            config::Connection {
                name: name.clone(),
                settings: config::ConnectionSettings::Sqlite { filename: path },
            }
        }
        _ => unreachable!("dialogue should be limited to these databases"),
    };

    let log_msg = format!("Stored {}: {:?}.", name, connection);
    get_config()?.add_connection(connection)?;
    logger.standard(&log_msg)?;

    Ok(())
}

fn handle_delete(logger: &Logger, args: &[&str]) -> Result<(), SqlFriendError> {
    if args.len() != 1 {
        let delete_usage = COMMANDS
            .get("delete")
            .ok_or(anyhow!("internal error: delete command doesn't exist"))?;

        return Err(SqlFriendError::InvalidCommandUsage(
            delete_usage.usage.to_string(),
        ));
    }

    let name = args[0];
    let mut config = get_config()?;
    config.delete_connection(name)?;
    logger.standard(&format!("Deleted {name}."))?;

    Ok(())
}

async fn handle_set_lsp_server(
    task_controller: &TaskController,
    db_client: &DbClient,
    lsp_client: &LspClient,
    args: &[&str],
) -> Result<(), SqlFriendError> {
    if args.len() != 1 {
        let cmd = COMMANDS.get("set_lsp_server").ok_or(anyhow!(
            "internal error: set_lsp_server command doesn't exist"
        ))?;

        return Err(SqlFriendError::InvalidCommandUsage(cmd.usage.to_string()));
    }

    let server_arg = args[0];
    let server_type = match server_arg.to_lowercase().as_str() {
        "sqls" => config::LspServerType::Sqls,
        "sqlls" => config::LspServerType::SqlLs,
        "pgtools" => config::LspServerType::PgTools,
        _ => {
            return Err(SqlFriendError::InvalidLspServer(
                server_arg.to_string(),
                config::LspServerType::VALUES.to_vec(),
            ))
        }
    };

    let mut config = get_config()?;
    config.set_lsp_server(server_type.clone())?;

    if let Some(connection) = config.get_current_connection() {
        connection
            .connect(task_controller, db_client, lsp_client)
            .await?;
    }

    Ok(())
}
