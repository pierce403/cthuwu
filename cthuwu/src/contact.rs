use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;

use crate::storage::{ensure_private_directory, restrict_file, sync_directory};

const NOT_SHARED: &str = "_Not shared yet._";
const SKIPPED: &str = "_Skipped._";
pub const CURRENT_SHARING_CONSENT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OnboardingStage {
    Name,
    Hopes,
    Resources,
    Needs,
    SharingConsent,
    Complete,
}

impl OnboardingStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Hopes => "hopes",
            Self::Resources => "resources",
            Self::Needs => "needs",
            Self::SharingConsent => "sharing-consent",
            Self::Complete => "complete",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "name" => Ok(Self::Name),
            "hopes" => Ok(Self::Hopes),
            "resources" => Ok(Self::Resources),
            "needs" => Ok(Self::Needs),
            "sharing-consent" => Ok(Self::SharingConsent),
            "complete" => Ok(Self::Complete),
            other => bail!("invalid onboarding stage {other:?}"),
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Hopes,
            Self::Hopes => Self::Resources,
            Self::Resources => Self::Needs,
            Self::Needs => Self::SharingConsent,
            Self::SharingConsent | Self::Complete => Self::Complete,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Contact {
    pub inbox_id: String,
    pub first_seen: u64,
    pub last_seen: u64,
    pub stage: OnboardingStage,
    pub name: Option<String>,
    pub hopes: Option<String>,
    pub resources: Option<String>,
    pub needs: Option<String>,
    pub sharing_enabled: bool,
    pub sharing_consent_version: u32,
    pub introductions_paused: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum ContactField {
    Name,
    Hopes,
    Resources,
    Needs,
}

impl ContactField {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "name" => Some(Self::Name),
            "hopes" | "dreams" => Some(Self::Hopes),
            "resources" | "offers" | "skills" => Some(Self::Resources),
            "needs" | "support" => Some(Self::Needs),
            _ => None,
        }
    }
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
            sharing_enabled: false,
            sharing_consent_version: 0,
            introductions_paused: false,
        }
    }

    /// Records an onboarding answer and returns whether the stage advanced.
    pub fn record_answer(&mut self, answer: &str) -> bool {
        match self.stage {
            OnboardingStage::Name => self.name = answer_value(answer),
            OnboardingStage::Hopes => self.hopes = answer_value(answer),
            OnboardingStage::Resources => self.resources = answer_value(answer),
            OnboardingStage::Needs => self.needs = answer_value(answer),
            OnboardingStage::SharingConsent => {
                let Some(consent) = sharing_consent(answer) else {
                    self.touch();
                    return false;
                };
                self.sharing_enabled = consent;
                self.sharing_consent_version = if consent {
                    CURRENT_SHARING_CONSENT_VERSION
                } else {
                    0
                };
            }
            OnboardingStage::Complete => return false,
        }
        self.stage = self.stage.next();
        self.touch();
        true
    }

    pub fn skip(&mut self) {
        match self.stage {
            OnboardingStage::Name => self.name = Some(SKIPPED.to_owned()),
            OnboardingStage::Hopes => self.hopes = Some(SKIPPED.to_owned()),
            OnboardingStage::Resources => self.resources = Some(SKIPPED.to_owned()),
            OnboardingStage::Needs => self.needs = Some(SKIPPED.to_owned()),
            OnboardingStage::SharingConsent => {
                self.sharing_enabled = false;
                self.sharing_consent_version = 0;
            }
            OnboardingStage::Complete => return,
        }
        self.stage = self.stage.next();
        self.touch();
    }

    pub fn set_field(&mut self, field: ContactField, value: &str) {
        let value = answer_value(value);
        match field {
            ContactField::Name => self.name = value,
            ContactField::Hopes => self.hopes = value,
            ContactField::Resources => self.resources = value,
            ContactField::Needs => self.needs = value,
        }
        self.touch();
    }

    pub fn display_name(&self) -> String {
        let name = self
            .name
            .as_deref()
            .filter(|name| *name != SKIPPED)
            .unwrap_or("an unnamed friend");
        let normalized = name
            .split_whitespace()
            .flat_map(|part| [part, " "])
            .collect::<String>()
            .trim()
            .chars()
            .filter(|character| !character.is_control())
            .take(64)
            .collect::<String>();
        if normalized.is_empty() {
            "an unnamed friend".to_owned()
        } else {
            normalized
        }
    }

    pub fn profile_markdown(&self) -> String {
        format!(
            "## Name\n{}\n\n## Hopes and dreams\n{}\n\n## Resources they may want to contribute\n{}\n\n## Needs and support\n{}\n\n## Sharing\n{}\n",
            markdown_value(self.name.as_deref()),
            markdown_value(self.hopes.as_deref()),
            markdown_value(self.resources.as_deref()),
            markdown_value(self.needs.as_deref()),
            if self.is_matching_enabled() {
                "Opted in to private match suggestions."
            } else if self.sharing_enabled {
                "Previous sharing consent needs renewal."
            } else {
                "Not opted in."
            }
        )
    }

    fn touch(&mut self) {
        self.last_seen = unix_seconds();
    }

    pub fn mark_seen(&mut self) {
        self.touch();
    }

    pub fn is_matching_enabled(&self) -> bool {
        self.sharing_enabled && self.sharing_consent_version == CURRENT_SHARING_CONSENT_VERSION
    }

    fn markdown(&self) -> String {
        format!(
            "---\ninbox_id: {}\nfirst_seen_unix: {}\nlast_seen_unix: {}\nonboarding_stage: {}\nsharing_enabled: {}\nsharing_consent_version: {}\nintroductions_paused: {}\n---\n\n# Contact {}\n\n{}",
            self.inbox_id,
            self.first_seen,
            self.last_seen,
            self.stage.as_str(),
            self.sharing_enabled,
            self.sharing_consent_version,
            self.introductions_paused,
            self.inbox_id,
            self.profile_markdown(),
        )
    }

    fn parse(markdown: &str) -> Result<Self> {
        let inbox_id = normalize_inbox_id(metadata(markdown, "inbox_id")?)?;
        let first_seen = metadata(markdown, "first_seen_unix")?.parse()?;
        let last_seen = metadata(markdown, "last_seen_unix")?.parse()?;
        let stage = OnboardingStage::parse(metadata(markdown, "onboarding_stage")?)?;
        let sharing_enabled = optional_bool_metadata(markdown, "sharing_enabled")?;
        let sharing_consent_version = optional_u32_metadata(markdown, "sharing_consent_version")?;
        let introductions_paused = optional_bool_metadata(markdown, "introductions_paused")?;

        Ok(Self {
            inbox_id,
            first_seen,
            last_seen,
            stage,
            name: section(markdown, "Name"),
            hopes: section(markdown, "Hopes and dreams"),
            resources: section(markdown, "Resources they may want to contribute"),
            needs: section(markdown, "Needs and support"),
            sharing_enabled,
            sharing_consent_version,
            introductions_paused,
        })
    }
}

#[derive(Clone)]
pub struct ContactStore {
    directory: PathBuf,
}

impl ContactStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let directory = data_dir.join("contacts");
        ensure_private_directory(&directory)?;
        Ok(Self { directory })
    }

    pub fn load_or_create(&self, inbox_id: &str) -> Result<(Contact, bool)> {
        let inbox_id = normalize_inbox_id(inbox_id)?;
        if let Some(contact) = self.load(&inbox_id)? {
            return Ok((contact, false));
        }
        let contact = Contact::new(inbox_id);
        self.save(&contact)?;
        Ok((contact, true))
    }

    pub fn load(&self, inbox_id: &str) -> Result<Option<Contact>> {
        let inbox_id = normalize_inbox_id(inbox_id)?;
        let path = self.path(&inbox_id);
        reject_symlink(&path)?;
        match fs::read_to_string(&path) {
            Ok(markdown) => {
                let contact = Contact::parse(&markdown)?;
                if contact.inbox_id != inbox_id {
                    bail!("contact note inbox ID does not match {}", path.display());
                }
                Ok(Some(contact))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn list(&self) -> Result<Vec<Contact>> {
        let mut contacts = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("md") {
                continue;
            }
            if entry.file_type()?.is_symlink() {
                bail!("contact note {} must not be a symlink", path.display());
            }
            let markdown =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let contact = Contact::parse(&markdown)?;
            let file_inbox_id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .context("contact note has a non-Unicode filename")?;
            if contact.inbox_id != normalize_inbox_id(file_inbox_id)? {
                bail!("contact note inbox ID does not match {}", path.display());
            }
            contacts.push(contact);
        }
        contacts.sort_by(|left, right| left.inbox_id.cmp(&right.inbox_id));
        Ok(contacts)
    }

    pub fn save(&self, contact: &Contact) -> Result<()> {
        let path = self.path(&normalize_inbox_id(&contact.inbox_id)?);
        let mut temp = NamedTempFile::new_in(&self.directory).with_context(|| {
            format!("creating temporary contact in {}", self.directory.display())
        })?;
        restrict_file(temp.as_file(), "temporary contact note")?;
        temp.write_all(contact.markdown().as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(&path)
            .map_err(|error| error.error)
            .with_context(|| format!("replacing {}", path.display()))?;
        sync_directory(&self.directory)?;
        Ok(())
    }

    pub fn delete(&self, inbox_id: &str) -> Result<bool> {
        let path = self.path(&normalize_inbox_id(inbox_id)?);
        match fs::remove_file(&path) {
            Ok(()) => {
                sync_directory(&self.directory)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).with_context(|| format!("deleting {}", path.display())),
        }
    }

    fn path(&self, inbox_id: &str) -> PathBuf {
        self.directory.join(format!("{inbox_id}.md"))
    }
}

pub fn normalize_inbox_id(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.len() > 128
        || !normalized
            .chars()
            .all(|character| character.is_ascii_hexdigit())
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

fn answer_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn sharing_consent(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "y" | "yeah" | "yep" | "sure" | "ok" | "okay" | "on" => Some(true),
        "no" | "n" | "nope" | "not now" | "off" => Some(false),
        _ => None,
    }
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("contact note {} must not be a symlink", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

fn markdown_value(value: Option<&str>) -> String {
    match value {
        Some(SKIPPED) => SKIPPED.to_owned(),
        Some(value) if !value.is_empty() => value
            .lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => NOT_SHARED.to_owned(),
    }
}

fn metadata<'a>(markdown: &'a str, key: &str) -> Result<&'a str> {
    markdown
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}: ")))
        .with_context(|| format!("contact is missing {key}"))
}

fn optional_bool_metadata(markdown: &str, key: &str) -> Result<bool> {
    match markdown
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}: ")))
    {
        Some(value) => value
            .parse()
            .with_context(|| format!("contact has invalid {key}")),
        None => Ok(false),
    }
}

fn optional_u32_metadata(markdown: &str, key: &str) -> Result<u32> {
    match markdown
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}: ")))
    {
        Some(value) => value
            .parse()
            .with_context(|| format!("contact has invalid {key}")),
        None => Ok(0),
    }
}

fn section(markdown: &str, heading: &str) -> Option<String> {
    let marker = format!("## {heading}\n");
    let value = markdown
        .split_once(&marker)?
        .1
        .trim_start_matches('\n')
        .split("\n\n## ")
        .next()?
        .trim();
    if value == NOT_SHARED {
        return None;
    }
    if value == SKIPPED {
        return Some(SKIPPED.to_owned());
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

        assert!(contact.record_answer("Nyx\n## not a real heading"));
        store.save(&contact).unwrap();

        let (loaded, created) = store.load_or_create(id).unwrap();
        assert!(!created);
        assert_eq!(loaded.name.as_deref(), Some("Nyx\n## not a real heading"));
        assert_eq!(loaded.stage, OnboardingStage::Hopes);
    }

    #[test]
    fn skips_and_corrects_answers() {
        let root = tempfile::tempdir().unwrap();
        let store = ContactStore::new(root.path()).unwrap();
        let (mut contact, _) = store.load_or_create("aabbcc").unwrap();

        contact.skip();
        contact.set_field(ContactField::Name, "Nyx");
        store.save(&contact).unwrap();

        let loaded = store.load("aabbcc").unwrap().unwrap();
        assert_eq!(loaded.display_name(), "Nyx");
        assert_eq!(loaded.stage, OnboardingStage::Hopes);
    }

    #[test]
    fn lists_and_deletes_contacts() {
        let root = tempfile::tempdir().unwrap();
        let store = ContactStore::new(root.path()).unwrap();
        store.load_or_create("aabb").unwrap();
        store.load_or_create("ccdd").unwrap();
        assert_eq!(store.list().unwrap().len(), 2);
        assert!(store.delete("aabb").unwrap());
        assert!(!store.delete("aabb").unwrap());
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn rejects_path_traversal_as_an_inbox_id() {
        let root = tempfile::tempdir().unwrap();
        let store = ContactStore::new(root.path()).unwrap();
        assert!(store.load_or_create("../../oops").is_err());
    }

    #[test]
    fn consent_requires_an_explicit_yes_or_no() {
        let mut contact = Contact::new("aabbcc".to_owned());
        contact.stage = OnboardingStage::SharingConsent;

        assert!(!contact.record_answer("maybe later"));
        assert_eq!(contact.stage, OnboardingStage::SharingConsent);
        assert!(contact.record_answer("no"));
        assert_eq!(contact.stage, OnboardingStage::Complete);
        assert!(!contact.sharing_enabled);
    }

    #[test]
    fn display_names_are_single_line_and_bounded() {
        let mut contact = Contact::new("aabbcc".to_owned());
        contact.name = Some(format!("Nyx\n{}", "x".repeat(100)));
        let name = contact.display_name();
        assert!(!name.contains('\n'));
        assert!(name.chars().count() <= 64);
    }
}
