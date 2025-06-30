use std::{
    fs::{self, File},
    path::PathBuf,
};

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    db_client::DbClient,
    error::SqlFriendError,
    lsp::{client::LspClient, server::CommunicationProtocol},
    task::{self, TaskController},
};

const CONFIG_SUBDIRECTORY: &str = "sqlfriend";
const CONFIG_FILENAME: &str = "sqlfriend.toml";

#[derive(Serialize, Deserialize, Default, Debug, Eq, PartialEq, Clone)]
pub enum LspServerType {
    #[default]
    /// sqls
    Sqls,
    /// sql-language-server
    SqlLs,
    /// postgrestools/postgres-language-server
    PgTools,
}

/// Connection configuration for sqls.
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct SqlsConnectionConfig {
    driver: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    passwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_source_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    db_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proto: Option<String>,
}

/// Connection configuration for sql-language-server.
#[derive(Default, Serialize)]
struct SqlLsConnectionConfig {
    name: String,
    adapter: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    database: Option<String>,
}

/// Connection configuration for sql-language-server.
#[derive(Default, Serialize)]
struct PgToolsConnectionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    database: Option<String>,
}

impl LspServerType {
    pub const VALUES: [Self; 3] = [Self::Sqls, Self::SqlLs, Self::PgTools];

    /// Convert into CommunicationProtocol::Stdio.
    pub fn to_stdio_cmd(
        &self,
        extra_args: impl IntoIterator<Item = String>,
    ) -> CommunicationProtocol {
        match self {
            Self::Sqls => CommunicationProtocol::Stdio {
                cmd: "sqls".to_string(),
                args: vec![],
            },
            Self::SqlLs => CommunicationProtocol::Stdio {
                cmd: "sql-language-server".to_string(),
                args: ["up", "--method", "stdio", "-d"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            },
            Self::PgTools => {
                let mut args = vec!["lsp-proxy".to_string()];
                args.extend(extra_args);

                CommunicationProtocol::Stdio {
                    cmd: "postgrestools".to_string(),
                    args,
                }
            }
        }
    }

    /// Convert into LSP initialization options.
    pub fn to_initialization_options(
        &self,
        connection: Connection,
    ) -> anyhow::Result<Option<Value>> {
        match self {
            Self::Sqls => Ok(connection.to_sqls_connection_config().map(Some)?),
            Self::SqlLs => Ok(connection.to_sql_ls_connection_config().map(Some)?),
            // PgTools doesn't support initialization options. We need to pass the connection
            // config through the postgrestools.jsonc config file.
            Self::PgTools => Ok(None),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ConnectionSettings {
    Sqlite {
        filename: String,
    },
    MySql {
        host: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        port: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        user: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        password: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        database: Option<String>,
    },
    Postgres {
        host: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        port: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        user: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        password: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        database: Option<String>,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Connection {
    pub name: String,
    pub settings: ConnectionSettings,
}

impl Connection {
    /// Convert DSN to a sqls-compatible connectionConfig.
    pub fn to_sqls_connection_config(self) -> Result<Value, SqlFriendError> {
        let driver = match self.settings {
            ConnectionSettings::Sqlite { .. } => "sqlite3",
            ConnectionSettings::MySql { .. } => "mysql",
            ConnectionSettings::Postgres { .. } => "postgresql",
        }
        .to_string();

        let config = match self.settings {
            ConnectionSettings::Sqlite { filename } => SqlsConnectionConfig {
                driver,
                data_source_name: Some("file:".to_string() + &filename),
                ..Default::default()
            },
            ConnectionSettings::MySql {
                host,
                port,
                user,
                password,
                database,
            } => SqlsConnectionConfig {
                driver,
                host: Some(host),
                port: Self::parse_port(port)?,
                user,
                passwd: password,
                db_name: database,
                proto: Some("tcp".to_string()),
                ..Default::default()
            },
            ConnectionSettings::Postgres {
                host,
                port,
                user,
                password,
                database,
            } => SqlsConnectionConfig {
                driver,
                host: Some(host),
                port: Self::parse_port(port)?,
                user,
                passwd: password,
                db_name: database,
                proto: Some("tcp".to_string()),
                ..Default::default()
            },
        };

        Ok(serde_json::json!({
            "connectionConfig": serde_json::to_value(config).map_err(|err| anyhow!(err))?
        }))
    }

    /// Create a temporary configuration file for postgrestools and return the path to it.
    pub fn to_postgres_ls_config_file(self) -> Result<String, SqlFriendError> {
        let config = match self.settings {
            ConnectionSettings::Postgres {
                host,
                port,
                user,
                password,
                database,
            } => PgToolsConnectionConfig {
                host: Some(host),
                port: Self::parse_port(port)?,
                username: user,
                password,
                database,
            },
            _ => {
                return Err(SqlFriendError::Unknown(anyhow!(
                    "cant use postgres-language-server with non-postgres database"
                )));
            }
        };

        let db_config = serde_json::to_value(config).map_err(|err| anyhow!(err))?;
        let config_value = serde_json::json!({
            "db": db_config
        });

        let tmp_dir = tempfile::TempDir::new().map_err(|err| anyhow!(err))?;
        let file_path = tmp_dir.path().join("postgrestools.jsonc");
        let file = File::create(&file_path).map_err(|err| anyhow!(err))?;
        serde_json::to_writer(&file, &config_value).map_err(|err| anyhow!(err))?;

        Ok(tmp_dir.keep().to_string_lossy().into_owned())
    }

    // Convert DSN to a sql-language-server-compatible connectionConfig.
    pub fn to_sql_ls_connection_config(self) -> Result<Value, SqlFriendError> {
        let adapter = match self.settings {
            ConnectionSettings::Sqlite { .. } => "sqlite3",
            ConnectionSettings::MySql { .. } => "mysql",
            ConnectionSettings::Postgres { .. } => "postgres",
        }
        .to_string();

        let name = self.name;
        let config = match self.settings {
            ConnectionSettings::Sqlite { filename } => SqlLsConnectionConfig {
                name,
                adapter,
                filename: Some(filename),
                ..Default::default()
            },
            ConnectionSettings::Postgres {
                host,
                port,
                user,
                password,
                database,
            }
            | ConnectionSettings::MySql {
                host,
                port,
                user,
                password,
                database,
            } => SqlLsConnectionConfig {
                name,
                adapter,
                host: Some(host),
                port: Self::parse_port(port)?,
                user,
                password,
                database,
                ..Default::default()
            },
        };

        let value = serde_json::to_value(config).map_err(|err| anyhow!(err))?;

        Ok(serde_json::json!({
            "connections": [value]
        }))
    }

    /// Connect to this connection
    pub async fn connect(
        &self,
        task_controller: &TaskController,
        db_client: &DbClient,
        lsp_client: &LspClient,
    ) -> anyhow::Result<()> {
        lsp_client
            .get_logger()
            .standard(&format!("Connecting to {}...", self.name))?;

        db_client.connect(self.clone()).await?;

        let server_type = match get_config()?.get_lsp_server() {
            Some(server) => server.to_owned(),
            None => {
                let default = LspServerType::default();
                lsp_client.get_logger().warn(&format!(
                    "No language server set, defaulting to {default:?}"
                ))?;
                default
            }
        };

        task_controller
            .execute(task::Command::SpawnLsp(server_type, self.clone()))
            .await?;

        Ok(())
    }

    fn parse_port(port: Option<String>) -> anyhow::Result<Option<u16>> {
        port.map(|port| {
            port.parse::<u16>()
                .with_context(|| format!("invalid port: {port}"))
        })
        .transpose()
    }
}

#[derive(Default, Serialize, Deserialize, Debug)]
pub struct Config {
    current_connection_name: Option<String>,
    lsp_server: Option<LspServerType>,
    connections: Vec<Connection>,
}

impl Config {
    pub fn add_connection(&mut self, connection: Connection) -> Result<(), SqlFriendError> {
        self.connections.push(connection);
        self.save()?;
        Ok(())
    }

    pub fn delete_connection(&mut self, name: &str) -> Result<(), SqlFriendError> {
        let connection_index = self
            .connections
            .iter_mut()
            .position(|connection| connection.name == name)
            .ok_or(SqlFriendError::InvalidConnectionName(name.to_string()))?;

        self.connections.swap_remove(connection_index);
        self.save()?;
        Ok(())
    }

    pub fn get_connections(&self) -> &Vec<Connection> {
        &self.connections
    }

    pub fn get_current_connection(&self) -> Option<&Connection> {
        let current_connection_name = self.current_connection_name.as_ref()?;

        self.connections
            .iter()
            .find(|connection| connection.name.as_str() == current_connection_name)
    }

    pub fn set_current_connection(&mut self, name: &str) -> Result<(), SqlFriendError> {
        let connection = self
            .connections
            .iter()
            .find(|connection| connection.name == name);

        if connection.is_none() {
            return Err(SqlFriendError::InvalidConnectionName(name.to_string()));
        }

        self.current_connection_name = Some(name.to_string());
        self.save()?;
        Ok(())
    }

    pub fn get_lsp_server(&self) -> Option<&LspServerType> {
        self.lsp_server.as_ref()
    }

    pub fn set_lsp_server(&mut self, lsp_server: LspServerType) -> anyhow::Result<()> {
        self.lsp_server = Some(lsp_server);
        self.save()?;
        Ok(())
    }

    fn save(&self) -> anyhow::Result<()> {
        let (dir_path, file_path) = get_config_path()?;
        let config_str = toml::to_string(self)?;
        fs::create_dir_all(dir_path)?;
        fs::write(file_path, config_str)?;

        Ok(())
    }
}

/// Returns (directory_path, file_path).
fn get_config_path() -> anyhow::Result<(PathBuf, PathBuf)> {
    let mut dir_path = dirs::config_dir().ok_or(anyhow!("couldn't find config directory"))?;
    dir_path.push(CONFIG_SUBDIRECTORY);

    let mut file_path = dir_path.clone();
    file_path.push(CONFIG_FILENAME);

    Ok((dir_path, file_path))
}

pub fn get_config() -> anyhow::Result<Config> {
    let (_, config_path) = get_config_path()?;
    if !config_path.exists() {
        return Ok(Default::default());
    }

    let config_file = fs::read_to_string(config_path)?;
    let config: Config = toml::from_str(config_file.as_str())?;
    Ok(config)
}
