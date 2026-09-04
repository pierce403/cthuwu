//! Operator-authorized scheduled work. Registrations are private; Markdown is task output, not authority.
use crate::{
    operator::OperatorHarness,
    principal::OperatorStore,
    sidecar::OperatorNotice,
    storage::{ensure_private_directory, restrict_file, sync_directory},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub inbox: String,
    pub generation: u64,
    pub interval: u64,
    pub next: u64,
    pub prompt: String,
    pub state: String,
    pub result: Option<String>,
}

pub struct OperatorTasks {
    path: PathBuf,
    tasks: Mutex<Vec<Task>>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl OperatorTasks {
    pub fn open(data: &Path) -> Result<Self> {
        let root = data.join("state/agent");
        ensure_private_directory(&root)?;
        let path = root.join("tasks.json");
        if fs::symlink_metadata(&path)
            .is_ok_and(|m| m.file_type().is_symlink() || !m.is_file() || m.len() > 1024 * 1024)
        {
            bail!("invalid task store");
        }
        let mut tasks: Vec<Task> = if path.exists() {
            serde_json::from_slice(&fs::read(&path)?)?
        } else {
            Vec::new()
        };
        if tasks.len() > 100 {
            bail!("too many tasks");
        }
        for task in &mut tasks {
            if task.prompt.len() > 8000 {
                bail!("oversized task");
            }
            // A crash may leave unknown effects. Never silently replay the interrupted action.
            if task.state == "running" {
                task.state = "paused".into();
                task.result = Some(
                    "Interrupted by restart. Inspect session receipts, then use /task resume <id>."
                        .into(),
                );
            }
        }
        let store = Self {
            path,
            tasks: Mutex::new(tasks),
        };
        store.save(
            &store
                .tasks
                .lock()
                .map_err(|_| anyhow::anyhow!("task lock"))?,
        )?;
        Ok(store)
    }

    fn save(&self, tasks: &[Task]) -> Result<()> {
        let root = self.path.parent().unwrap();
        let mut file = tempfile::NamedTempFile::new_in(root)?;
        restrict_file(file.as_file(), "operator tasks")?;
        file.write_all(&serde_json::to_vec(tasks)?)?;
        file.as_file().sync_all()?;
        file.persist(&self.path).map_err(|e| e.error)?;
        sync_directory(root)
    }

    pub fn command(&self, inbox: &str, generation: u64, text: &str) -> Result<String> {
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| anyhow::anyhow!("task lock"))?;
        let mut next = tasks.clone();
        let (operation, rest) = text.trim().split_once(' ').unwrap_or((text.trim(), ""));
        if operation == "list" || operation.is_empty() {
            return Ok(next
                .iter()
                .filter(|t| t.inbox == inbox && t.generation == generation)
                .map(|t| {
                    format!(
                        "{}: {} · every {}s · next {}\n{}",
                        t.id,
                        t.state,
                        t.interval,
                        t.next,
                        t.result.as_deref().unwrap_or("No result yet.")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n"));
        }
        let output = match operation {
            "run" | "add" => {
                let (interval, prompt) = if operation == "add" {
                    let (seconds, prompt) = rest
                        .split_once(' ')
                        .context("usage: /task add <seconds> <request>")?;
                    let interval: u64 = seconds.parse()?;
                    if !(60..=31_536_000).contains(&interval) {
                        bail!("interval must be 60 seconds to one year");
                    }
                    (interval, prompt)
                } else {
                    (0, rest)
                };
                if prompt.trim().is_empty()
                    || prompt.len() > 8000
                    || prompt.trim().starts_with('/')
                    || next.len() >= 100
                {
                    bail!(
                        "task needs a natural-language request, up to 8000 bytes, and space in the 100-task store"
                    );
                }
                let mut entropy = [0u8; 8];
                getrandom::fill(&mut entropy)?;
                let id = entropy
                    .iter()
                    .map(|v| format!("{v:02x}"))
                    .collect::<String>();
                next.push(Task {
                    id: id.clone(),
                    inbox: inbox.to_owned(),
                    generation,
                    interval,
                    next: now(),
                    prompt: prompt.to_owned(),
                    state: "ready".into(),
                    result: None,
                });
                format!(
                    "Task {id} registered. Use /task list, /task remove {id}, or /task resume {id}. Long runs have a 15-minute execution budget."
                )
            }
            "steer" => {
                let (id, prompt) = rest
                    .split_once(' ')
                    .context("usage: /task steer <id> <updated request>")?;
                if prompt.trim().is_empty() || prompt.len() > 8000 || prompt.starts_with('/') {
                    bail!("provide a bounded natural-language task");
                }
                let task = next
                    .iter_mut()
                    .find(|t| t.id == id && t.inbox == inbox && t.generation == generation)
                    .context("task not found")?;
                task.prompt = prompt.into();
                task.state = "ready".into();
                task.next = now();
                format!(
                    "Task {id} steered. The current run will stop; the updated request resumes with prior receipts. Inspect uncertain effects before repeating actions."
                )
            }
            "remove" | "pause" | "resume" => {
                let index = next
                    .iter()
                    .position(|t| {
                        t.id == rest.trim() && t.inbox == inbox && t.generation == generation
                    })
                    .context("task not found for this operator authorization")?;
                if operation == "remove" {
                    next.remove(index);
                } else {
                    next[index].state = if operation == "pause" {
                        "paused"
                    } else {
                        "ready"
                    }
                    .into();
                    next[index].next = now();
                }
                format!("Task {}: {operation} committed.", rest.trim())
            }
            _ => bail!(
                "usage: /task run <request> | add <seconds> <request> | list | pause/resume/remove <id> | steer <id> <request>"
            ),
        };
        self.save(&next)?;
        *tasks = next;
        Ok(output)
    }

    fn claim(&self, operators: &OperatorStore) -> Result<Option<Task>> {
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| anyhow::anyhow!("task lock"))?;
        let mut next = tasks.clone();
        let Some(task) = next
            .iter_mut()
            .find(|t| t.state == "ready" && t.next <= now() && authorized(operators, t))
        else {
            return Ok(None);
        };
        task.state = "running".into();
        let selected = task.clone();
        self.save(&next)?;
        *tasks = next;
        Ok(Some(selected))
    }

    fn running(&self, id: &str) -> bool {
        self.tasks
            .lock()
            .is_ok_and(|tasks| tasks.iter().any(|t| t.id == id && t.state == "running"))
    }

    fn finish(&self, id: &str, result: &str, success: bool) -> Result<()> {
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| anyhow::anyhow!("task lock"))?;
        let mut next = tasks.clone();
        if let Some(task) = next.iter_mut().find(|t| t.id == id && t.state == "running") {
            task.state = if !success {
                "paused"
            } else if task.interval == 0 {
                "done"
            } else {
                "ready"
            }
            .into();
            task.next = now().saturating_add(task.interval);
            task.result = Some(result.chars().take(8000).collect());
            self.save(&next)?;
            *tasks = next;
        }
        Ok(())
    }
}

fn authorized(operators: &OperatorStore, task: &Task) -> bool {
    operators.list().any(|(id, _, status, generation)| {
        id == task.inbox && status == "active" && generation == task.generation
    })
}

pub async fn supervise(
    tasks: Arc<OperatorTasks>,
    harness: Arc<OperatorHarness>,
    operators: Arc<Mutex<OperatorStore>>,
    notices: tokio::sync::mpsc::Sender<OperatorNotice>,
) {
    loop {
        let task = operators
            .lock()
            .ok()
            .and_then(|operators| tasks.claim(&operators).ok().flatten());
        if let Some(task) = task {
            let deliver = |text: String| async {
                if let Ok((notice, ack)) =
                    OperatorNotice::with_acknowledgement(task.inbox.clone(), text)
                    && notices.send(notice).await.is_ok()
                {
                    let _ = tokio::time::timeout(Duration::from_secs(30), ack).await;
                }
            };
            if task.interval == 0 {
                deliver(format!(
                    "Starting operator task {}. Use /task pause {} to cancel; results follow here.",
                    task.id, task.id
                ))
                .await;
            }
            let cancel = async {
                loop {
                    if !tasks.running(&task.id)
                        || !operators.lock().is_ok_and(|o| authorized(&o, &task))
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            };
            let result = tokio::select! {
                result = tokio::time::timeout(Duration::from_secs(900), crate::deadline::scope_authenticated_deadline(crate::deadline::InferenceLane::Operator, Duration::from_secs(900), harness.respond_scheduled(&task.inbox, &task.prompt))) => match result {
                    Ok(Ok(Ok(outcome))) => outcome,
                    _ => ("Task interrupted or failed. Inspect the private session receipts before resuming; effects may have completed.".into(), false),
                },
                _ = cancel => ("Task cancelled or operator authority changed. Inspect receipts before retrying.".into(), false),
            };
            let _ = tasks.finish(&task.id, &result.0, result.1);
            if operators.lock().is_ok_and(|o| authorized(&o, &task))
                && !(result.1 && result.0.trim() == "[NO_UPDATE]")
            {
                deliver(format!(
                    "Task {}\n{}",
                    task.id,
                    result.0.chars().take(12000).collect::<String>()
                ))
                .await;
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tasks_are_bound_to_operator_epoch_and_pause_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let mut operators = OperatorStore::new(root.path(), "production").unwrap();
        let inbox = "a".repeat(64);
        let auth = operators.add(&inbox, "test").unwrap();
        let tasks = OperatorTasks::open(root.path()).unwrap();
        tasks
            .command(&inbox, auth.generation, "run inspect the environment")
            .unwrap();
        let task = tasks.claim(&operators).unwrap().unwrap();
        assert!(tasks.running(&task.id));
        let reopened = OperatorTasks::open(root.path()).unwrap();
        assert!(!reopened.running(&task.id));
        assert!(
            reopened
                .command(&inbox, auth.generation + 1, &format!("resume {}", task.id))
                .is_err()
        );
        assert!(reopened.claim(&operators).unwrap().is_none());
        reopened
            .command(
                &inbox,
                auth.generation,
                &format!("steer {} inspect the updated plan", task.id),
            )
            .unwrap();
        let resumed = reopened.claim(&operators).unwrap().unwrap();
        assert_eq!(resumed.prompt, "inspect the updated plan");
        reopened
            .finish(&resumed.id, "An action was interrupted", false)
            .unwrap();
        assert!(
            reopened
                .command(&inbox, auth.generation, "list")
                .unwrap()
                .contains("paused")
        );
        assert!(reopened.claim(&operators).unwrap().is_none());
    }
}
