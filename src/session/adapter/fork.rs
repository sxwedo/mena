//! Explicit, bounded native fork requests. Never starts a model turn.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};

use super::super::{
    NativeResumeCommand, NativeSessionFork, read_json_file, validate_storage_identifier,
};

pub(super) fn codex(id: &str, point: &str) -> Result<NativeSessionFork> {
    validate_storage_identifier(id, "session ID")?;
    validate_storage_identifier(point, "turn ID")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), codex_async(id, point))
            .await
            .context("native fork timed out; reopen the session list before retrying")?
    })
}

async fn codex_async(id: &str, point: &str) -> Result<NativeSessionFork> {
    // Older servers may ignore unknown fields. Check the installed protocol
    // before sending lastTurnId, so an old binary cannot copy later turns.
    let directory = tempfile::tempdir()?;
    let executable = crate::process::find_executable("codex")
        .context("Codex is not installed or cannot be located")?;
    let status = Command::new(&executable)
        .args(["app-server", "generate-json-schema", "--out"])
        .arg(directory.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .context("failed to check Codex fork support")?;
    let schema = read_json_file(&directory.path().join("v2/ThreadForkParams.json"))?;
    if !status.success()
        || schema
            .as_ref()
            .and_then(|value| value.pointer("/properties/lastTurnId"))
            .is_none()
    {
        bail!("installed Codex cannot fork at a selected turn; create a context fork instead");
    }
    let mut child = Command::new(&executable)
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("failed to start Codex fork service")?;
    let result = async {
        let mut input = child.stdin.take().context("missing Codex stdin")?;
        let mut output = BufReader::new(child.stdout.take().context("missing Codex stdout")?);
        send(
            &mut input,
            json!({"id": 1, "method": "initialize", "params": {
                "clientInfo": {"name": "mena", "version": env!("CARGO_PKG_VERSION")}
            }}),
        )
        .await?;
        reply(&mut output, 1).await?;
        send(&mut input, json!({"method": "initialized"})).await?;
        send(
            &mut input,
            json!({"id": 2, "method": "thread/fork", "params": {
                "threadId": id, "lastTurnId": point, "excludeTurns": true
            }}),
        )
        .await?;
        let response = reply(&mut output, 2).await?;
        let fork = response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .context("Codex fork response has no thread ID")?;
        validate_storage_identifier(fork, "fork ID")?;
        if fork == id {
            bail!("Codex did not create an independent fork");
        }
        Ok(NativeSessionFork {
            session_id: fork.to_owned(),
            command: NativeResumeCommand {
                program: executable.to_string_lossy().into_owned(),
                args: vec!["resume".to_owned(), fork.to_owned()],
            },
        })
    }
    .await;
    let _ = child.kill().await;
    result
}

async fn send(input: &mut ChildStdin, value: Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    input.write_all(&bytes).await?;
    input.flush().await?;
    Ok(())
}

async fn reply<R: tokio::io::AsyncBufRead + Unpin>(output: &mut R, id: u64) -> Result<Value> {
    const MAX_BYTES: usize = 4 * 1024 * 1024;
    let mut bytes = 0;
    for _ in 0..512 {
        let mut line = String::new();
        (&mut *output)
            .take((MAX_BYTES + 1) as u64)
            .read_line(&mut line)
            .await?;
        bytes += line.len();
        if line.is_empty() || bytes > MAX_BYTES {
            bail!("Codex fork response ended or exceeded the read limit");
        }
        let value: Value = serde_json::from_str(&line).context("invalid Codex fork response")?;
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if value.get("error").is_some() {
            bail!("Codex rejected the native fork; create a context fork instead");
        }
        return value
            .get("result")
            .cloned()
            .context("missing Codex fork result");
    }
    bail!("Codex fork exceeded the response limit")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_reply_is_correlated_and_bounded() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let bytes = b"{\"method\":\"notice\"}\n{\"id\":1,\"result\":{}}\n{\"id\":2,\"result\":{\"thread\":{\"id\":\"child\"}}}\n";
            let mut reader = BufReader::new(&bytes[..]);
            assert_eq!(reply(&mut reader, 2).await.expect("response")["thread"]["id"], "child");
            let bytes = vec![b'x'; 4 * 1024 * 1024 + 2];
            let mut reader = BufReader::new(bytes.as_slice());
            assert!(reply(&mut reader, 2).await.is_err());
            let mut reader = BufReader::new(&b"{\"id\":2,\"error\":{\"message\":\"secret\"}}\n"[..]);
            let error = reply(&mut reader, 2).await.expect_err("rejected fork").to_string();
            assert!(!error.contains("secret"));
        });
    }
}
