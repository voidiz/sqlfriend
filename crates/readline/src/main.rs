use crate::printer::Printer;

use readline::init_repl;
use rustyline::{CompletionType, Config as RustylineConfig, EditMode, Editor};
use sqlfriend_core::{
    config::get_config,
    db_client::DbClient,
    logging::{Logger, Verbosity},
    lsp::{build_lsp, notification_handler::HandlerType},
    state::State,
    task::{TaskController, TaskManager},
};

mod printer;
mod readline;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    DbClient::initialize();

    let state = State::default();
    let config = get_config()?;
    let printer = Printer::new(Verbosity::Standard);
    let logger = Logger::new(printer.get_sender());

    let (lsp_client, lsp_server, notification_handler) = build_lsp(state, logger.clone());
    let db_client = DbClient::default();

    let repl_config = RustylineConfig::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Vi)
        .build();
    let mut rl = Editor::with_config(repl_config)?;

    let mut task_manager = TaskManager::new(logger.clone(), lsp_server, lsp_client.clone());
    let task_controller = TaskController::new(task_manager.get_command_tx());

    task_manager
        .set
        .spawn(printer.init(rl.create_external_printer()?));
    task_manager
        .set
        .spawn(notification_handler.init(HandlerType::Logger));

    lsp_client
        .get_logger()
        .standard("sqlfriend\nType /help for a list of commands.")?;

    // Spawn LSP and connect to previous connection
    if let Some(connection) = config.get_current_connection() {
        if let Err(e) = connection
            .connect(&task_controller, &db_client, &lsp_client)
            .await
        {
            lsp_client.get_logger().error(&format!("{e:#}"))?;
        }
    }

    // Rustyline does blocking I/O, so we need to spawn its task on a different thread to prevent
    // the async tasks from being blocked.
    task_manager.set.spawn_blocking(move || {
        let handle = tokio::runtime::Handle::current();

        // The rest of the code is async for future compatibility, so we still need to await the
        // future.
        handle.block_on(async {
            init_repl(rl, task_controller, lsp_client.clone(), db_client.clone()).await?;
            Ok(())
        })
    });

    task_manager.run().await?;

    Ok(())
}
