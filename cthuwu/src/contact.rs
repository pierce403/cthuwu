use anyhow::{bail, Context, Result};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OnboardingStage {
    Name,
    Hopes,
    Resources,
    Needs,
    Complete,
}

impl OnboardingStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Hopes => "hopes",
            Self::Resources => "resources",
            Self::Needs => "needs",
            Self::Complete => "complete",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "name" => Ok(Self::Name),
            "hopes" => Ok(Self::Hopes),
            "resources" => Ok(Self::Resources),
            "needs" => Ok(Self::Needs),
            "complete" => Ok(Self::Complete),
            other => bail!("invalid onboarding stage {other:?}"),
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Hopes,
            Self::Hopes => Self::Resources,
            Self::Resources => Self::Needs,
            Self::Needs | Self::Complete => Self::Complete,
        }
    }
}

#[derive(Debug)]
pub struct Contact {
    pub inbox_id: String,
    pub first_seen: u64,
    pub last_seen: u64,
    pub stage: OnboardingStage,
    pub name: Option<String>,
    pub hopes: Option<String>,
    pub resources: Option<String>,
    pub needs: Option<String>,
}

impl Contact {
    fn new(inbox_id: String) -> Self {
        let now = unix_seconds();
        Self {
            inbox_id,
            first_seen: now,
            last_seen: now,
            stage: OnboardingStage::Name,
            name: None,
            hopes: None,
            resources: None,
            needs: None,
        }
    }

    pub fn record_answer(&mut self, answer: &str) {
        let answer = answer.trim().to_owned();
        match self.stage {
            OnboardingStage::Name => self.name = Some(answer),
            OnboardingStage::Hopes => self.hopes = Some(answer),
            OnboardingStage::Resources => self.resources = Some(answer),
            OnboardingStage::Needs => self.needs = Some(answer),
            OnboardingStage::Complete => {}
        }
        self.stage = self.stage.next();
        self.last_seen = unix_seconds();
    }

    fn markdown(&self) -> String {
        format!(
            "---\ninbox_id: {}\nfirst_seen_unix: {}\nlast_seen_unix: {}\nonboarding_stage: {}\n---\n\n# Contact {}\n\n## Name\n\n{}\n\n## Hopes and dreams\n\n{}\n\n## Resources they may want to contribute\n\n{}\n\n## Needs and support\n\n{}\n",
            self.inbox_id,
            self.first_seen,
            self.last_seen,
            self.stage.as_str(),
            self.inbox_id,
            markdown_value(self.name.as_deref()),
            markdown_value(self.hopes.as_deref()),
            markdown_value(self.resources.as_deref()),
            markdown_value(self.needs.as_deref()),
        )
    }

    fn parse(markdown: &str) -> Result<Self> {
        let inbox_id = metadata(markdown, "inbox_id")?.to_owned();
        let first_seen = metadata(markdown, "first_seen_unix")?.parse()?;
        let last_seen = metadata(markdown, "last_seen_unix")?.parse()?;
        let stage = OnboardingStage::parse(metadata(markdown, "onboarding_stage")?)?;

        Ok(Self {
            inbox_id,
            first_seen,
            last_seen,
            stage,
            name: section(markdown, "Name"),
            hopes: section(markdown, "Hopes and dreams"),
            resources: section(markdown, "Resources they may want to contribute"),
            needs: section(markdown, "Needs and support"),
        })
    }
}

pub struct ContactStore {
    directory: PathBuf,
}

impl ContactStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let directory = data_dir.join("contacts");
        fs::create_dir_all(&directory)
            .with_context(|| format!("creating {}", directory.display()))?;
        Ok(Self { directory })
    }

    pub fn load_or_create(&self, inbox_id: &str) -> Result<(Contact, bool)> {
        let inbox_id = normalize_inbox_id(inbox_id)?;
        let path = self.path(&inbox_id);
        match fs::read_to_string(&path) {
            Ok(markdown) => Ok((Contact::parse(&markdown)?, false)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let contact = Contact::new(inbox_id);
                self.save(&contact)?;
                Ok((contact, true))
            }
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, contact: &Contact) -> Result<()> {
        let path = self.path(&contact.inbox_id);
        let temp = path.with_extension(format!("md.tmp.{}", std::process::id()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .with_context(|| format!("creating {}", temp.display()))?;
        file.write_all(contact.markdown().as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, &path)
            .with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    fn path(&self, inbox_id: &str) -> PathBuf {
        self.directory.join(format!("{inbox_id}.md"))
    }
}

fn normalize_inbox_id(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.len() > 128
        || !normalized.chars().all(|c| c.is_ascii_hexdigit())
    {
        bail!("invalid XMTP inbox ID");
    }
    Ok(normalized)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn markdown_value(value: Option<&str>) -> String {
    match value {
        Some(value) if !value.is_empty() => value
            .lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => "_Not shared yet._".to_owned(),
    }
}

fn metadata<'a>(markdown: &'a str, key: &str) -> Result<&'a str> {
    markdown
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}: ")))
        .with_context(|| format!("contact is missing {key}"))
}

fn section(markdown: &str, heading: &str) -> Option<String> {
    let marker = format!("## {heading}\n\n");
    let value = markdown.split_once(&marker)?.1.split("\n\n## ").next()?.trim();
    if value == "_Not shared yet._" {
        return None;
    }
    Some(
        value
            .lines()
            .map(|line| line.strip_prefix("> ").unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_exact_contact_path_and_persists_answers() {
        let root = tempfile::tempdir().unwrap();
        let store = ContactStore::new(root.path()).unwrap();
        let id = "aabbcc001122";

        let (mut contact, created) = store.load_or_create(id).unwrap();
        assert!(created);
        assert_eq!(contact.stage, OnboardingStage::Name);
        assert!(root.path().join("contacts/aabbcc001122.md").exists());

        contact.record_answer("Nyx\n## not a real heading");
        store.save(&contact).unwrap();

        let (loaded, created) = store.load_or_create(id).unwrap();
        assert!(!created);
        assert_eq!(loaded.name.as_deref(), Some("Nyx\n## not a real heading"));
        assert_eq!(loaded.stage, OnboardingStage::Hopes);
    }

    #[test]
    fn rejects_path_traversal_as_an_inbox_id() {
        let root = tempfile::tempdir().unwrap();
        let store = ContactStore::new(root.path()).unwrap();
        assert!(store.load_or_create("../../oops").is_err());
    }
}
