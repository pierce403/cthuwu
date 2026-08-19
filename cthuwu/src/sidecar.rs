use crate::{
    bot::UwUBot,
    contact::normalize_inbox_id,
    deadline::{DEFAULT_PUBLIC_WORK_BUDGET, InferenceLane, scope_authenticated_deadline},
    principal::PrincipalRole,
    token_eye::Address,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    path::Path,
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{Mutex, Semaphore, mpsc, oneshot, watch},
    task::JoinSet,
    time::timeout,
};
use tracing::{error, info};

// This matches the sidecar's limit for JSONL received from Rust. It also leaves ample room for a
// 16 KiB text message after worst-case JSON string escaping plus bounded protocol metadata.
const MAX_SIDECAR_FRAME_BYTES: usize = 256 * 1024;
const MAX_REQUEST_DEADLINE_MS: u64 = 300_000;
const RESPONSE_RESERVE_MS: u64 = 1_000;
const TRANSPORT_ENVIRONMENT_ALLOWLIST: &[&str] = &[
    "XMTP_DB_DIRECTORY",
    "XMTP_GATEWAY_HOST",
    "UWUBOT_REPLY_TIMEOUT_MS",
    "CTHUWU_RPC_ENDPOINT",
    "CTHUWU_BRANDING_CONTRACT",
    "CTHUWU_GLOBAL_GROUP_ID",
    "CTHUWU_GLOBAL_ADMIN_INBOX_IDS",
    "CTHUWU_ASSIGNMENT_REVALIDATE_SECONDS",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InboundText {
    #[serde(rename = "type")]
    event_type: String,
    id: String,
    #[serde(rename = "messageId")]
    message_id: String,
    #[serde(rename = "senderInboxId")]
    sender_inbox_id: String,
    #[serde(rename = "senderAddress")]
    sender_address: Option<String>,
    #[serde(rename = "sentAtNs")]
    sent_at_ns: String,
    #[serde(rename = "deadlineUnixMs")]
    deadline_unix_ms: u64,
    #[serde(rename = "conversationId")]
    conversation_id: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct XmtpIdentityFrame {
    #[serde(rename = "type")]
    frame_type: String,
    #[serde(rename = "walletAddress")]
    wallet_address: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum OperatorIdentityFrame {
    OperatorIdentity {
        address: String,
        #[serde(rename = "inboxId")]
        inbox_id: String,
    },
    OperatorIdentityError {
        message: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SidecarResponse {
    Reply {
        id: String,
        text: String,
    },
    Ignore {
        id: String,
    },
    OperatorNotice {
        #[serde(rename = "noticeId")]
        notice_id: String,
        #[serde(rename = "inboxId")]
        inbox_id: String,
        text: String,
    },
}

#[derive(Debug)]
pub struct OperatorNotice {
    notice_id: String,
    pub inbox_id: String,
    pub text: String,
    acknowledgement: oneshot::Sender<bool>,
}

impl OperatorNotice {
    pub fn with_acknowledgement(
        inbox_id: String,
        text: String,
    ) -> Result<(Self, oneshot::Receiver<bool>)> {
        let mut entropy = [0_u8; 16];
        getrandom::fill(&mut entropy).context("generating an operator notice ID")?;
        let mut encoded = String::with_capacity(entropy.len() * 2);
        for byte in entropy {
            use std::fmt::Write as _;
            let _ = write!(encoded, "{byte:02x}");
        }
        let (acknowledgement, receiver) = oneshot::channel();
        Ok((
            Self {
                notice_id: format!("erc8004-notice:{encoded}"),
                inbox_id,
                text,
                acknowledgement,
            },
            receiver,
        ))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OperatorNoticeResult {
    #[serde(rename = "type")]
    frame_type: String,
    #[serde(rename = "noticeId")]
    notice_id: String,
    delivered: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GlobalGroupAdminFrame {
    #[serde(rename = "type")]
    frame_type: String,
    #[serde(rename = "groupId")]
    group_id: String,
    #[serde(rename = "adminInboxIds")]
    admin_inbox_ids: Vec<String>,
    created: bool,
    recovered: bool,
}

/// Runs the narrowly scoped production Global-group bootstrap/inspection path in the same
/// persistent XMTP sidecar. The sidecar owns group validation and mutation; Rust admits only its
/// one bounded, identifier-only result frame.
pub async fn manage_global_group(
    node: &Path,
    sidecar: &Path,
    data_dir: &Path,
    xmtp_environment: &str,
    action: &str,
) -> Result<String> {
    if xmtp_environment != "production" {
        bail!("Global group administration requires XMTP production");
    }
    if !matches!(action, "create" | "inspect") {
        bail!("unsupported Global group administration action");
    }
    if !sidecar.is_file() {
        bail!(
            "XMTP transport {} is missing; run `npm ci && npm run build` in agent/ or set UWUBOT_SIDECAR",
            sidecar.display()
        );
    }

    let mut command = Command::new(node);
    command
        .arg(sidecar)
        .arg("--global-group-bootstrap")
        .arg(action)
        .env_clear()
        .env("UWUBOT_DATA_DIR", data_dir)
        .env("UWUBOT_XMTP_ENV", xmtp_environment)
        .env("XMTP_ENV", xmtp_environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    copy_transport_environment(&mut command);

    let mut child = command.spawn().with_context(|| {
        format!(
            "administering the XMTP Global group with {}",
            node.display()
        )
    })?;
    #[cfg(unix)]
    let _process_group = ProcessGroupGuard::new(child.id());
    let stdout = child
        .stdout
        .take()
        .context("Global group helper stdout was not piped")?;
    let mut stdout = BufReader::new(stdout);
    let (line, trailing, status) = timeout(Duration::from_secs(120), async {
        let line = read_sidecar_frame(&mut stdout).await?;
        let trailing = read_sidecar_frame(&mut stdout).await?;
        let status = child
            .wait()
            .await
            .context("waiting for the Global group helper")?;
        Ok::<_, anyhow::Error>((line, trailing, status))
    })
    .await
    .context("Global group helper timed out")??;
    if !status.success() {
        bail!("Global group helper exited with {status}");
    }
    if trailing.is_some() {
        bail!("Global group helper emitted more than one protocol frame");
    }
    let line = line.context("Global group helper exited without a result")?;
    if line.is_empty() {
        bail!("Global group helper emitted an oversized result frame");
    }
    let frame: GlobalGroupAdminFrame =
        serde_json::from_str(&line).context("Global group helper emitted malformed JSON")?;
    ensure!(
        frame.frame_type == "cthuwu_global_group",
        "Global group helper emitted an unsupported frame"
    );
    let group_id = normalize_inbox_id(&frame.group_id)
        .context("Global group helper emitted an invalid conversation ID")?;
    ensure!(
        group_id.len() == 64,
        "Global group helper emitted a noncanonical conversation ID"
    );
    ensure!(
        !frame.admin_inbox_ids.is_empty() && frame.admin_inbox_ids.len() <= 32,
        "Global group helper emitted an invalid admin set"
    );
    let mut admins = std::collections::BTreeSet::new();
    for inbox_id in frame.admin_inbox_ids {
        let inbox_id = normalize_inbox_id(&inbox_id)
            .context("Global group helper emitted an invalid admin inbox ID")?;
        ensure!(
            inbox_id.len() == 64 && admins.insert(inbox_id),
            "Global group helper emitted a duplicate or noncanonical admin inbox ID"
        );
    }
    let valid_outcome = if action == "create" {
        frame.created ^ frame.recovered
    } else {
        !frame.created && !frame.recovered
    };
    ensure!(
        valid_outcome,
        "Global group helper result does not match the requested action"
    );
    Ok(format!(
        "Global group: {group_id}\nStatus: {}\nAuthorized Tentacle admins: {}",
        if frame.created {
            "created"
        } else if frame.recovered {
            "recovered"
        } else {
            "verified"
        },
        admins.into_iter().collect::<Vec<_>>().join(", ")
    ))
}

/// Loads or creates the same persistent XMTP identity used by the live sidecar and returns its
/// locally derived EVM address. The private key remains inside the Node identity process; only the
/// address crosses stdout in one bounded frame.
pub async fn resolve_xmtp_wallet_address(
    node: &Path,
    sidecar: &Path,
    data_dir: &Path,
    xmtp_environment: &str,
) -> Result<Address> {
    if !sidecar.is_file() {
        bail!(
            "XMTP transport {} is missing; run `npm ci && npm run build` in agent/ or set UWUBOT_SIDECAR",
            sidecar.display()
        );
    }

    let mut command = Command::new(node);
    command
        .arg(sidecar)
        .arg("--print-xmtp-wallet-address")
        .env_clear()
        .env("UWUBOT_DATA_DIR", data_dir)
        .env("UWUBOT_XMTP_ENV", xmtp_environment)
        .env("XMTP_ENV", xmtp_environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    copy_transport_environment(&mut command);

    let mut child = command
        .spawn()
        .with_context(|| format!("loading XMTP identity with {}", node.display()))?;
    #[cfg(unix)]
    let _process_group = ProcessGroupGuard::new(child.id());
    let stdout = child
        .stdout
        .take()
        .context("XMTP identity helper stdout was not piped")?;
    let mut stdout = BufReader::new(stdout);
    let (line, trailing, status) = timeout(Duration::from_secs(30), async {
        let line = read_sidecar_frame(&mut stdout).await?;
        let trailing = read_sidecar_frame(&mut stdout).await?;
        let status = child
            .wait()
            .await
            .context("waiting for XMTP identity helper")?;
        Ok::<_, anyhow::Error>((line, trailing, status))
    })
    .await
    .context("XMTP identity helper timed out")??;
    if !status.success() {
        bail!("XMTP identity helper exited with {status}");
    }
    if trailing.is_some() {
        bail!("XMTP identity helper emitted more than one protocol frame");
    }
    let line = line.context("XMTP identity helper exited without an address")?;
    if line.is_empty() {
        bail!("XMTP identity helper emitted an oversized address frame");
    }
    let frame: XmtpIdentityFrame =
        serde_json::from_str(&line).context("XMTP identity helper emitted malformed JSON")?;
    if frame.frame_type != "xmtp_identity" {
        bail!("XMTP identity helper emitted an unsupported frame");
    }
    let address: Address = frame
        .wallet_address
        .parse()
        .context("XMTP identity helper emitted an invalid EVM address")?;
    if address == Address::ZERO {
        bail!("XMTP identity helper derived the zero address");
    }
    Ok(address)
}

/// Resolves an ENS name or Ethereum address through Ethereum mainnet and the selected XMTP network.
/// Only the canonical address and inbox ID cross the short-lived helper boundary.
pub async fn resolve_operator_inbox(
    node: &Path,
    sidecar: &Path,
    operator_identity: &str,
    xmtp_environment: &str,
) -> Result<(Address, String)> {
    if !sidecar.is_file() {
        bail!(
            "XMTP transport {} is missing; run `npm ci && npm run build` in agent/ or set UWUBOT_SIDECAR",
            sidecar.display()
        );
    }

    let mut command = Command::new(node);
    command
        .arg(sidecar)
        .arg("--resolve-operator-inbox")
        .arg(operator_identity)
        .env_clear()
        .env("XMTP_ENV", xmtp_environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    copy_network_environment(&mut command);

    let mut child = command
        .spawn()
        .with_context(|| format!("resolving operator identity with {}", node.display()))?;
    #[cfg(unix)]
    let _process_group = ProcessGroupGuard::new(child.id());
    let stdout = child
        .stdout
        .take()
        .context("operator identity helper stdout was not piped")?;
    let mut stdout = BufReader::new(stdout);
    let (line, trailing, status) = timeout(Duration::from_secs(30), async {
        let line = read_sidecar_frame(&mut stdout).await?;
        let trailing = read_sidecar_frame(&mut stdout).await?;
        let status = child
            .wait()
            .await
            .context("waiting for operator identity helper")?;
        Ok::<_, anyhow::Error>((line, trailing, status))
    })
    .await
    .context("operator identity resolution timed out")??;
    if !status.success() {
        bail!("operator identity helper exited with {status}");
    }
    if trailing.is_some() {
        bail!("operator identity helper emitted more than one protocol frame");
    }
    let line = line.context("operator identity helper exited without a result")?;
    if line.is_empty() {
        bail!("operator identity helper emitted an oversized result frame");
    }
    let frame: OperatorIdentityFrame =
        serde_json::from_str(&line).context("operator identity helper emitted malformed JSON")?;
    let (address, inbox_id) = match frame {
        OperatorIdentityFrame::OperatorIdentity { address, inbox_id } => (address, inbox_id),
        OperatorIdentityFrame::OperatorIdentityError { message } => {
            if message.is_empty() || message.len() > 512 || message.contains(['\r', '\n']) {
                bail!("operator identity helper emitted an invalid error");
            }
            bail!("{message}");
        }
    };
    let address: Address = address
        .parse()
        .context("operator identity helper emitted an invalid EVM address")?;
    if address == Address::ZERO {
        bail!("operator identity helper resolved the zero address");
    }
    let inbox_id = normalize_inbox_id(&inbox_id)
        .context("operator identity helper emitted an invalid XMTP inbox ID")?;
    if inbox_id.len() != 64 {
        bail!("operator identity helper did not emit a full XMTP inbox ID");
    }
    Ok((address, inbox_id))
}

pub async fn run_xmtp_sidecar(
    bot: UwUBot,
    node: &Path,
    sidecar: &Path,
    data_dir: &Path,
    xmtp_environment: &str,
    mut lifecycle_shutdown: watch::Receiver<bool>,
    mut operator_notices: mpsc::Receiver<OperatorNotice>,
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
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    copy_transport_environment(&mut command);

    let mut child = command
        .spawn()
        .with_context(|| format!("starting XMTP transport with {}", node.display()))?;
    #[cfg(unix)]
    let _process_group = ProcessGroupGuard::new(child.id());

    let stdout = child
        .stdout
        .take()
        .context("XMTP transport stdout was not piped")?;
    let stdin = child
        .stdin
        .take()
        .context("XMTP transport stdin was not piped")?;
    let stdin = Arc::new(Mutex::new(stdin));
    let mut stdout = BufReader::new(stdout);
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    let bot = Arc::new(bot);
    // Requests in each authority lane are processed in sequential FIFO order. A public DM can progress
    // beside an operator request, while additional same-lane messages queue cleanly up to bounded capacity.
    let public_lane = Arc::new(Semaphore::new(1));
    let operator_lane = Arc::new(Semaphore::new(1));
    let mut tasks = JoinSet::new();
    let mut pending_operator_notices: HashMap<String, oneshot::Sender<bool>> = HashMap::new();
    info!(sidecar = %sidecar.display(), "XMTP transport started");

    loop {
        while let Some(result) = tasks.try_join_next() {
            if let Err(cause) = result {
                error!(error = %cause, "XMTP request worker failed");
            }
        }
        let line = tokio::select! {
            result = read_sidecar_frame(&mut stdout) => result?,
            signal = &mut shutdown => {
                signal?;
                info!("stopping XMTP transport");
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                drop(stdin);
                return stop_transport(&mut child).await;
            }
            changed = lifecycle_shutdown.changed() => {
                changed.context("autonomous lifecycle shutdown controller closed")?;
                if !*lifecycle_shutdown.borrow() {
                    continue;
                }
                info!("stopping XMTP transport after binding lifecycle shutdown");
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                drop(stdin);
                return stop_transport(&mut child).await;
            }
            notice = operator_notices.recv() => {
                let Some(notice) = notice else {
                    continue;
                };
                let OperatorNotice {
                    notice_id,
                    inbox_id,
                    text,
                    acknowledgement,
                } = notice;
                let inbox_id = match normalize_inbox_id(&inbox_id) {
                    Ok(inbox_id) => inbox_id,
                    Err(_) => {
                        let _ = acknowledgement.send(false);
                        error!("ignored an ERC-8004 notice with an invalid operator inbox");
                        continue;
                    }
                };
                if inbox_id.len() != 64 || text.is_empty() || text.len() > 16 * 1024 {
                    let _ = acknowledgement.send(false);
                    error!("ignored an invalid bounded ERC-8004 operator notice");
                    continue;
                }
                if pending_operator_notices.contains_key(&notice_id) {
                    let _ = acknowledgement.send(false);
                    error!("ignored a duplicate ERC-8004 operator notice ID");
                    continue;
                }
                pending_operator_notices.insert(notice_id.clone(), acknowledgement);
                send_response(
                    &stdin,
                    SidecarResponse::OperatorNotice {
                        notice_id,
                        inbox_id,
                        text,
                    },
                )
                .await?;
                continue;
            }
        };

        let Some(line) = line else {
            let shutdown_requested = timeout(Duration::from_millis(250), &mut shutdown)
                .await
                .is_ok();
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            let status = timeout(Duration::from_secs(5), child.wait())
                .await
                .context("XMTP transport did not exit after closing stdout")?
                .context("waiting for XMTP transport")?;
            if status.success() && shutdown_requested {
                return Ok(());
            }
            bail!("XMTP transport exited with {status}");
        };

        if line.is_empty() {
            error!("ignored an oversized XMTP transport frame");
            continue;
        }
        if let Ok(result) = serde_json::from_str::<OperatorNoticeResult>(&line) {
            if result.frame_type != "operator_notice_result"
                || result.notice_id.is_empty()
                || result.notice_id.len() > 128
                || !result.notice_id.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, ':' | '_' | '-')
                })
            {
                error!("ignored an invalid ERC-8004 operator notice acknowledgement");
                continue;
            }
            if let Some(acknowledgement) = pending_operator_notices.remove(&result.notice_id) {
                let _ = acknowledgement.send(result.delivered);
            } else {
                error!("ignored an unknown ERC-8004 operator notice acknowledgement");
            }
            continue;
        }
        let request: InboundText = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(_) => {
                error!("ignored malformed JSONL from the XMTP transport");
                continue;
            }
        };
        if validate_request(&request).is_err() {
            error!("ignored invalid XMTP transport metadata");
            continue;
        }
        let imprint = bot.classify_or_imprint_operator(
            &request.sender_inbox_id,
            request.sender_address.as_deref(),
            &request.sent_at_ns,
        )?;
        let role = imprint.role;
        if let Some(address) = imprint.imprinted_address {
            info!("Tentacle has imprinted on {address}");
        }
        let first_delivery =
            bot.claim_authenticated_message(&request.message_id, &request.sender_inbox_id)?;
        if request.event_type == "reject_inbound" {
            let response = reject_inbound_response(request.id, first_delivery);
            send_response(&stdin, response).await?;
            continue;
        }
        if request.event_type == "reject_oversized" {
            let response = reject_oversized_response(request.id, first_delivery, role);
            send_response(&stdin, response).await?;
            continue;
        }
        if !first_delivery {
            send_response(&stdin, SidecarResponse::Ignore { id: request.id }).await?;
            continue;
        }
        info!(
            role = ?role,
            message_bytes = request.text.len(),
            sender_address_available = request.sender_address.is_some(),
            "received new authenticated XMTP direct message"
        );
        let lane = match role {
            PrincipalRole::User => public_lane.clone(),
            PrincipalRole::Operator
            | PrincipalRole::StaleOperator
            | PrincipalRole::RevokedOperator => operator_lane.clone(),
        };
        const MAX_PENDING_TASKS: usize = 64;
        if tasks.len() >= MAX_PENDING_TASKS {
            let response = reject_inbound_response(request.id, true);
            send_response(&stdin, response).await?;
            continue;
        }
        let bot = bot.clone();
        let stdin = stdin.clone();
        tasks.spawn(async move {
            let _permit = match lane.acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => {
                    error!("authority lane semaphore closed unexpectedly");
                    return;
                }
            };
            let inference_lane = match role {
                PrincipalRole::User => InferenceLane::Public,
                PrincipalRole::Operator
                | PrincipalRole::StaleOperator
                | PrincipalRole::RevokedOperator => InferenceLane::Operator,
            };
            info!(
                lane = inference_lane.as_str(),
                "thinking about XMTP message"
            );
            let response = match processing_budget(request.deadline_unix_ms, role) {
                Some(budget) => match timeout(budget, async {
                    scope_authenticated_deadline(inference_lane, budget, async {
                        bot.receive_authenticated_claimed_with_address(
                            &request.message_id,
                            &request.sender_inbox_id,
                            request.sender_address.as_deref(),
                            &request.sent_at_ns,
                            &request.text,
                            role,
                        )
                        .await
                    })
                    .await?
                })
                .await
                {
                    Ok(Ok(Some(text))) => SidecarResponse::Reply {
                        id: request.id.clone(),
                        text,
                    },
                    Ok(Ok(None)) => SidecarResponse::Ignore {
                        id: request.id.clone(),
                    },
                    Ok(Err(cause)) => {
                        error!(
                            request_id = %request.id,
                            conversation_id = %request.conversation_id,
                            error = %cause,
                            "could not process inbound XMTP message"
                        );
                        SidecarResponse::Reply {
                            id: request.id.clone(),
                            text: failure_response(role),
                        }
                    }
                    Err(_) => {
                        error!(
                            request_id = %request.id,
                            conversation_id = %request.conversation_id,
                            lane = inference_lane.as_str(),
                            phase = "authenticated_route",
                            "cancelled XMTP work at its authenticated request deadline"
                        );
                        SidecarResponse::Reply {
                            id: request.id.clone(),
                            text: deadline_response(role),
                        }
                    }
                },
                None => {
                    error!(
                        request_id = %request.id,
                        conversation_id = %request.conversation_id,
                        lane = inference_lane.as_str(),
                        phase = "authenticated_route_admission",
                        "rejected XMTP work without enough authenticated request budget"
                    );
                    SidecarResponse::Reply {
                        id: request.id.clone(),
                        text: deadline_response(role),
                    }
                }
            };
            let response_kind = match &response {
                SidecarResponse::Reply { .. } => "reply",
                SidecarResponse::Ignore { .. } => "ignore",
                SidecarResponse::OperatorNotice { .. } => "operator_notice",
            };
            info!(
                lane = inference_lane.as_str(),
                response = response_kind,
                "finished XMTP message processing"
            );
            if let Err(cause) = send_response(&stdin, response).await {
                error!(error = %cause, "could not write XMTP sidecar response");
            }
        });
    }
}

fn reject_inbound_response(id: String, first_delivery: bool) -> SidecarResponse {
    if first_delivery {
        SidecarResponse::Reply {
            id,
            text: "CTHUWU IS ALREADY PROCESSING THE MAXIMUM PENDING WORK. RETRY WITH A NEW MESSAGE AFTER A REPLY ARRIVES."
                .to_owned(),
        }
    } else {
        SidecarResponse::Ignore { id }
    }
}

fn reject_oversized_response(
    id: String,
    first_delivery: bool,
    role: PrincipalRole,
) -> SidecarResponse {
    if !first_delivery {
        return SidecarResponse::Ignore { id };
    }
    let text = match role {
        PrincipalRole::User => {
            "that message is too big for this lil XMTP mouth, fwiend. send a shorter one and i'll listen uwu."
        }
        PrincipalRole::Operator => {
            "YOUR MESSAGE EXCEEDED THE OPERATOR INPUT BOUND. I DISPATCHED NO MODEL OR TOOL, OPERATOR. SEND A SHORTER NEW MESSAGE."
        }
        PrincipalRole::StaleOperator => {
            "THIS OVERSIZED MESSAGE PREDATES THE LOCAL OPERATOR AUTHORIZATION BOUNDARY. NO MODEL OR TOOL WAS DISPATCHED; SEND A SHORTER NEW MESSAGE."
        }
        PrincipalRole::RevokedOperator => {
            "THIS OVERSIZED MESSAGE WAS REJECTED. THIS INBOX REMAINS REVOKED, AND NO MODEL OR TOOL WAS DISPATCHED."
        }
    };
    SidecarResponse::Reply {
        id,
        text: text.to_owned(),
    }
}

fn current_unix_ms() -> Result<u64> {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis();
    u64::try_from(milliseconds).context("system clock exceeds the supported range")
}

fn processing_budget(deadline_unix_ms: u64, role: PrincipalRole) -> Option<Duration> {
    processing_budget_at(deadline_unix_ms, current_unix_ms().ok()?, role)
}

fn processing_budget_at(
    deadline_unix_ms: u64,
    now_unix_ms: u64,
    role: PrincipalRole,
) -> Option<Duration> {
    let remaining = deadline_unix_ms.checked_sub(now_unix_ms)?;
    if remaining <= RESPONSE_RESERVE_MS || remaining > MAX_REQUEST_DEADLINE_MS {
        return None;
    }
    let authenticated_budget = Duration::from_millis(remaining - RESPONSE_RESERVE_MS);
    Some(match role {
        PrincipalRole::User => authenticated_budget.min(DEFAULT_PUBLIC_WORK_BUDGET),
        PrincipalRole::Operator | PrincipalRole::StaleOperator | PrincipalRole::RevokedOperator => {
            authenticated_budget
        }
    })
}

fn failure_response(role: PrincipalRole) -> String {
    match role {
        PrincipalRole::Operator => "THE PRIVILEGED DREAM-CURRENT FAILED. I DID NOT COMPLETE YOUR REQUEST, OPERATOR."
            .to_owned(),
        PrincipalRole::StaleOperator | PrincipalRole::RevokedOperator => {
            "THIS PRIVILEGED INBOX COULD NOT PROCESS THE MESSAGE. NO TOOL AUTHORITY WAS CHANGED."
                .to_owned()
        }
        PrincipalRole::User => "the dream-current tangled before i could remember that safely. please try again in a moment."
            .to_owned(),
    }
}

fn deadline_response(role: PrincipalRole) -> String {
    match role {
        PrincipalRole::Operator => "THE REQUEST DEADLINE CLOSED ITS JAWS. I DID NOT COMPLETE THE REQUEST. WORK MAY NOT HAVE STARTED; IF IT DID, ONE OR MORE TOOLS MAY HAVE MADE PARTIAL CHANGES. VERIFY STATE BEFORE RETRYING."
            .to_owned(),
        PrincipalRole::StaleOperator | PrincipalRole::RevokedOperator => {
            "THE REQUEST DEADLINE EXPIRED. NO TOOL AUTHORITY WAS CHANGED.".to_owned()
        }
        PrincipalRole::User => "the dream-current took too long, lil star, so i stopped that reply safely. please try again uwu."
            .to_owned(),
    }
}

async fn send_response(stdin: &Arc<Mutex<ChildStdin>>, response: SidecarResponse) -> Result<()> {
    let mut encoded = serde_json::to_vec(&response).context("encoding XMTP reply")?;
    encoded.push(b'\n');
    let mut stdin = stdin.lock().await;
    stdin
        .write_all(&encoded)
        .await
        .context("writing reply to XMTP transport")?;
    stdin.flush().await.context("flushing XMTP reply")
}

async fn read_sidecar_frame<R>(reader: &mut R) -> Result<Option<String>>
where
    R: AsyncBufRead + Unpin,
{
    let mut frame = Vec::new();
    let mut oversized = false;

    loop {
        let buffer = reader
            .fill_buf()
            .await
            .context("reading XMTP transport output")?;
        if buffer.is_empty() {
            if frame.is_empty() {
                return Ok(None);
            }
            break;
        }

        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let content_length = newline.unwrap_or(buffer.len());
        if !oversized
            && frame
                .len()
                .checked_add(content_length)
                .is_none_or(|length| length > MAX_SIDECAR_FRAME_BYTES)
        {
            oversized = true;
            frame.clear();
        }

        if !oversized {
            frame.extend_from_slice(&buffer[..content_length]);
        }
        reader.consume(content_length + usize::from(newline.is_some()));
        if newline.is_some() {
            if frame.last() == Some(&b'\r') {
                frame.pop();
            }
            break;
        }
    }

    if oversized {
        return Ok(Some(String::new()));
    }

    String::from_utf8(frame)
        .context("XMTP transport emitted non-UTF-8 JSONL on stdout")
        .map(Some)
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

#[cfg(unix)]
struct ProcessGroupGuard {
    process_id: Option<i32>,
}

#[cfg(unix)]
impl ProcessGroupGuard {
    fn new(process_id: Option<u32>) -> Self {
        Self {
            process_id: process_id.and_then(|value| i32::try_from(value).ok()),
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if let Some(process_id) = self.process_id {
            // The Node sidecar is its process-group leader. Binding shutdown is complete only when
            // every descendant is gone, including helpers forked by the direct Node process.
            unsafe {
                libc::kill(-process_id, libc::SIGKILL);
            }
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
    // boundary. The persistent wallet and database keys are read by Node from the owner-only
    // identity file and are never inherited from or copied through Rust's environment.
    copy_network_environment(command);
    for name in TRANSPORT_ENVIRONMENT_ALLOWLIST {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
}

fn copy_network_environment(command: &mut Command) {
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
    ] {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
}

fn validate_request(request: &InboundText) -> Result<()> {
    if !matches!(
        request.event_type.as_str(),
        "inbound_text" | "reject_inbound" | "reject_oversized"
    ) {
        bail!("unsupported XMTP transport event");
    }
    if request.event_type != "inbound_text" && !request.text.is_empty() {
        bail!("XMTP rejection control frames must not contain message text");
    }
    if request.id.is_empty() || request.id.len() > 128 {
        bail!("invalid XMTP transport request ID");
    }
    if request.message_id.is_empty() || request.message_id.len() > 512 {
        bail!("invalid XMTP message ID");
    }
    normalize_inbox_id(&request.sender_inbox_id).context("invalid XMTP sender inbox ID")?;
    if let Some(address) = &request.sender_address
        && (address.len() != 42
            || !address.starts_with("0x")
            || !address[2..].bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        bail!("invalid authenticated XMTP sender address");
    }
    if request.sent_at_ns.is_empty()
        || request.sent_at_ns.len() > 32
        || !request.sent_at_ns.bytes().all(|byte| byte.is_ascii_digit())
        || request.sent_at_ns.parse::<u128>().is_err()
    {
        bail!("invalid XMTP sentAtNs metadata");
    }
    let now = current_unix_ms()?;
    if request.deadline_unix_ms <= now
        || request.deadline_unix_ms.saturating_sub(now) > MAX_REQUEST_DEADLINE_MS
    {
        bail!("invalid or expired XMTP request deadline");
    }
    if request.conversation_id.is_empty() || request.conversation_id.len() > 512 {
        bail!("invalid XMTP conversation ID");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn identity_helper_returns_the_xmtp_derived_wallet_address() {
        let directory = tempfile::tempdir().unwrap();
        let helper = directory.path().join("identity-helper.sh");
        std::fs::write(
            &helper,
            r#"#!/bin/sh
test "$1" = "--print-xmtp-wallet-address" || exit 2
printf '%s\n' '{"type":"xmtp_identity","walletAddress":"0x4200000000000000000000000000000000000006"}'
"#,
        )
        .unwrap();

        let address = resolve_xmtp_wallet_address(
            Path::new("/bin/sh"),
            &helper,
            directory.path(),
            "production",
        )
        .await
        .unwrap();
        assert_eq!(
            address,
            "0x4200000000000000000000000000000000000006"
                .parse()
                .unwrap()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn operator_identity_helper_returns_only_a_valid_address_and_full_inbox() {
        let directory = tempfile::tempdir().unwrap();
        let helper = directory.path().join("operator-helper.sh");
        std::fs::write(
            &helper,
            r#"#!/bin/sh
test "$1" = "--resolve-operator-inbox" || exit 2
test "$2" = "dean.eth" || exit 3
test "$XMTP_ENV" = "production" || exit 4
printf '%s\n' '{"type":"operator_identity","address":"0x4200000000000000000000000000000000000006","inboxId":"abababababababababababababababababababababababababababababababab"}'
"#,
        )
        .unwrap();

        let (address, inbox_id) =
            resolve_operator_inbox(Path::new("/bin/sh"), &helper, "dean.eth", "production")
                .await
                .unwrap();
        assert_eq!(
            address,
            "0x4200000000000000000000000000000000000006"
                .parse()
                .unwrap()
        );
        assert_eq!(inbox_id, "ab".repeat(32));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn completed_transport_parent_cannot_leave_descendants_running() {
        use std::os::unix::process::CommandExt;

        let directory = tempfile::tempdir().unwrap();
        let late_effect = directory.path().join("late-sidecar-effect");
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(format!(
            "(sleep 0.2; touch '{}') & exit 0",
            late_effect.display()
        ));
        command.as_std_mut().process_group(0);
        let mut child = command.spawn().unwrap();
        let guard = ProcessGroupGuard::new(child.id());
        assert!(child.wait().await.unwrap().success());
        drop(guard);
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert!(!late_effect.exists());
    }

    #[test]
    fn parses_the_transport_contract() {
        let request: InboundText = serde_json::from_str(
            &format!(r#"{{"type":"inbound_text","id":"request-1","messageId":"message-1","senderInboxId":"aabbcc","senderAddress":"0x4200000000000000000000000000000000000006","sentAtNs":"1750000000000000000","deadlineUnixMs":{},"conversationId":"dm-1","text":"hello"}}"#, current_unix_ms().unwrap() + 10_000),
        )
        .unwrap();
        validate_request(&request).unwrap();
        assert_eq!(request.message_id, "message-1");

        let response = SidecarResponse::Reply {
            id: request.id.clone(),
            text: "hewwo".to_owned(),
        };
        assert_eq!(
            serde_json::to_string(&response).unwrap(),
            r#"{"type":"reply","id":"request-1","text":"hewwo"}"#
        );
    }

    #[tokio::test]
    async fn operator_notice_uses_a_typed_delivery_acknowledgement() {
        let (notice, acknowledgement) = OperatorNotice::with_acknowledgement(
            "ab".repeat(32),
            "registration complete".to_owned(),
        )
        .unwrap();
        let notice_id = notice.notice_id.clone();
        let response = SidecarResponse::OperatorNotice {
            notice_id: notice.notice_id,
            inbox_id: notice.inbox_id,
            text: notice.text,
        };
        let encoded = serde_json::to_value(response).unwrap();
        assert_eq!(encoded["type"], "operator_notice");
        assert_eq!(encoded["noticeId"], notice_id);
        let result: OperatorNoticeResult = serde_json::from_value(serde_json::json!({
            "type": "operator_notice_result",
            "noticeId": notice_id,
            "delivered": true,
        }))
        .unwrap();
        assert!(result.delivered);
        notice.acknowledgement.send(result.delivered).unwrap();
        assert!(acknowledgement.await.unwrap());
    }

    #[test]
    fn overload_rejection_replies_once_and_ignores_duplicates() {
        let first = reject_inbound_response("request-first".to_owned(), true);
        assert!(matches!(
            first,
            SidecarResponse::Reply { ref id, ref text }
                if id == "request-first" && text.contains("NEW MESSAGE")
        ));
        let duplicate = reject_inbound_response("request-duplicate".to_owned(), false);
        assert!(matches!(
            duplicate,
            SidecarResponse::Ignore { ref id } if id == "request-duplicate"
        ));
    }

    #[test]
    fn oversized_rejection_is_role_specific_deduplicated_and_text_free() {
        let user = reject_oversized_response("request-user".to_owned(), true, PrincipalRole::User);
        assert!(matches!(
            user,
            SidecarResponse::Reply { ref text, .. }
                if text.contains("fwiend") && !text.contains("TOOL")
        ));
        let operator =
            reject_oversized_response("request-operator".to_owned(), true, PrincipalRole::Operator);
        assert!(matches!(
            operator,
            SidecarResponse::Reply { ref text, .. }
                if text == &text.to_uppercase() && text.contains("NO MODEL OR TOOL")
        ));
        assert!(matches!(
            reject_oversized_response(
                "request-duplicate".to_owned(),
                false,
                PrincipalRole::Operator,
            ),
            SidecarResponse::Ignore { .. }
        ));

        let mut request = InboundText {
            event_type: "reject_oversized".to_owned(),
            id: "request-oversized".to_owned(),
            message_id: "message-oversized".to_owned(),
            sender_inbox_id: "aabbcc".to_owned(),
            sender_address: Some("0x4200000000000000000000000000000000000006".to_owned()),
            sent_at_ns: "1750000000000000000".to_owned(),
            deadline_unix_ms: current_unix_ms().unwrap() + 10_000,
            conversation_id: "dm-1".to_owned(),
            text: String::new(),
        };
        validate_request(&request).unwrap();
        request.text = "content must not cross this control frame".to_owned();
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn rejects_unknown_transport_events() {
        let request = InboundText {
            event_type: "group_message".to_owned(),
            id: "request-1".to_owned(),
            message_id: "message-1".to_owned(),
            sender_inbox_id: "aabbcc".to_owned(),
            sender_address: None,
            sent_at_ns: "1750000000000000000".to_owned(),
            deadline_unix_ms: current_unix_ms().unwrap() + 10_000,
            conversation_id: "dm-1".to_owned(),
            text: "hello".to_owned(),
        };
        assert!(validate_request(&request).is_err());
    }

    #[tokio::test]
    async fn accepts_a_bounded_frame_with_maximum_json_escaped_text() {
        let text = "\0".repeat(16 * 1024);
        let mut encoded = serde_json::to_vec(&serde_json::json!({
            "type": "inbound_text",
            "id": "request-1",
            "messageId": "message-1",
            "senderInboxId": "aabbcc",
            "sentAtNs": "1750000000000000000",
            "deadlineUnixMs": current_unix_ms().unwrap() + 10_000,
            "conversationId": "dm-1",
            "text": text,
        }))
        .unwrap();
        assert!(encoded.len() < MAX_SIDECAR_FRAME_BYTES);
        encoded.resize(MAX_SIDECAR_FRAME_BYTES, b' ');
        encoded.push(b'\n');

        let mut reader = BufReader::new(encoded.as_slice());
        let frame = read_sidecar_frame(&mut reader).await.unwrap().unwrap();
        assert_eq!(frame.len(), MAX_SIDECAR_FRAME_BYTES);
        let request: InboundText = serde_json::from_str(&frame).unwrap();
        validate_request(&request).unwrap();
        assert_eq!(request.text.len(), 16 * 1024);
    }

    #[tokio::test]
    async fn rejects_a_sidecar_frame_over_the_hard_limit() {
        let mut encoded = vec![b' '; MAX_SIDECAR_FRAME_BYTES + 1];
        encoded.push(b'\n');
        encoded.extend_from_slice(b"valid-next-frame\n");
        let mut reader = BufReader::new(encoded.as_slice());

        assert_eq!(
            read_sidecar_frame(&mut reader).await.unwrap(),
            Some(String::new())
        );
        assert_eq!(
            read_sidecar_frame(&mut reader).await.unwrap(),
            Some("valid-next-frame".to_owned())
        );
    }

    #[test]
    fn rejects_role_injection_and_malformed_authenticated_metadata() {
        let injected = format!(
            r#"{{"type":"inbound_text","id":"request-1","messageId":"message-1","senderInboxId":"aabbcc","sentAtNs":"1750000000000000000","deadlineUnixMs":{},"conversationId":"dm-1","text":"hello","role":"operator"}}"#,
            current_unix_ms().unwrap() + 10_000
        );
        assert!(serde_json::from_str::<InboundText>(&injected).is_err());

        let malformed = InboundText {
            event_type: "inbound_text".to_owned(),
            id: "request-1".to_owned(),
            message_id: "message-1".to_owned(),
            sender_inbox_id: "not-an-inbox".to_owned(),
            sender_address: Some("not-an-address".to_owned()),
            sent_at_ns: "yesterday".to_owned(),
            deadline_unix_ms: current_unix_ms().unwrap() + 10_000,
            conversation_id: "dm-1".to_owned(),
            text: "/exec true".to_owned(),
        };
        assert!(validate_request(&malformed).is_err());
    }

    #[test]
    fn request_deadline_reserves_time_for_the_transport_reply() {
        assert_eq!(
            processing_budget_at(12_000, 10_000, PrincipalRole::Operator),
            Some(Duration::from_millis(1_000))
        );
        assert_eq!(
            processing_budget_at(
                10_000 + MAX_REQUEST_DEADLINE_MS,
                10_000,
                PrincipalRole::User,
            ),
            Some(DEFAULT_PUBLIC_WORK_BUDGET)
        );
        assert_eq!(
            processing_budget_at(
                10_000 + MAX_REQUEST_DEADLINE_MS,
                10_000,
                PrincipalRole::Operator,
            ),
            Some(Duration::from_millis(
                MAX_REQUEST_DEADLINE_MS - RESPONSE_RESERVE_MS
            ))
        );
        assert_eq!(
            processing_budget_at(11_000, 10_000, PrincipalRole::User),
            None
        );
        assert_eq!(
            processing_budget_at(10_000, 10_000, PrincipalRole::Operator),
            None
        );
        assert_eq!(
            processing_budget_at(
                10_000 + MAX_REQUEST_DEADLINE_MS + 1,
                10_000,
                PrincipalRole::Operator,
            ),
            None
        );
    }

    #[test]
    fn rejects_missing_past_and_unbounded_request_deadlines() {
        let missing = r#"{"type":"inbound_text","id":"request-1","messageId":"message-1","senderInboxId":"aabbcc","sentAtNs":"1750000000000000000","conversationId":"dm-1","text":"hello"}"#;
        assert!(serde_json::from_str::<InboundText>(missing).is_err());

        let now = current_unix_ms().unwrap();
        let mut request = InboundText {
            event_type: "inbound_text".to_owned(),
            id: "request-1".to_owned(),
            message_id: "message-1".to_owned(),
            sender_inbox_id: "aabbcc".to_owned(),
            sender_address: None,
            sent_at_ns: "1750000000000000000".to_owned(),
            deadline_unix_ms: now.saturating_sub(1),
            conversation_id: "dm-1".to_owned(),
            text: "hello".to_owned(),
        };
        assert!(validate_request(&request).is_err());
        request.deadline_unix_ms = now + MAX_REQUEST_DEADLINE_MS + 10_000;
        assert!(validate_request(&request).is_err());
    }

    #[tokio::test]
    async fn authority_lane_processes_concurrent_requests_sequentially() {
        let lane = Arc::new(Semaphore::new(1));
        let order = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let mut tasks = JoinSet::new();
        for index in 0..3 {
            let lane = lane.clone();
            let order = order.clone();
            tasks.spawn(async move {
                let _permit = lane.acquire_owned().await.unwrap();
                tokio::time::sleep(Duration::from_millis(10)).await;
                order.lock().await.push(index);
            });
        }

        while tasks.join_next().await.is_some() {}
        let completed = order.lock().await.clone();
        assert_eq!(completed, vec![0, 1, 2]);
    }
}
#[test]
fn transport_environment_never_allows_persistent_private_keys() {
    assert!(!TRANSPORT_ENVIRONMENT_ALLOWLIST.contains(&"XMTP_WALLET_KEY"));
    assert!(!TRANSPORT_ENVIRONMENT_ALLOWLIST.contains(&"XMTP_DB_ENCRYPTION_KEY"));
}
