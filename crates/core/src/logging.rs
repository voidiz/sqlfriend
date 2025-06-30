use std::fmt::Display;

use tokio::sync::mpsc;

/// Lower discriminant (higher up in the enum declaration) implies a lower
/// logging level. Messages for all verbosity levels less or equal to the set level
/// should be printed (see [`Self::should_print()`]).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verbosity {
    Error,
    Warn,
    Standard,
    Debug,
}

impl Verbosity {
    /// Returns true if the given verbosity should be printed given self as the set verbosity
    /// level.
    pub fn should_print(&self, verbosity: &Verbosity) -> bool {
        verbosity <= self
    }
}

impl Display for Verbosity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Standard => Ok(()),
            Self::Warn => write!(f, "WARN:"),
            Self::Debug => write!(f, "DEBUG:"),
            Self::Error => write!(f, "ERROR:"),
        }
    }
}

/// PrintPayload represents the messages that can be sent from a logger.
#[derive(Debug)]
pub enum PrintPayload {
    // TODO: Implement changing of verbosity (through config?)
    #[allow(dead_code)]
    SetVerbosity(Verbosity),
    Output(Verbosity, String),
}

/// Logger is used to asynchronously pass messages that should be output by Printer.
#[derive(Clone)]
pub struct Logger {
    log_tx: mpsc::UnboundedSender<PrintPayload>,
}

impl Logger {
    pub fn new(log_tx: mpsc::UnboundedSender<PrintPayload>) -> Self {
        Self { log_tx }
    }

    /// Output with standard verbosity.
    pub fn standard(&self, msg: &str) -> anyhow::Result<()> {
        self.log_tx
            .send(PrintPayload::Output(Verbosity::Standard, msg.to_string()))?;

        Ok(())
    }

    /// Output with error verbosity.
    pub fn error(&self, msg: &str) -> anyhow::Result<()> {
        self.log_tx
            .send(PrintPayload::Output(Verbosity::Error, msg.to_string()))?;

        Ok(())
    }

    /// Output with warn verbosity.
    pub fn warn(&self, msg: &str) -> anyhow::Result<()> {
        self.log_tx
            .send(PrintPayload::Output(Verbosity::Warn, msg.to_string()))?;

        Ok(())
    }

    /// Output with debug verbosity.
    pub fn debug(&self, msg: &str) -> anyhow::Result<()> {
        self.log_tx
            .send(PrintPayload::Output(Verbosity::Debug, msg.to_string()))?;

        Ok(())
    }
}
