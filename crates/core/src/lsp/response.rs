use std::pin::Pin;

use anyhow::{anyhow, Context};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt};

/// Read an LSP response or notification body into a byte slice.
pub async fn read_body(reader: &mut Pin<Box<dyn AsyncBufRead + Send>>) -> anyhow::Result<Vec<u8>> {
    let content_length = parse_header(reader)
        .await
        .with_context(|| "failed to parse response header")?;

    let mut bytes = vec![0u8; content_length];
    reader
        .read_exact(&mut bytes)
        .await
        .with_context(|| "failed to read bytes")?;

    Ok(bytes)
}

/// Return the response length (only relevant header) and advance the reader
/// until the body
async fn parse_header(reader: &mut Pin<Box<dyn AsyncBufRead + Send>>) -> anyhow::Result<usize> {
    let mut content_length = 0;

    // Read header
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;

        if line.trim().is_empty() {
            // End of header
            break;
        }

        let (key, value) = line
            .split_once(":")
            .ok_or(anyhow!("colon missing in LSP response header"))?;

        if key.trim().to_lowercase() == "content-length" {
            content_length = value.trim().parse()?;
        }
    }

    Ok(content_length)
}
