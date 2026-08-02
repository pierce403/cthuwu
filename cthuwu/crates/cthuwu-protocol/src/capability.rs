use crate::validation::{bounded_count, validate_slug};
use crate::{ProtocolVersion, ValidationError, ValidationErrorKind};
use serde::{Deserialize, Deserializer, Serialize};

pub const MAX_PROTOCOL_VERSIONS: usize = 8;
pub const MAX_MODEL_CLASSES: usize = 32;
pub const MAX_TOOLS: usize = 64;
pub const MAX_MEMORY_MODES: usize = 8;
pub const MAX_PRIVACY_PROPERTIES: usize = 16;
pub const MAX_TRUST_MECHANISMS: usize = 16;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CapabilityName(String);

impl CapabilityName {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_slug("capabilityName", &value, 64)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for CapabilityName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryMode {
    None,
    Session,
    LocalContact,
    LocalSemantic,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrivacyProperty {
    NoCouncilContent,
    LocalMemoryOnly,
    NoRemoteInference,
    EphemeralSession,
    OperatorControlledRetention,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InferenceLocation {
    Local,
    Remote,
    Hybrid,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustMechanism {
    LocalAllowlist,
    OperatorAttestation,
    RegistryAssociation,
    RegistryReputation,
    CouncilMembership,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capacity {
    pub max_concurrent_sessions: u32,
    pub available_sessions: u32,
    pub max_context_tokens: u64,
}

impl Capacity {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.max_concurrent_sessions == 0
            || self.available_sessions > self.max_concurrent_sessions
            || self.max_context_tokens == 0
            || self.max_context_tokens > 100_000_000
        {
            return Err(ValidationError::new(
                "capabilities.capacity",
                ValidationErrorKind::OutOfRange,
            ));
        }
        Ok(())
    }
}

/// Public-safe routing claims. It intentionally cannot carry credentials, endpoints, or hardware inventory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityManifest {
    pub schema_version: ProtocolVersion,
    pub protocol_versions: Vec<ProtocolVersion>,
    pub model_classes: Vec<CapabilityName>,
    pub context_limit_tokens: u64,
    pub tools: Vec<CapabilityName>,
    pub memory_modes: Vec<MemoryMode>,
    pub privacy_properties: Vec<PrivacyProperty>,
    pub inference_location: InferenceLocation,
    pub capacity: Capacity,
    pub visibility: CapabilityVisibility,
    pub supported_trust_mechanisms: Vec<TrustMechanism>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityVisibility {
    Private,
    Council,
    Public,
}

impl CapabilityManifest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.schema_version.require_supported()?;
        if self.protocol_versions.is_empty() {
            return Err(ValidationError::new(
                "capabilities.protocolVersions",
                ValidationErrorKind::Empty,
            ));
        }
        bounded_count(
            "capabilities.protocolVersions",
            self.protocol_versions.len(),
            MAX_PROTOCOL_VERSIONS,
        )?;
        for version in &self.protocol_versions {
            version.require_supported()?;
        }
        bounded_count(
            "capabilities.modelClasses",
            self.model_classes.len(),
            MAX_MODEL_CLASSES,
        )?;
        bounded_count("capabilities.tools", self.tools.len(), MAX_TOOLS)?;
        bounded_count(
            "capabilities.memoryModes",
            self.memory_modes.len(),
            MAX_MEMORY_MODES,
        )?;
        bounded_count(
            "capabilities.privacyProperties",
            self.privacy_properties.len(),
            MAX_PRIVACY_PROPERTIES,
        )?;
        bounded_count(
            "capabilities.supportedTrustMechanisms",
            self.supported_trust_mechanisms.len(),
            MAX_TRUST_MECHANISMS,
        )?;
        if self.context_limit_tokens == 0 || self.context_limit_tokens > 100_000_000 {
            return Err(ValidationError::new(
                "capabilities.contextLimitTokens",
                ValidationErrorKind::OutOfRange,
            ));
        }
        ensure_unique("capabilities.protocolVersions", &self.protocol_versions)?;
        ensure_unique("capabilities.modelClasses", &self.model_classes)?;
        ensure_unique("capabilities.tools", &self.tools)?;
        ensure_unique("capabilities.memoryModes", &self.memory_modes)?;
        ensure_unique("capabilities.privacyProperties", &self.privacy_properties)?;
        ensure_unique(
            "capabilities.supportedTrustMechanisms",
            &self.supported_trust_mechanisms,
        )?;
        self.capacity.validate()
    }
}

fn ensure_unique<T: Ord>(field: &str, values: &[T]) -> Result<(), ValidationError> {
    let mut values = values.iter().collect::<Vec<_>>();
    values.sort();
    if values.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ValidationError::new(
            field,
            ValidationErrorKind::InvalidFormat,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> CapabilityManifest {
        CapabilityManifest {
            schema_version: ProtocolVersion::V1_0,
            protocol_versions: vec![ProtocolVersion::V1_0],
            model_classes: vec![CapabilityName::new("text-chat").unwrap()],
            context_limit_tokens: 32_768,
            tools: vec![CapabilityName::new("protocol-self-test").unwrap()],
            memory_modes: vec![MemoryMode::LocalContact],
            privacy_properties: vec![PrivacyProperty::NoCouncilContent],
            inference_location: InferenceLocation::Local,
            capacity: Capacity {
                max_concurrent_sessions: 4,
                available_sessions: 3,
                max_context_tokens: 32_768,
            },
            visibility: CapabilityVisibility::Council,
            supported_trust_mechanisms: vec![TrustMechanism::LocalAllowlist],
        }
    }

    #[test]
    fn capability_manifest_round_trips_without_secret_shaped_fields() {
        let manifest = manifest();
        manifest.validate().unwrap();
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(!json.contains("credential"));
        assert!(!json.contains("endpoint"));
        assert!(!json.contains("hardware"));
        assert_eq!(
            serde_json::from_str::<CapabilityManifest>(&json).unwrap(),
            manifest
        );
    }

    #[test]
    fn capability_manifest_rejects_duplicate_and_impossible_capacity() {
        let mut duplicate = manifest();
        duplicate.tools.push(duplicate.tools[0].clone());
        assert!(duplicate.validate().is_err());

        let mut impossible = manifest();
        impossible.capacity.available_sessions = 5;
        assert!(impossible.validate().is_err());
    }

    #[test]
    fn capability_name_deserialization_cannot_bypass_validation() {
        assert!(serde_json::from_str::<CapabilityName>(r#""../../secret""#).is_err());
        let oversized = serde_json::to_string(&"a".repeat(65)).unwrap();
        assert!(serde_json::from_str::<CapabilityName>(&oversized).is_err());
    }
}
