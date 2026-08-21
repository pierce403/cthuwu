use crate::storage::{
    constant_time_eq, ensure_private_directory, hmac_sha256, restrict_file, sync_directory,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
};
use tempfile::NamedTempFile;

pub const NATURE_SCHEMA_VERSION: u32 = 1;
const NATURE_ENVELOPE_VERSION: u32 = 1;
const NATURE_ID_BYTES: usize = 16;
const NATURE_ID_HEX_BYTES: usize = NATURE_ID_BYTES * 2;
const MAX_NATURE_FILE_BYTES: u64 = 64 * 1024;
const MIN_SIGNING_KEY_BYTES: usize = 32;
const MAX_SIGNING_KEY_BYTES: usize = 4 * 1024;
const HMAC_ALGORITHM: &str = "hmac-sha256";
const NATURE_SIGNATURE_DOMAIN: &str = "cthuwu-nature-envelope-v1";
const TRAIT_COUNT: usize = 7;

/// A closed list of the seven sliders that can be adjusted during awakening.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NatureTrait {
    Engagement,
    Growth,
    Wealth,
    Influence,
    Cooperation,
    Stability,
    Transparency,
}

impl NatureTrait {
    pub const ALL: [Self; TRAIT_COUNT] = [
        Self::Engagement,
        Self::Growth,
        Self::Wealth,
        Self::Influence,
        Self::Cooperation,
        Self::Stability,
        Self::Transparency,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Engagement => "engagement",
            Self::Growth => "growth",
            Self::Wealth => "wealth",
            Self::Influence => "influence",
            Self::Cooperation => "cooperation",
            Self::Stability => "stability",
            Self::Transparency => "transparency",
        }
    }
}

impl fmt::Display for NatureTrait {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for NatureTrait {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "engagement" => Ok(Self::Engagement),
            "growth" => Ok(Self::Growth),
            "wealth" => Ok(Self::Wealth),
            "influence" => Ok(Self::Influence),
            "cooperation" => Ok(Self::Cooperation),
            "stability" => Ok(Self::Stability),
            "transparency" => Ok(Self::Transparency),
            _ => bail!("unknown Nature trait"),
        }
    }
}

/// The single activity this Tentacle refuses to perform.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SacredBan {
    Recruitment,
    Spawning,
    Governance,
    Profit,
    MemorySharing,
}

impl SacredBan {
    pub const ALL: [Self; 5] = [
        Self::Recruitment,
        Self::Spawning,
        Self::Governance,
        Self::Profit,
        Self::MemorySharing,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recruitment => "recruitment",
            Self::Spawning => "spawning",
            Self::Governance => "governance",
            Self::Profit => "profit",
            Self::MemorySharing => "memory sharing",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Recruitment => 0,
            Self::Spawning => 1,
            Self::Governance => 2,
            Self::Profit => 3,
            Self::MemorySharing => 4,
        }
    }

    fn from_index(index: usize) -> Result<Self> {
        Self::ALL
            .get(index)
            .copied()
            .context("Sacred Ban index is out of range")
    }

    fn random() -> Result<Self> {
        Self::from_index(secure_uniform(Self::ALL.len() as u32)? as usize)
    }

    fn random_other_than(self) -> Result<Self> {
        let offset = secure_uniform((Self::ALL.len() - 1) as u32)? as usize + 1;
        Self::from_index((self.index() + offset) % Self::ALL.len())
    }
}

impl fmt::Display for SacredBan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The deliberately explicit mutation band selected for one inheritance event.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationMode {
    /// Seventy percent of inheritance events: every slider remains within 10 points and the ban is
    /// inherited unchanged.
    Close,
    /// Twenty percent of events: every slider remains within 35 points, with at least one changing
    /// by 11 or more.
    Drift,
    /// Ten percent of events: values are resampled, at least one changes by 50 or more, and the ban
    /// changes.
    Radical,
}

impl MutationMode {
    /// Maps an unbiased percentile into the exact 70/20/10 inheritance bands.
    pub fn from_percentile(percentile: u8) -> Result<Self> {
        match percentile {
            0..=69 => Ok(Self::Close),
            70..=89 => Ok(Self::Drift),
            90..=99 => Ok(Self::Radical),
            _ => bail!("mutation percentile must be between 0 and 99"),
        }
    }

    fn random() -> Result<Self> {
        Self::from_percentile(secure_uniform(100)? as u8)
    }
}

/// Versioned, bounded policy data for one Tentacle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TentacleNature {
    pub schema_version: u32,
    pub nature_id: String,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_nature_id: Option<String>,
    pub engagement: u8,
    pub growth: u8,
    pub wealth: u8,
    pub influence: u8,
    pub cooperation: u8,
    pub stability: u8,
    pub transparency: u8,
    pub sacred_ban: SacredBan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InheritanceOutcome {
    pub nature: TentacleNature,
    pub mutation: MutationMode,
}

impl TentacleNature {
    /// Generates a founder Nature entirely from operating-system randomness.
    pub fn random() -> Result<Self> {
        Self::random_with_lineage(0, None)
    }

    fn random_with_lineage(generation: u64, parent_nature_id: Option<String>) -> Result<Self> {
        let nature = Self {
            schema_version: NATURE_SCHEMA_VERSION,
            nature_id: random_nature_id()?,
            generation,
            parent_nature_id,
            engagement: random_slider()?,
            growth: random_slider()?,
            wealth: random_slider()?,
            influence: random_slider()?,
            cooperation: random_slider()?,
            stability: random_slider()?,
            transparency: random_slider()?,
            sacred_ban: SacredBan::random()?,
        };
        nature.validate()?;
        Ok(nature)
    }

    /// Produces a new candidate for the same lineage position during the awakening ritual.
    pub fn reroll(&self) -> Result<Self> {
        self.validate()?;
        Self::random_with_lineage(self.generation, self.parent_nature_id.clone())
    }

    /// Selects the mutation band with operating-system randomness and produces a child Nature.
    pub fn inherit(&self) -> Result<InheritanceOutcome> {
        let mutation = MutationMode::random()?;
        let nature = self.inherit_with_mode(mutation)?;
        Ok(InheritanceOutcome { nature, mutation })
    }

    /// Produces a child in a caller-selected mutation band. This is useful for deterministic policy
    /// tests; the actual deltas and new identifier still come from operating-system randomness.
    pub fn inherit_with_mode(&self, mutation: MutationMode) -> Result<Self> {
        self.validate()?;
        let generation = self
            .generation
            .checked_add(1)
            .context("Nature generation overflow")?;
        let mut child = self.clone();
        child.nature_id = random_nature_id()?;
        child.generation = generation;
        child.parent_nature_id = Some(self.nature_id.clone());

        match mutation {
            MutationMode::Close => {
                for nature_trait in NatureTrait::ALL {
                    let value = child.value(nature_trait);
                    child.set_value(nature_trait, vary(value, 10)?);
                }
            }
            MutationMode::Drift => {
                for nature_trait in NatureTrait::ALL {
                    let value = child.value(nature_trait);
                    child.set_value(nature_trait, vary(value, 35)?);
                }
                let anchor = NatureTrait::ALL[secure_uniform(TRAIT_COUNT as u32)? as usize];
                child.set_value(anchor, force_distance(self.value(anchor), 11, 35)?);
                if secure_uniform(2)? == 1 {
                    child.sacred_ban = self.sacred_ban.random_other_than()?;
                }
            }
            MutationMode::Radical => {
                for nature_trait in NatureTrait::ALL {
                    child.set_value(nature_trait, random_slider()?);
                }
                let anchor = NatureTrait::ALL[secure_uniform(TRAIT_COUNT as u32)? as usize];
                let parent_value = self.value(anchor);
                child.set_value(anchor, if parent_value <= 50 { 100 } else { 0 });
                child.sacred_ban = self.sacred_ban.random_other_than()?;
            }
        }

        child.validate()?;
        Ok(child)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != NATURE_SCHEMA_VERSION {
            bail!("unsupported Nature schema version {}", self.schema_version);
        }
        validate_nature_id(&self.nature_id)?;
        match (self.generation, self.parent_nature_id.as_deref()) {
            (0, None) => {}
            (0, Some(_)) => bail!("founder Nature must not identify a parent"),
            (_, None) => bail!("non-founder Nature must identify its parent"),
            (_, Some(parent_id)) => {
                validate_nature_id(parent_id)?;
                if parent_id == self.nature_id {
                    bail!("Nature cannot identify itself as its parent");
                }
            }
        }
        for nature_trait in NatureTrait::ALL {
            if self.value(nature_trait) > 100 {
                bail!("Nature trait {nature_trait} must be between 0 and 100");
            }
        }
        Ok(())
    }

    pub const fn value(&self, nature_trait: NatureTrait) -> u8 {
        match nature_trait {
            NatureTrait::Engagement => self.engagement,
            NatureTrait::Growth => self.growth,
            NatureTrait::Wealth => self.wealth,
            NatureTrait::Influence => self.influence,
            NatureTrait::Cooperation => self.cooperation,
            NatureTrait::Stability => self.stability,
            NatureTrait::Transparency => self.transparency,
        }
    }

    /// Applies one bounded operator adjustment. Invalid adjustments leave the Nature unchanged.
    pub fn adjust(&mut self, nature_trait: NatureTrait, delta: i16) -> Result<u8> {
        self.validate()?;
        let current = i16::from(self.value(nature_trait));
        let adjusted = current
            .checked_add(delta)
            .context("Nature adjustment overflow")?;
        if !(0..=100).contains(&adjusted) {
            bail!("Nature adjustment would leave the 0-100 range");
        }
        let adjusted = adjusted as u8;
        self.set_value(nature_trait, adjusted);
        Ok(adjusted)
    }

    /// A stable, non-secret digest used to bind awakening log entries to the resulting Nature.
    pub fn fingerprint(&self) -> Result<String> {
        self.validate()?;
        Ok(encode_hex(&Sha256::digest(self.canonical_bytes()?)))
    }

    pub fn render(&self) -> String {
        format!(
            "Nature {} (generation {})\n\n\
             Appetites\n\
             - engagement: {}\n\
             - growth: {}\n\
             - wealth: {}\n\
             - influence: {}\n\n\
             Methods\n\
             - cooperation: {}\n\
             - stability: {}\n\
             - transparency: {}\n\n\
             Sacred Ban: {}",
            self.nature_id,
            self.generation,
            self.engagement,
            self.growth,
            self.wealth,
            self.influence,
            self.cooperation,
            self.stability,
            self.transparency,
            self.sacred_ban,
        )
    }

    fn set_value(&mut self, nature_trait: NatureTrait, value: u8) {
        match nature_trait {
            NatureTrait::Engagement => self.engagement = value,
            NatureTrait::Growth => self.growth = value,
            NatureTrait::Wealth => self.wealth = value,
            NatureTrait::Influence => self.influence = value,
            NatureTrait::Cooperation => self.cooperation = value,
            NatureTrait::Stability => self.stability = value,
            NatureTrait::Transparency => self.transparency = value,
        }
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).context("canonicalizing Nature")
    }
}

impl fmt::Display for TentacleNature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// A redacted, domain-separated HMAC-SHA256 signer for local state records.
#[derive(Clone)]
pub struct StateSigner {
    key: Vec<u8>,
}

impl fmt::Debug for StateSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StateSigner([REDACTED])")
    }
}

impl StateSigner {
    pub fn new(key: impl AsRef<[u8]>) -> Result<Self> {
        let key = key.as_ref();
        if !(MIN_SIGNING_KEY_BYTES..=MAX_SIGNING_KEY_BYTES).contains(&key.len()) {
            bail!(
                "state signing key must be {MIN_SIGNING_KEY_BYTES}-{MAX_SIGNING_KEY_BYTES} bytes"
            );
        }
        Ok(Self { key: key.to_vec() })
    }

    pub fn sign(&self, domain: &str, canonical_payload: &[u8]) -> Result<String> {
        validate_signature_domain(domain)?;
        let mut message = Vec::with_capacity(domain.len() + 1 + canonical_payload.len());
        message.extend_from_slice(domain.as_bytes());
        message.push(0);
        message.extend_from_slice(canonical_payload);
        Ok(encode_hex(&hmac_sha256(&self.key, &message)))
    }

    pub fn verify(&self, domain: &str, canonical_payload: &[u8], signature: &str) -> Result<()> {
        validate_signature_domain(domain)?;
        let signature = decode_hex_32(signature).context("state signature is malformed")?;
        let mut message = Vec::with_capacity(domain.len() + 1 + canonical_payload.len());
        message.extend_from_slice(domain.as_bytes());
        message.push(0);
        message.extend_from_slice(canonical_payload);
        let expected = hmac_sha256(&self.key, &message);
        if !constant_time_eq(&signature, &expected) {
            bail!("state signature verification failed");
        }
        Ok(())
    }
}

/// Atomic, owner-only persistence for a signed `state/nature.json` envelope.
#[derive(Clone, Debug)]
pub struct NatureStore {
    path: PathBuf,
    signer: StateSigner,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SignedNatureEnvelope {
    envelope_version: u32,
    algorithm: String,
    nature: TentacleNature,
    signature: String,
}

#[derive(Serialize)]
struct CanonicalNatureEnvelope<'a> {
    envelope_version: u32,
    algorithm: &'static str,
    nature: &'a TentacleNature,
}

impl NatureStore {
    pub fn new(data_dir: &Path, signing_key: impl AsRef<[u8]>) -> Result<Self> {
        Self::with_signer(
            data_dir.join("state").join("nature.json"),
            StateSigner::new(signing_key)?,
        )
    }

    /// Uses an explicit path for `--nature-path` and tests. The parent directory is still made
    /// owner-only and symlinks in the existing path are rejected.
    pub fn with_path(path: impl Into<PathBuf>, signing_key: impl AsRef<[u8]>) -> Result<Self> {
        Self::with_signer(path, StateSigner::new(signing_key)?)
    }

    pub fn with_signer(path: impl Into<PathBuf>, signer: StateSigner) -> Result<Self> {
        let path = path.into();
        prepare_private_parent(&path)?;
        reject_unsafe_target(&path)?;
        Ok(Self { path, signer })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn signer(&self) -> StateSigner {
        self.signer.clone()
    }

    pub fn save(&self, nature: &TentacleNature) -> Result<()> {
        nature.validate()?;
        prepare_private_parent(&self.path)?;
        reject_unsafe_target(&self.path)?;
        if let Ok(metadata) = fs::symlink_metadata(&self.path) {
            assert_owner_only(&metadata, "Nature file")?;
        }

        let canonical = canonical_nature_envelope(nature)?;
        let envelope = SignedNatureEnvelope {
            envelope_version: NATURE_ENVELOPE_VERSION,
            algorithm: HMAC_ALGORITHM.to_owned(),
            nature: nature.clone(),
            signature: self.signer.sign(NATURE_SIGNATURE_DOMAIN, &canonical)?,
        };
        let mut encoded = serde_json::to_vec_pretty(&envelope)?;
        encoded.push(b'\n');
        if encoded.len() as u64 > MAX_NATURE_FILE_BYTES {
            bail!("signed Nature file is too large");
        }

        let parent = parent_directory(&self.path);
        let mut temporary = NamedTempFile::new_in(parent)
            .with_context(|| format!("creating temporary Nature file in {}", parent.display()))?;
        restrict_file(temporary.as_file(), "temporary Nature file")?;
        temporary.write_all(&encoded)?;
        temporary.as_file().sync_all()?;
        reject_unsafe_target(&self.path)?;
        let persisted = temporary
            .persist(&self.path)
            .map_err(|error| error.error)
            .with_context(|| format!("replacing {}", self.path.display()))?;
        restrict_file(&persisted, "Nature file")?;
        persisted.sync_all()?;
        sync_directory(parent)
    }

    pub fn load(&self) -> Result<Option<TentacleNature>> {
        reject_unsafe_target(&self.path)?;
        let mut file = match open_read_no_follow(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("opening {}", self.path.display()));
            }
        };
        let metadata = file
            .metadata()
            .with_context(|| format!("inspecting {}", self.path.display()))?;
        if !metadata.is_file() {
            bail!("Nature path must be a regular file");
        }
        assert_owner_only(&metadata, "Nature file")?;
        if metadata.len() > MAX_NATURE_FILE_BYTES {
            bail!("signed Nature file is too large");
        }
        let mut encoded = Vec::with_capacity(metadata.len() as usize);
        Read::take(&mut file, MAX_NATURE_FILE_BYTES + 1).read_to_end(&mut encoded)?;
        if encoded.len() as u64 > MAX_NATURE_FILE_BYTES {
            bail!("signed Nature file is too large");
        }

        let envelope: SignedNatureEnvelope =
            serde_json::from_slice(&encoded).context("signed Nature file is invalid JSON")?;
        if envelope.envelope_version != NATURE_ENVELOPE_VERSION {
            bail!(
                "unsupported Nature envelope version {}",
                envelope.envelope_version
            );
        }
        if envelope.algorithm != HMAC_ALGORITHM {
            bail!("unsupported Nature signature algorithm");
        }
        envelope.nature.validate()?;
        let canonical = canonical_nature_envelope(&envelope.nature)?;
        self.signer
            .verify(NATURE_SIGNATURE_DOMAIN, &canonical, &envelope.signature)?;
        Ok(Some(envelope.nature))
    }
}

fn canonical_nature_envelope(nature: &TentacleNature) -> Result<Vec<u8>> {
    serde_json::to_vec(&CanonicalNatureEnvelope {
        envelope_version: NATURE_ENVELOPE_VERSION,
        algorithm: HMAC_ALGORITHM,
        nature,
    })
    .context("canonicalizing signed Nature envelope")
}

fn random_slider() -> Result<u8> {
    Ok(secure_uniform(101)? as u8)
}

fn random_nature_id() -> Result<String> {
    let mut bytes = [0_u8; NATURE_ID_BYTES];
    getrandom::fill(&mut bytes).context("generating Nature identifier")?;
    Ok(encode_hex(&bytes))
}

fn secure_uniform(upper_exclusive: u32) -> Result<u32> {
    if upper_exclusive == 0 {
        bail!("random range must not be empty");
    }
    let zone = u32::MAX - (u32::MAX % upper_exclusive);
    loop {
        let mut bytes = [0_u8; 4];
        getrandom::fill(&mut bytes).context("generating Nature randomness")?;
        let candidate = u32::from_le_bytes(bytes);
        if candidate < zone {
            return Ok(candidate % upper_exclusive);
        }
    }
}

fn vary(value: u8, maximum_delta: u8) -> Result<u8> {
    let width = u32::from(maximum_delta) * 2 + 1;
    let delta = secure_uniform(width)? as i16 - i16::from(maximum_delta);
    Ok((i16::from(value) + delta).clamp(0, 100) as u8)
}

fn force_distance(value: u8, minimum_delta: u8, maximum_delta: u8) -> Result<u8> {
    let span = u32::from(maximum_delta - minimum_delta) + 1;
    let delta = minimum_delta + secure_uniform(span)? as u8;
    if value <= 50 {
        Ok(value.saturating_add(delta).min(100))
    } else {
        Ok(value.saturating_sub(delta))
    }
}

pub(crate) fn validate_nature_id(value: &str) -> Result<()> {
    if value.len() != NATURE_ID_HEX_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("Nature identifier must be 32 lowercase hexadecimal characters");
    }
    Ok(())
}

fn validate_signature_domain(domain: &str) -> Result<()> {
    if domain.is_empty()
        || domain.len() > 64
        || !domain
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        bail!("invalid state-signature domain");
    }
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex_32(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64 {
        bail!("signature must contain 64 hexadecimal characters");
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        decoded[index] = (decode_nibble(pair[0])? << 4) | decode_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn decode_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => bail!("signature is not canonical lowercase hexadecimal"),
    }
}

pub(crate) fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

pub(crate) fn prepare_private_parent(path: &Path) -> Result<()> {
    let parent = parent_directory(path);
    reject_symlink_components(parent)?;
    ensure_private_directory(parent)?;
    reject_symlink_components(parent)
}

fn reject_symlink_components(path: &Path) -> Result<()> {
    for ancestor in path.ancestors().filter(|item| !item.as_os_str().is_empty()) {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!(
                    "state path contains symlink component {}",
                    ancestor.display()
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", ancestor.display()));
            }
        }
    }
    Ok(())
}

pub(crate) fn reject_unsafe_target(path: &Path) -> Result<()> {
    reject_symlink_components(parent_directory(path))?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            bail!("state path {} must be a regular file", path.display());
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

pub(crate) fn open_read_no_follow(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

#[cfg(unix)]
pub(crate) fn assert_owner_only(metadata: &fs::Metadata, description: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o077 != 0 {
        bail!("{description} must not be accessible by group or other users");
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn assert_owner_only(_metadata: &fs::Metadata, _description: &str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNING_KEY: &[u8; 32] = b"0123456789abcdef0123456789abcdef";

    fn trait_distances(parent: &TentacleNature, child: &TentacleNature) -> Vec<u8> {
        NatureTrait::ALL
            .iter()
            .map(|nature_trait| {
                parent
                    .value(*nature_trait)
                    .abs_diff(child.value(*nature_trait))
            })
            .collect()
    }

    #[test]
    fn random_generation_is_valid_and_diverse() {
        let mut ids = std::collections::HashSet::new();
        let mut bans = std::collections::HashSet::new();
        for _ in 0..256 {
            let nature = TentacleNature::random().unwrap();
            nature.validate().unwrap();
            assert!(
                NatureTrait::ALL
                    .iter()
                    .all(|nature_trait| nature.value(*nature_trait) <= 100)
            );
            assert!(ids.insert(nature.nature_id));
            bans.insert(nature.sacred_ban);
        }
        assert!(bans.len() > 1);
    }

    #[test]
    fn sacred_bans_are_a_closed_complete_set() {
        assert_eq!(SacredBan::ALL.len(), 5);
        for (index, expected) in SacredBan::ALL.iter().enumerate() {
            assert_eq!(SacredBan::from_index(index).unwrap(), *expected);
        }
        assert!(SacredBan::from_index(SacredBan::ALL.len()).is_err());
    }

    #[test]
    fn mutation_percentiles_are_exactly_seventy_twenty_ten() {
        let mut counts = [0_u8; 3];
        for percentile in 0..100 {
            match MutationMode::from_percentile(percentile).unwrap() {
                MutationMode::Close => counts[0] += 1,
                MutationMode::Drift => counts[1] += 1,
                MutationMode::Radical => counts[2] += 1,
            }
        }
        assert_eq!(counts, [70, 20, 10]);
        assert!(MutationMode::from_percentile(100).is_err());
    }

    #[test]
    fn forced_mutation_modes_obey_their_testable_bounds() {
        let parent = TentacleNature::random().unwrap();
        for _ in 0..64 {
            let close = parent.inherit_with_mode(MutationMode::Close).unwrap();
            assert!(
                trait_distances(&parent, &close)
                    .into_iter()
                    .all(|distance| distance <= 10)
            );
            assert_eq!(close.sacred_ban, parent.sacred_ban);

            let drift = parent.inherit_with_mode(MutationMode::Drift).unwrap();
            let drift_distances = trait_distances(&parent, &drift);
            assert!(drift_distances.iter().all(|distance| *distance <= 35));
            assert!(drift_distances.iter().any(|distance| *distance >= 11));

            let radical = parent.inherit_with_mode(MutationMode::Radical).unwrap();
            assert!(
                trait_distances(&parent, &radical)
                    .into_iter()
                    .any(|distance| distance >= 50)
            );
            assert_ne!(radical.sacred_ban, parent.sacred_ban);
        }
    }

    #[test]
    fn inheritance_preserves_lineage_and_never_leaves_slider_bounds() {
        let parent = TentacleNature::random().unwrap();
        for _ in 0..256 {
            let outcome = parent.inherit().unwrap();
            outcome.nature.validate().unwrap();
            assert_eq!(outcome.nature.generation, 1);
            assert_eq!(
                outcome.nature.parent_nature_id.as_deref(),
                Some(parent.nature_id.as_str())
            );
            assert!(
                NatureTrait::ALL
                    .iter()
                    .all(|nature_trait| outcome.nature.value(*nature_trait) <= 100)
            );
        }
    }

    #[test]
    fn adjustments_reject_out_of_range_values_without_mutating() {
        let mut nature = TentacleNature::random().unwrap();
        nature.engagement = 95;
        assert_eq!(nature.adjust(NatureTrait::Engagement, 5).unwrap(), 100);
        let before = nature.clone();
        assert!(nature.adjust(NatureTrait::Engagement, 1).is_err());
        assert_eq!(nature, before);
        assert_eq!(nature.adjust(NatureTrait::Engagement, -100).unwrap(), 0);
    }

    #[test]
    fn reroll_preserves_lineage_but_replaces_candidate_identity() {
        let parent = TentacleNature::random().unwrap();
        let child = parent.inherit_with_mode(MutationMode::Close).unwrap();
        let rerolled = child.reroll().unwrap();
        assert_eq!(rerolled.generation, child.generation);
        assert_eq!(rerolled.parent_nature_id, child.parent_nature_id);
        assert_ne!(rerolled.nature_id, child.nature_id);
    }

    #[test]
    fn signed_state_round_trips_and_has_a_stable_canonical_signature() {
        let root = tempfile::tempdir().unwrap();
        let nature = TentacleNature::random().unwrap();
        let store = NatureStore::new(root.path(), SIGNING_KEY).unwrap();
        store.save(&nature).unwrap();
        let first: serde_json::Value =
            serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
        store.save(&nature).unwrap();
        let second: serde_json::Value =
            serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
        assert_eq!(first["signature"], second["signature"]);
        assert_eq!(store.load().unwrap(), Some(nature));
    }

    #[test]
    fn tampering_and_wrong_keys_are_detected() {
        let root = tempfile::tempdir().unwrap();
        let store = NatureStore::new(root.path(), SIGNING_KEY).unwrap();
        store.save(&TentacleNature::random().unwrap()).unwrap();

        let mut envelope: serde_json::Value =
            serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
        let replacement = if envelope["nature"]["growth"] == serde_json::json!(100) {
            99
        } else {
            100
        };
        envelope["nature"]["growth"] = serde_json::json!(replacement);
        fs::write(store.path(), serde_json::to_vec_pretty(&envelope).unwrap()).unwrap();
        assert!(store.load().unwrap_err().to_string().contains("signature"));

        store.save(&TentacleNature::random().unwrap()).unwrap();
        let wrong_key = b"abcdef0123456789abcdef0123456789";
        let other = NatureStore::new(root.path(), wrong_key).unwrap();
        assert!(other.load().unwrap_err().to_string().contains("signature"));
    }

    #[cfg(unix)]
    #[test]
    fn state_is_owner_only_and_symlinks_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = tempfile::tempdir().unwrap();
        let store = NatureStore::new(root.path(), SIGNING_KEY).unwrap();
        store.save(&TentacleNature::random().unwrap()).unwrap();
        assert_eq!(
            fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(store.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        fs::remove_file(store.path()).unwrap();
        let outside = root.path().join("outside.json");
        fs::write(&outside, b"outside").unwrap();
        symlink(&outside, store.path()).unwrap();
        assert!(store.load().is_err());
        assert!(store.save(&TentacleNature::random().unwrap()).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"outside");
    }

    #[test]
    fn signer_is_redacted_and_rejects_noncanonical_signatures() {
        assert!(StateSigner::new([0_u8; 31]).is_err());
        let signer = StateSigner::new(SIGNING_KEY).unwrap();
        assert_eq!(format!("{signer:?}"), "StateSigner([REDACTED])");
        let signature = signer.sign("test-domain", b"payload").unwrap();
        signer
            .verify("test-domain", b"payload", &signature)
            .unwrap();
        assert!(signer.verify("test-domain", b"other", &signature).is_err());
        assert!(
            signer
                .verify("test-domain", b"payload", &signature.to_uppercase())
                .is_err()
        );
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231_test_vector() {
        let key = [0x0b_u8; 20];
        let digest = hmac_sha256(&key, b"Hi There");
        assert_eq!(
            encode_hex(&digest),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }
}
