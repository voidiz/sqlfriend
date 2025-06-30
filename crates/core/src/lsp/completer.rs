use anyhow::anyhow;

use crate::{command::is_maybe_command, config::get_config, lsp::client::LspClient};

use crate::command;

pub struct LspCompleter {
    client: LspClient,
}

/// Completion candidate pair.
#[derive(Debug, Clone)]
pub struct CandidatePair {
    /// Text to display when listing alternatives.
    pub display: String,
    /// Text to insert in line.
    pub replacement: String,
}

impl LspCompleter {
    pub fn new(client: LspClient) -> Self {
        LspCompleter { client }
    }

    /// Perform completion, handling errors by logging them.
    /// Returns an error if logging failed.
    pub async fn complete_with_logging(
        &self,
        line: &str,
        pos: usize,
    ) -> anyhow::Result<(usize, Vec<CandidatePair>)> {
        let completions = self.complete(line, pos).await;

        match completions {
            Err(e) => {
                self.client.get_logger().error(&e.to_string())?;
                Ok((0, vec![]))
            }
            _ => completions,
        }
    }

    /// Perform completion.
    async fn complete(
        &self,
        line: &str,
        pos: usize,
    ) -> anyhow::Result<(usize, Vec<CandidatePair>)> {
        if is_maybe_command(line) {
            return self.complete_command(line);
        }

        if self.client.is_initialized().await {
            return self.complete_lsp(line, pos).await;
        }

        // Fall back to command
        self.complete_command(line)
    }

    /// Perform completion using LSP.
    async fn complete_lsp(
        &self,
        line: &str,
        pos: usize,
    ) -> anyhow::Result<(usize, Vec<CandidatePair>)> {
        // Need the line to at least contain an empty character
        let line = if line.is_empty() { " " } else { line };

        self.client.on_change(line).await?;
        let (row, col) = row_and_col_from_offset(line, pos).ok_or(anyhow!("pos out of bounds"))?;
        let res = self
            .client
            .request_completion(row.try_into()?, col.try_into()?)
            .await?;

        let candidates = res
            .into_iter()
            .map(|candidate| CandidatePair {
                display: candidate.clone(),
                replacement: candidate,
            })
            .collect();

        Ok((find_sql_token_start(line, pos), candidates))
    }

    /// Perform command completion.
    fn complete_command(&self, line: &str) -> anyhow::Result<(usize, Vec<CandidatePair>)> {
        let matching = command::COMMANDS
            .iter()
            .filter(|(name, _cmd)| {
                let full_cmd = command::command_prefix!().to_owned() + name;
                full_cmd.starts_with(line)
            })
            .map(|(name, _cmd)| {
                let full_cmd = command::command_prefix!().to_owned() + name;
                CandidatePair {
                    display: full_cmd.to_string(),
                    replacement: full_cmd.to_string(),
                }
            })
            .collect::<Vec<_>>();

        if !matching.is_empty() {
            return Ok((0, matching));
        }

        let delete_prefix = concat!(command::command_prefix!(), "delete ");
        if let Some(arg) = line.strip_prefix(delete_prefix) {
            return self.complete_connection_names(arg, delete_prefix.len());
        }

        let use_prefix = concat!(command::command_prefix!(), "use ");
        if let Some(arg) = line.strip_prefix(use_prefix) {
            return self.complete_connection_names(arg, use_prefix.len());
        }

        Ok((0, vec![]))
    }

    fn complete_connection_names(
        &self,
        arg: &str,
        offset: usize,
    ) -> anyhow::Result<(usize, Vec<CandidatePair>)> {
        let config = get_config()?;

        let matching = config
            .get_connections()
            .iter()
            .filter(|conn| conn.name.starts_with(arg))
            .map(|conn| CandidatePair {
                display: format!("{}: {:?}", conn.name, conn.settings),
                replacement: conn.name.clone(),
            });

        Ok((offset, matching.collect()))
    }
}

/// Find the beginning of the token at pos (even if the word only consists of an
/// empty string).
fn find_sql_token_start(line: &str, pos: usize) -> usize {
    for (i, c) in line
        .chars()
        .take(pos)
        .enumerate()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        // Space or . to separate schema from identifier
        if c.is_whitespace() || c == '.' {
            return i + 1;
        }
    }

    0
}

/// Compute the row and col based on the byte index of text.
fn row_and_col_from_offset(text: &str, offset: usize) -> Option<(usize, usize)> {
    if offset > text.len() {
        return None;
    }

    // Assuming that all line endings are the same
    let line_ending_len = if text.contains("\r\n") { "\r\n" } else { "\n" }.len();

    let mut line_start = 0;
    for (line_index, line) in text.lines().enumerate() {
        let line_end = line_start + line.len();
        if offset <= line_end {
            return Some((line_index, offset - line_start));
        }

        line_start = line_end + line_ending_len
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_find_word_start_with_empty_line() {
        assert_eq!(find_sql_token_start(" ", 0), 0);
    }

    #[test]
    fn can_find_word_start_at_end() {
        assert_eq!(find_sql_token_start("CREATE", 6), 0);
    }

    #[test]
    fn can_find_word_start_at_new_word() {
        assert_eq!(find_sql_token_start("CREATE ", 7), 7);
    }

    #[test]
    fn can_find_word_start_with_dot() {
        assert_eq!(find_sql_token_start("public.", 7), 7);
    }

    #[test]
    fn can_compute_row_and_col_with_lf() {
        assert_eq!(row_and_col_from_offset("foo\nbar\nbaz", 4), Some((1, 0)));
        assert_eq!(row_and_col_from_offset("foo\nbar\nbaz", 10), Some((2, 2)));
        assert_eq!(row_and_col_from_offset("foo\nbar\nbaz", 30), None);
    }

    #[test]
    fn can_compute_row_and_col_with_crlf() {
        assert_eq!(
            row_and_col_from_offset("foo\r\nbar\r\nbaz", 5),
            Some((1, 0))
        );
        assert_eq!(
            row_and_col_from_offset("foo\r\nbar\r\nbaz", 12),
            Some((2, 2))
        );
        assert_eq!(
            row_and_col_from_offset("foo\r\nbar\r\nbaz", 13),
            Some((2, 3))
        );
        assert_eq!(row_and_col_from_offset("foo\r\nbar\r\nbaz", 30), None);
    }
}
