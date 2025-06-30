use anyhow::anyhow;
use rustyline::ExternalPrinter;

use sqlfriend_core::logging::{PrintPayload, Verbosity};
use tokio::sync::mpsc;

/// Printer is responsible for receiving log messages (usually from a Logger) and outputting them
/// to the screen.
pub struct Printer {
    log_tx: mpsc::UnboundedSender<PrintPayload>,
    log_rx: mpsc::UnboundedReceiver<PrintPayload>,

    verbosity: Verbosity,
}

impl Printer {
    pub fn new(verbosity: Verbosity) -> Self {
        let (log_tx, log_rx) = mpsc::unbounded_channel::<PrintPayload>();
        Self {
            log_tx,
            log_rx,
            verbosity,
        }
    }

    pub async fn init(
        mut self,
        mut external_printer: impl ExternalPrinter + Send + Sync + 'static,
    ) -> anyhow::Result<()> {
        loop {
            let msg = self
                .log_rx
                .recv()
                .await
                .ok_or(anyhow!("logger channel closed"))?;

            match msg {
                PrintPayload::Output(verbosity, output) => {
                    if self.verbosity.should_print(&verbosity) {
                        if verbosity == Verbosity::Standard {
                            external_printer.print(format!("{output}\n"))?;
                        } else {
                            external_printer.print(format!("{verbosity} {output}\n"))?;
                        }
                    }
                }
                PrintPayload::SetVerbosity(verbosity) => {
                    self.verbosity = verbosity;
                }
            };
        }
    }

    pub fn get_sender(&self) -> mpsc::UnboundedSender<PrintPayload> {
        self.log_tx.clone()
    }
}
