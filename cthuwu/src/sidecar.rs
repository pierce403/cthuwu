use crate::bot::UwUBot;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{env, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    time::timeout,
};
use tracing::{error, info};

#[derive(Debug, Deserialize)]
struct InboundText {
    #[serde(rename = "type")]
    event_type: String,
    id: String,
    #[serde(rename = "messageId")]
    message_id: String,
    #[serde(rename = "senderInboxId")]
    sender_inbox_id: String,
    #[serde(rename = "conversationId")]
    conversation_id: String,
    text: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SidecarResponse<'a> {
    Reply { id: &'a str, text: String },
    Ignore { id: &'a str },
}

pub async fn run_xmtp_sidecar(
    bot: UwUBot,
    node: &Path,
    sidecar: &Path,
    data_dir: &Path,
    xmtp_environment: &str,
) -> Result<()> {
    if !sidecar.is_file() {
        bail!(
            "XMTP transport {} is missing; run `npm ci && npm run build` in agent/ or set UWUBOT_SIDECAR",
            sidecar.display()
        );
    }

    let mut command = Command::new(node);
    command
        .arg(sidecar)
        .env_clear()
        .env("UWUBOT_DATA_DIR", data_dir)
        .env("UWUBOT_XMTP_ENV", xmtp_environment)
        .env("XMTP_ENV", xmtp_environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    copy_transport_environment(&mut command);

    let mut child = command
        .spawn()
        .with_context(|| format!("starting XMTP transport with {}", node.display()))?;

    let stdout = child
        .stdout
        .take()
        .context("XMTP transport stdout was not piped")?;
    let mut stdin = child
        .stdin
        .take()
        .context("XMTP transport stdin was not piped")?;
    let mut lines = BufReader::new(stdout).lines();
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    info!(sidecar = %sidecar.display(), "XMTP transport started");

    loop {
        let line = tokio::select! {
            result = lines.next_line() => result.context("reading XMTP transport output")?,
            signal = &mut shutdown => {
                signal?;
                info!("stopping XMTP transport");
                drop(stdin);
                return stop_transport(&mut child).await;
            }
        };

        let Some(line) = line else {
            let shutdown_requested = timeout(Duration::from_millis(250), &mut shutdown)
                .await
                .is_ok();
            let status = timeout(Duration::from_secs(5), child.wait())
                .await
                .context("XMTP transport did not exit after closing stdout")?
                .context("waiting for XMTP transport")?;
            if status.success() && shutdown_requested {
                return Ok(());
            }
            bail!("XMTP transport exited with {status}");
        };

        let request: InboundText = serde_json::from_str(&line)
            .context("XMTP transport emitted invalid JSONL on stdout")?;
        validate_request(&request)?;

        let response = match bot
            .receive_text(&request.message_id, &request.sender_inbox_id, &request.text)
            .await
        {
            Ok(Some(text)) => SidecarResponse::Reply {
                id: &request.id,
                text,
            },
            Ok(None) => SidecarResponse::Ignore { id: &request.id },
            Err(cause) => {
                error!(
                    request_id = %request.id,
                    conversation_id = %request.conversation_id,
                    error = %cause,
                    "could not process inbound XMTP message"
                );
                SidecarResponse::Reply {
                    id: &request.id,
                    text: "the dream-current tangled before i could remember that safely. please try again in a moment."
                        .to_owned(),
                }
            }
        };

        let mut encoded = serde_json::to_vec(&response).context("encoding XMTP reply")?;
        encoded.push(b'\n');
        stdin
            .write_all(&encoded)
            .await
            .context("writing reply to XMTP transport")?;
        stdin.flush().await.context("flushing XMTP reply")?;
    }
}

async fn stop_transport(child: &mut Child) -> Result<()> {
    match timeout(Duration::from_secs(10), child.wait()).await {
        Ok(status) => {
            let status = status.context("waiting for XMTP transport shutdown")?;
            if status.success() {
                Ok(())
            } else {
                bail!("XMTP transport exited with {status} during shutdown")
            }
        }
        Err(_) => {
            child
                .start_kill()
                .context("forcing XMTP transport to stop")?;
            child.wait().await.context("reaping XMTP transport")?;
            Ok(())
        }
    }
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate()).context("listening for SIGTERM")?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.context("listening for Ctrl-C"),
            value = terminate.recv() => value.context("SIGTERM listener closed").map(|_| ()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c()
        .await
        .context("listening for Ctrl-C")
}

fn copy_transport_environment(command: &mut Command) {
    // Model credentials and other application secrets intentionally do not cross the transport
    // boundary. These are the only inherited values the Node/XMTP process may need.
    for name in [
        "PATH",
        "LD_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
        "SYSTEMROOT",
        "WINDIR",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "NODE_EXTRA_CA_CERTS",
        "XMTP_WALLET_KEY",
        "XMTP_DB_ENCRYPTION_KEY",
        "XMTP_DB_DIRECTORY",
        "XMTP_GATEWAY_HOST",
        "UWUBOT_REPLY_TIMEOUT_MS",
    ] {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
}

fn validate_request(request: &InboundText) -> Result<()> {
    if request.event_type != "inbound_text" {
        bail!("unsupported XMTP transport event {:?}", request.event_type);
    }
    if request.id.is_empty() || request.id.len() > 128 {
        bail!("invalid XMTP transport request ID");
    }
    if request.conversation_id.is_empty() || request.conversation_id.len() > 512 {
        bail!("invalid XMTP conversation ID");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_transport_contract() {
        let request: InboundText = serde_json::from_str(
            r#"{"type":"inbound_text","id":"request-1","messageId":"message-1","senderInboxId":"aabbcc","conversationId":"dm-1","text":"hello"}"#,
        )
        .unwrap();
        validate_request(&request).unwrap();
        assert_eq!(request.message_id, "message-1");

        let response = SidecarResponse::Reply {
            id: &request.id,
            text: "hewwo".to_owned(),
        };
        assert_eq!(
            serde_json::to_string(&response).unwrap(),
            r#"{"type":"reply","id":"request-1","text":"hewwo"}"#
        );
    }

    #[test]
    fn rejects_unknown_transport_events() {
        let request = InboundText {
            event_type: "group_message".to_owned(),
            id: "request-1".to_owned(),
            message_id: "message-1".to_owned(),
            sender_inbox_id: "aabbcc".to_owned(),
            conversation_id: "dm-1".to_owned(),
            text: "hello".to_owned(),
        };
        assert!(validate_request(&request).is_err());
    }
}
