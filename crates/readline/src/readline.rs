use sqlfriend_core::{
    command::{handle_command, is_maybe_command},
    db_client::DbClient,
    lsp::{client::LspClient, completer::LspCompleter},
    task::TaskController,
};

use anyhow::bail;
use completer::ReadlineCompleter;
use rustyline::{
    error::ReadlineError, highlight::MatchingBracketHighlighter, hint::HistoryHinter,
    history::FileHistory, Editor,
};
use rustyline_derive::{Completer, Helper, Highlighter, Hinter, Validator};
use validator::ReadlineValidator;

mod completer;
mod validator;

#[derive(Helper, Completer, Highlighter, Hinter, Validator)]
pub struct ReadlineHelper {
    #[rustyline(Completer)]
    completer: ReadlineCompleter,
    #[rustyline(Highlighter)]
    highlighter: MatchingBracketHighlighter,
    #[rustyline(Validator)]
    validator: ReadlineValidator,
    #[rustyline(Hinter)]
    hinter: HistoryHinter,
}

impl ReadlineHelper {
    pub fn new(lsp_client: &LspClient) -> Self {
        ReadlineHelper {
            completer: ReadlineCompleter::new(LspCompleter::new(lsp_client.clone())),
            highlighter: MatchingBracketHighlighter::new(),
            hinter: HistoryHinter::new(),
            validator: ReadlineValidator::default(),
        }
    }
}

/// Create task for REPL.
pub async fn init_repl(
    mut rl: Editor<ReadlineHelper, FileHistory>,
    task_controller: TaskController,
    lsp_client: LspClient,
    db_client: DbClient,
) -> anyhow::Result<()> {
    let helper = ReadlineHelper::new(&lsp_client);

    rl.set_helper(Some(helper));
    loop {
        let prompt = get_prompt(&db_client).await;
        match rl.readline(&prompt) {
            Ok(line) => {
                rl.add_history_entry(line.as_str())?;
                if let Err(e) = handle_line(&task_controller, &db_client, &lsp_client, &line).await
                {
                    lsp_client.get_logger().error(&e.to_string())?;
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break;
            }
            Err(err) => {
                bail!(err);
            }
        }
    }

    Ok(())
}

async fn get_prompt(db_client: &DbClient) -> String {
    let connection = db_client.get_current_connection().await;
    let name = match &*connection {
        Some(connection) => connection.name.clone(),
        None => "sqlfriend".to_string(),
    };

    format!("{name}> ")
}

async fn handle_line(
    task_controller: &TaskController,
    db_client: &DbClient,
    lsp_client: &LspClient,
    line: &str,
) -> anyhow::Result<()> {
    if is_maybe_command(line) {
        handle_command(task_controller, db_client, lsp_client, line).await?;
    } else {
        db_client
            .fetch_all_with_output(line, lsp_client.get_logger())
            .await?;
    }

    Ok(())
}
