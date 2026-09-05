//! Protected generic configuration and named credential slots. Values never appear in status.
use crate::storage::{ensure_private_directory, restrict_file, sync_directory};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub name: String,
    pub value: String,
    pub enabled: bool,
    pub cooldown: u64,
    #[serde(default)]
    pub failure: Option<CredentialFailure>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialFailure {
    Rejected,
    RateLimited,
    Transient,
}
pub struct Environment {
    path: PathBuf,
    entries: Mutex<BTreeMap<String, Vec<Entry>>>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Environment {
    pub fn open(data: &Path) -> Result<Self> {
        let root = data.join("state");
        ensure_private_directory(&root)?;
        let path = root.join("environment.json");
        if fs::symlink_metadata(&path)
            .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink() || m.len() > 256 * 1024)
        {
            bail!("invalid environment store");
        }
        let entries: BTreeMap<String, Vec<Entry>> = if path.exists() {
            serde_json::from_slice(&fs::read(&path)?)?
        } else {
            BTreeMap::new()
        };
        for (name, values) in &entries {
            validate_name(name)?;
            if values.len() > 8 {
                bail!("too many credential slots");
            }
            for entry in values {
                validate_entry(&entry.name, &entry.value)?;
            }
        }
        Ok(Self {
            path,
            entries: Mutex::new(entries),
        })
    }

    pub fn candidates(&self, name: &str) -> Result<Vec<Entry>> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("environment lock"))?
            .get(name)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.enabled && e.cooldown <= now())
            .collect())
    }

    /// Explicit diagnostics may probe enabled slots during cooldown, never disabled slots.
    pub fn diagnostic_candidates(&self, name: &str) -> Result<Vec<Entry>> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("environment lock"))?
            .get(name)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.enabled)
            .collect())
    }

    pub fn verified(&self, variable: &str, tested: &Entry) -> Result<bool> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("environment lock"))?;
        let mut next = entries.clone();
        let Some(entry) = next.get_mut(variable).and_then(|slots| {
            slots
                .iter_mut()
                .find(|e| e.name == tested.name && e.value == tested.value && e.enabled)
        }) else {
            return Ok(false);
        };
        entry.cooldown = 0;
        entry.failure = None;
        self.save(&next)?;
        *entries = next;
        Ok(true)
    }

    pub fn contains(&self, name: &str) -> bool {
        // A damaged override store must not silently revive a legacy startup credential.
        self.entries.lock().map_or(true, |e| e.contains_key(name))
    }

    pub fn configured(&self, name: &str) -> Result<bool> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("environment lock"))?
            .get(name)
            .is_some_and(|values| values.iter().any(|entry| entry.enabled)))
    }

    pub fn failed(&self, variable: &str, slot: &str, failure: CredentialFailure) {
        if let Ok(mut entries) = self.entries.lock() {
            let mut next = entries.clone();
            if let Some(entry) = next
                .get_mut(variable)
                .and_then(|v| v.iter_mut().find(|e| e.name == slot))
            {
                entry.failure = Some(failure);
                entry.cooldown = now()
                    + if matches!(failure, CredentialFailure::RateLimited) {
                        300
                    } else {
                        60
                    };
                if matches!(failure, CredentialFailure::Rejected) {
                    entry.enabled = false;
                }
            }
            if self.save(&next).is_ok() {
                *entries = next;
            }
        }
    }

    fn save(&self, entries: &BTreeMap<String, Vec<Entry>>) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(self.path.parent().unwrap())?;
        restrict_file(file.as_file(), "environment configuration")?;
        let bytes = serde_json::to_vec(entries)?;
        if bytes.len() > 256 * 1024 {
            bail!("environment store is full");
        }
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(&self.path).map_err(|e| e.error)?;
        sync_directory(self.path.parent().unwrap())
    }

    pub fn command(&self, arguments: &str) -> Result<String> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("environment lock"))?;
        let mut next = entries.clone();
        let (op, rest) = arguments
            .trim()
            .split_once(' ')
            .unwrap_or((arguments.trim(), ""));
        if matches!(op, "" | "list" | "get") {
            return Ok(next
                .iter()
                .filter(|(name, _)| op != "get" || name.as_str() == rest.trim())
                .map(|(name, slots)| {
                    format!(
                        "{name}: {}",
                        slots
                            .iter()
                            .map(|entry| format!(
                                "{}=[redacted], {}, {}",
                                entry.name,
                                if entry.enabled { "enabled" } else { "disabled" },
                                match entry.failure {
                                    Some(CredentialFailure::Rejected) => "credential rejected; replace or explicitly enable to retry",
                                    Some(CredentialFailure::RateLimited) if entry.cooldown > now() => "rate limited; five-minute cooldown",
                                    Some(CredentialFailure::Transient) if entry.cooldown > now() => "temporary failure; one-minute cooldown",
                                    _ => "enabled; inference health is checked on use or with /doctor",
                                }
                            ))
                            .collect::<Vec<_>>()
                            .join("; ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"));
        }
        let (variable, remaining) = rest.split_once(' ').unwrap_or((rest, ""));
        validate_name(variable)?;
        match op {
            "set" | "add" => {
                let (slot, value) = if op == "set" {
                    ("primary", remaining)
                } else {
                    remaining
                        .split_once(' ')
                        .context("usage: /env add NAME slot value")?
                };
                validate_entry(slot, value)?;
                let values = next.entry(variable.into()).or_default();
                if values.iter().any(|entry| entry.name == slot) && op == "add" {
                    bail!("slot exists; remove it before replacing it");
                }
                values.retain(|entry| entry.name != slot);
                if values.len() >= 8 {
                    bail!("at most eight slots per variable");
                }
                let entry = Entry {
                    name: slot.into(),
                    value: value.into(),
                    enabled: true,
                    cooldown: 0,
                    failure: None,
                };
                if op == "set" {
                    values.insert(0, entry);
                } else {
                    values.push(entry);
                }
            }
            "unset" => {
                // An empty credential override must not resurrect an old file or startup key.
                if matches!(variable, "VENICE_API_KEY" | "UWUBOT_MODEL_API_KEY") {
                    next.insert(variable.into(), Vec::new());
                } else {
                    next.remove(variable);
                }
            }
            "remove" | "enable" | "disable" => {
                let values = next.get_mut(variable).context("variable not configured")?;
                let index = values
                    .iter()
                    .position(|entry| entry.name == remaining.trim())
                    .context("slot not found")?;
                if op == "remove" {
                    values.remove(index);
                } else {
                    values[index].enabled = op == "enable";
                    values[index].cooldown = 0;
                    values[index].failure = None;
                }
            }
            _ => bail!(
                "usage: /env set NAME value | add NAME slot value | get NAME | list | unset NAME | remove/enable/disable NAME slot"
            ),
        }
        self.save(&next)?;
        *entries = next;
        Ok(format!(
            "{variable}: {op} saved. Value redacted. Model credentials apply on the next request; TOOL_* values apply to the next operator command. Other settings require a supported adapter."
        ))
    }

    pub fn tool_values(&self) -> Result<BTreeMap<String, String>> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("environment lock"))?;
        Ok(entries
            .iter()
            .filter(|(name, _)| name.starts_with("TOOL_"))
            .filter_map(|(name, values)| {
                values
                    .iter()
                    .find(|v| v.enabled && v.cooldown <= now())
                    .map(|v| (name.clone(), v.value.clone()))
            })
            .collect())
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        || name.as_bytes()[0].is_ascii_digit()
    {
        bail!("invalid environment variable name");
    }
    if !matches!(
        name,
        "VENICE_API_KEY"
            | "UWUBOT_VENICE_API_KEY"
            | "UWUBOT_MODEL_API_KEY"
            | "UWUBOT_PROVIDER"
            | "UWUBOT_MODEL"
            | "CTHUWU_RPC_ENDPOINT"
    ) && !name.starts_with("TOOL_")
    {
        bail!("unsupported runtime variable; use TOOL_* for operator command environment values");
    }
    Ok(())
}
fn validate_entry(slot: &str, value: &str) -> Result<()> {
    if slot.is_empty()
        || slot.len() > 64
        || !slot
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        || value.is_empty()
        || value.len() > 4096
        || value.chars().any(char::is_control)
    {
        bail!("invalid configuration slot or value");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_recovery_preserves_disabled_and_changed_slots() {
        let root = tempfile::tempdir().unwrap();
        let env = Environment::open(root.path()).unwrap();
        env.command("set VENICE_API_KEY original").unwrap();
        let tested = env
            .diagnostic_candidates("VENICE_API_KEY")
            .unwrap()
            .remove(0);
        env.failed("VENICE_API_KEY", "primary", CredentialFailure::Rejected);
        assert!(
            env.diagnostic_candidates("VENICE_API_KEY")
                .unwrap()
                .is_empty()
        );
        assert!(!env.verified("VENICE_API_KEY", &tested).unwrap());
        env.command("set VENICE_API_KEY replacement").unwrap();
        env.failed("VENICE_API_KEY", "primary", CredentialFailure::Transient);
        assert!(!env.verified("VENICE_API_KEY", &tested).unwrap());
        assert!(env.candidates("VENICE_API_KEY").unwrap().is_empty());
        let tested = env
            .diagnostic_candidates("VENICE_API_KEY")
            .unwrap()
            .remove(0);
        assert!(env.verified("VENICE_API_KEY", &tested).unwrap());
        assert_eq!(
            Environment::open(root.path())
                .unwrap()
                .candidates("VENICE_API_KEY")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn redaction_failover_and_loader_rejection() {
        let root = tempfile::tempdir().unwrap();
        let env = Environment::open(root.path()).unwrap();
        env.command("set VENICE_API_KEY secret-primary").unwrap();
        env.command("add VENICE_API_KEY backup secret-backup")
            .unwrap();
        assert!(!env.command("list").unwrap().contains("secret-"));
        env.failed("VENICE_API_KEY", "primary", CredentialFailure::Transient);
        assert_eq!(env.candidates("VENICE_API_KEY").unwrap()[0].name, "backup");
        assert!(env.command("set BASH_ENV /tmp/startup").is_err());
        assert!(env.tool_values().unwrap().is_empty());
        assert_eq!(
            Environment::open(root.path())
                .unwrap()
                .candidates("VENICE_API_KEY")
                .unwrap()[0]
                .name,
            "backup"
        );
    }
}
