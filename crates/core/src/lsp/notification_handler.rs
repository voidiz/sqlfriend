use std::future::Future;

use ariadne::{Label, Report, ReportKind, Source};
use jsonrpsee_types::Notification;
use lsp_types::{Diagnostic, DiagnosticSeverity, PublishDiagnosticsParams};
use tokio::sync::broadcast;

use crate::{logging::Logger, state::State};

pub enum HandlerType {
    Logger,
}

/// NotificationHandler displays notifications received from the LSP server.
pub struct NotificationHandler {
    state: State,
    logger: Logger,

    /// Used to receive LSP notifications from the LSP server.
    notif_rx: broadcast::Receiver<Vec<u8>>,
}

impl NotificationHandler {
    pub fn new(state: State, logger: Logger, notif_rx: broadcast::Receiver<Vec<u8>>) -> Self {
        NotificationHandler {
            state,
            logger,
            notif_rx,
        }
    }

    pub fn init(self, handler_type: HandlerType) -> impl Future<Output = anyhow::Result<()>> {
        match handler_type {
            HandlerType::Logger => self.init_logger(),
        }
    }

    /// Forward notifications to logger.
    async fn init_logger(mut self) -> anyhow::Result<()> {
        loop {
            let body = self.notif_rx.recv().await?;

            // TODO: Improve this logic to handle different types of notifications
            if let Ok(notification) =
                serde_json::from_slice::<Notification<PublishDiagnosticsParams>>(&body)
            {
                let text = self.state.lsp_text.lock().await;
                let msg = handle_diagnostics(&text, &notification)?;
                if !msg.is_empty() {
                    self.logger.standard(&msg)?;
                }
            } else {
                self.logger.debug(&format!(
                    "unsupported notification: {}",
                    String::from_utf8_lossy(&body)
                ))?;
            }
        }
    }
}

fn handle_diagnostics(
    text: &str,
    notification: &Notification<PublishDiagnosticsParams>,
) -> anyhow::Result<String> {
    let report = notification
        .params
        .diagnostics
        .iter()
        .map(|diagnostic| format_diagnostic(text, diagnostic))
        .collect::<anyhow::Result<Vec<_>>>()?
        .join("\n");

    Ok(report)
}

fn format_diagnostic(text: &str, diagnostic: &Diagnostic) -> anyhow::Result<String> {
    let report_kind = if let Some(severity) = diagnostic.severity {
        match severity {
            DiagnosticSeverity::ERROR => ReportKind::Error,
            DiagnosticSeverity::WARNING | DiagnosticSeverity::INFORMATION => ReportKind::Warning,
            DiagnosticSeverity::HINT => ReportKind::Advice,
            _ => unreachable!(),
        }
    } else {
        ReportKind::Warning
    };

    let start_line: usize = diagnostic.range.start.line.try_into()?;
    let start_offset: usize = diagnostic.range.start.character.try_into()?;
    let end_line: usize = diagnostic.range.end.line.try_into()?;
    let end_offset: usize = diagnostic.range.end.character.try_into()?;
    let start_index = compute_byte_offset(text, start_line, start_offset);
    let end_index = compute_byte_offset(text, end_line, end_offset);

    let mut buffer = vec![];
    const SOURCE_ID: &str = "query";
    Report::build(report_kind, (SOURCE_ID, 0..text.len()))
        .with_label(
            Label::new((SOURCE_ID, start_index..end_index))
                .with_message(diagnostic.message.to_string()),
        )
        .finish()
        .write((SOURCE_ID, Source::from(text)), &mut buffer)?;

    let str = String::from_utf8(buffer)?;
    Ok(str)
}

/// Get byte offset of the given row and col in text. All values are zero-indexed.
fn compute_byte_offset(text: &str, row: usize, col: usize) -> usize {
    // Assuming that all line endings are the same
    let line_ending_len = if text.contains("\r\n") { "\r\n" } else { "\n" }.len();

    let offset = text
        .lines()
        .take(row + 1)
        .enumerate()
        .fold(0, |acc, (i, line)| {
            if i == row {
                // If the offset extends outside the line, make it the last character
                // (zero-indexed) instead.
                if col >= line.len() {
                    return acc + line.len() - 1;
                } else {
                    return acc + col;
                }
            }

            acc + line.len() + line_ending_len
        });

    // Return the last byte offset if the row is out of bounds.
    offset.min(text.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_compute_byte_offset_with_lf() {
        assert_eq!(compute_byte_offset("foo\nbar\nbaz", 1, 0), 4);
        assert_eq!(compute_byte_offset("foo\nbar\nbaz", 2, 2), 10);
        assert_eq!(compute_byte_offset("foo\nbar\nbaz", 7, 7), 10);
    }

    #[test]
    fn can_compute_byte_offset_with_crlf() {
        assert_eq!(compute_byte_offset("foo\r\nbar\r\nbaz", 1, 0), 5);
        assert_eq!(compute_byte_offset("foo\r\nbar\r\nbaz", 2, 2), 12);
        assert_eq!(compute_byte_offset("foo\r\nbar\r\nbaz", 7, 7), 12);
    }
}
