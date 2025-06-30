use std::{ops::Deref, sync::Arc};

use anyhow::anyhow;
use sqlx::{
    any::install_default_drivers,
    mysql::{MySqlConnectOptions, MySqlRow},
    postgres::{PgConnectOptions, PgRow},
    sqlite::{SqliteConnectOptions, SqliteRow},
    Column, FromRow, MySql, MySqlPool, PgPool, Postgres, Row, Sqlite, SqlitePool, ValueRef,
};
use sqlx_core::type_checking::TypeChecking;
use tokio::sync::{Mutex, RwLock, RwLockReadGuard};

use crate::{
    config::{self, Connection},
    logging::Logger,
};

#[derive(Debug, Clone)]
enum DbPool {
    Sqlite(SqlitePool),
    MySql(MySqlPool),
    Postgres(PgPool),
}

#[derive(Default, Clone)]
pub struct DbClient {
    /// Database connection pool.
    pool: Arc<Mutex<Option<DbPool>>>,

    /// Connection that the pool is using.
    current_connection: Arc<RwLock<Option<Connection>>>,
}

#[derive(Default)]
struct DbRow {
    /// Columns and their values in this row.
    columns: Vec<(String, String)>,
}

impl FromRow<'_, SqliteRow> for DbRow {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        let mut db_row = DbRow::default();
        for (index, column) in row.columns().iter().enumerate() {
            db_row
                .columns
                .push((column.name().to_string(), format_sqlite_value(row, index)));
        }

        Ok(db_row)
    }
}

fn format_sqlite_value(row: &SqliteRow, index: usize) -> String {
    match row.try_get_raw(index) {
        Ok(value) => {
            if ValueRef::is_null(&value) {
                return "<NULL>".to_string();
            }

            // fmt_value_debug gives precedence to bool which is compatible with the
            // INTEGER type in sqlite. We therefore need to try to decode the value
            // manually as an i64, otherwise all INTEGER values will be decoded as bool.
            let sqlite_value = value.to_owned();
            let fmt_value =
                if <i64 as sqlx_core::types::Type<Sqlite>>::compatible(&value.type_info()) {
                    sqlx_core::type_checking::FmtValue::debug::<i64>(&sqlite_value)
                } else {
                    Sqlite::fmt_value_debug(&sqlite_value)
                };

            format!("{fmt_value:?}").trim_matches('"').to_string()
        }
        Err(e) => format!("decode error: {e:?}"),
    }
}

impl FromRow<'_, MySqlRow> for DbRow {
    fn from_row(row: &MySqlRow) -> Result<Self, sqlx::Error> {
        let mut db_row = DbRow::default();
        for (index, column) in row.columns().iter().enumerate() {
            db_row
                .columns
                .push((column.name().to_string(), format_mysql_value(row, index)));
        }

        Ok(db_row)
    }
}

fn format_mysql_value(row: &MySqlRow, index: usize) -> String {
    match row.try_get_raw(index) {
        Ok(value) => {
            if ValueRef::is_null(&value) {
                return "<NULL>".to_string();
            }

            let mysql_value = ValueRef::to_owned(&value);
            let fmt_value = MySql::fmt_value_debug(&mysql_value);
            format!("{fmt_value:?}").trim_matches('"').to_string()
        }
        Err(e) => format!("decode error: {e:?}"),
    }
}

impl FromRow<'_, PgRow> for DbRow {
    fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
        let mut db_row = DbRow::default();
        for (index, column) in row.columns().iter().enumerate() {
            db_row
                .columns
                .push((column.name().to_string(), format_pg_value(row, index)));
        }

        Ok(db_row)
    }
}

fn format_pg_value(row: &PgRow, index: usize) -> String {
    match row.try_get_raw(index) {
        Ok(value) => {
            if ValueRef::is_null(&value) {
                return "<NULL>".to_string();
            }

            let pg_value = ValueRef::to_owned(&value);
            let fmt_value = Postgres::fmt_value_debug(&pg_value);
            format!("{fmt_value:?}").trim_matches('"').to_string()
        }
        Err(e) => format!("decode error: {e:?}"),
    }
}

impl DbClient {
    /// Initialize database drivers.
    pub fn initialize() {
        install_default_drivers();
    }

    /// Connect to the given DSN and replace the stored pool.
    pub async fn connect(&self, connection: Connection) -> anyhow::Result<()> {
        // Close any existing connection pools.
        self.close().await;

        let pool = match &connection.settings {
            config::ConnectionSettings::Sqlite { filename } => {
                let connect_options = SqliteConnectOptions::new().filename(filename);
                DbPool::Sqlite(SqlitePool::connect_with(connect_options).await?)
            }
            config::ConnectionSettings::MySql {
                host,
                port,
                user,
                password,
                database,
            } => {
                let mut connect_options = MySqlConnectOptions::new().host(host);
                if let Some(port) = port {
                    connect_options = connect_options.port(port.parse()?);
                }
                if let Some(user) = user {
                    connect_options = connect_options.username(user);
                }
                if let Some(password) = password {
                    connect_options = connect_options.password(password);
                }
                if let Some(database) = database {
                    connect_options = connect_options.database(database);
                }
                DbPool::MySql(MySqlPool::connect_with(connect_options).await?)
            }
            config::ConnectionSettings::Postgres {
                host,
                port,
                user,
                password,
                database,
            } => {
                let mut connect_options = PgConnectOptions::new().host(host);
                if let Some(port) = port {
                    connect_options = connect_options.port(port.parse()?);
                }
                if let Some(user) = user {
                    connect_options = connect_options.username(user);
                }
                if let Some(password) = password {
                    connect_options = connect_options.password(password);
                }
                if let Some(database) = database {
                    connect_options = connect_options.database(database);
                }
                DbPool::Postgres(PgPool::connect_with(connect_options).await?)
            }
        };
        self.pool.lock().await.replace(pool);
        self.current_connection.write().await.replace(connection);

        Ok(())
    }

    /// Clean up database connections.
    pub async fn close(&self) {
        if let Some(pool) = self.pool.lock().await.take() {
            match pool {
                DbPool::Sqlite(p) => p.close().await,
                DbPool::MySql(p) => p.close().await,
                DbPool::Postgres(p) => p.close().await,
            }
        }
    }

    /// Fetch all results (if any) and output them.
    pub async fn fetch_all_with_output(&self, query: &str, logger: &Logger) -> anyhow::Result<()> {
        let rows = self.fetch_all(query).await?;
        Self::print_table(logger, &rows)?;
        Ok(())
    }

    pub async fn get_current_connection(&self) -> RwLockReadGuard<Option<Connection>> {
        self.current_connection.read().await
    }

    /// Fetch all results (if any).
    async fn fetch_all(&self, query: &str) -> anyhow::Result<Vec<DbRow>> {
        let lock = self.pool.lock().await;
        let pool = lock
            .deref()
            .as_ref()
            .ok_or(anyhow!("not connected to any database"))?;

        match pool {
            DbPool::Sqlite(p) => Ok(sqlx::query_as(query).fetch_all(p).await?),
            DbPool::MySql(p) => Ok(sqlx::query_as(query).fetch_all(p).await?),
            DbPool::Postgres(p) => Ok(sqlx::query_as(query).fetch_all(p).await?),
        }
    }

    // Print a list of rows using the logger.
    fn print_table(logger: &Logger, rows: &[DbRow]) -> anyhow::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }

        let column_names = rows[0]
            .columns
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();

        let mut column_widths: Vec<usize> = column_names.iter().map(|col| col.len()).collect();

        for row in rows {
            for (i, (_, value)) in row.columns.iter().enumerate() {
                column_widths[i] = column_widths[i].max(value.len());
            }
        }

        // Helper function to create a row string
        let make_row = |values: Vec<&str>| -> String {
            values
                .into_iter()
                .enumerate()
                .map(|(i, value)| format!(" {:<width$} ", value, width = column_widths[i]))
                .collect::<Vec<_>>()
                .join("|")
        };

        let separator: String = column_widths
            .iter()
            .map(|&width| format!("{:-<width$}", "", width = width + 2))
            .collect::<Vec<_>>()
            .join("+");

        let mut lines: Vec<String> = Vec::new();

        let header = make_row(column_names);
        lines.push(header);
        lines.push(separator);

        for row in rows {
            let line = make_row(
                row.columns
                    .iter()
                    .map(|(_, value)| value.as_str())
                    .collect(),
            );
            lines.push(line);
        }

        logger.standard(&lines.join("\n"))?;
        Ok(())
    }
}
