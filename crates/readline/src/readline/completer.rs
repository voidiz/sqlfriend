use std::io;

use rustyline::completion::{Completer, Pair};

use sqlfriend_core::lsp::completer::{CandidatePair, LspCompleter};
use tokio::{runtime, task};

// Doesn't look like this can be pub(crate) due to the rustyline Completer macro
pub struct ReadlineCompleter {
    lsp_completer: LspCompleter,
}

impl ReadlineCompleter {
    pub fn new(lsp_completer: LspCompleter) -> Self {
        Self { lsp_completer }
    }
}

impl Completer for ReadlineCompleter {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let (pos, pairs) = task::block_in_place(move || {
            runtime::Handle::current().block_on(async move {
                self.lsp_completer
                    .complete_with_logging(line, pos)
                    .await
                    .map_err(|e| io::Error::other(e.to_string()))
            })
        })?;
        Ok((pos, to_rustyline_pairs(pairs)))
    }
}

/// Convert internal completion pairs to rustyline completion pairs.
fn to_rustyline_pairs(pairs: Vec<CandidatePair>) -> Vec<Pair> {
    pairs
        .into_iter()
        .map(|pair| Pair {
            display: pair.display,
            replacement: pair.replacement,
        })
        .collect()
}
