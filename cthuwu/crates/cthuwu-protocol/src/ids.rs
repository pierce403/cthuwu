use crate::validation::{bounded_text, validate_slug};
use crate::{ValidationError, ValidationErrorKind};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};

const MAX_ID_BYTES: usize = 96;
const MAX_SUFFIX_BYTES: usize = 64;

fn validate_prefixed_id(field: &str, value: &str, prefix: &str) -> Result<(), ValidationError> {
    bounded_text(field, value, MAX_ID_BYTES)?;
    // Evolution v1 durably generated founder IDs with `tentacle-`; retain that exact serialized
    // ID as a compatibility form so it can key the same Tentacle's ERC-8004 binding. New protocol
    // IDs continue to use the canonical `tentacle_` prefix. No other ID namespace gains an alias.
    let suffix = value.strip_prefix(prefix).or_else(|| {
        (prefix == "tentacle_")
            .then(|| value.strip_prefix("tentacle-"))
            .flatten()
    });
    let Some(suffix) = suffix else {
        return Err(ValidationError::new(
            field,
            ValidationErrorKind::InvalidFormat,
        ));
    };
    validate_slug(field, suffix, MAX_SUFFIX_BYTES)
}

macro_rules! define_id {
    ($(#[$metadata:meta])* $name:ident, $prefix:literal, $field:literal) => {
        $(#[$metadata])*
        #[doc = concat!("A validated `", $prefix, "…` protocol identifier.")]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
                let value = value.into();
                validate_prefixed_id($field, &value, Self::PREFIX)?;
                Ok(Self(value))
            }

            pub fn parse(value: &str) -> Result<Self, ValidationError> {
                Self::new(value)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl FromStr for $name {
            type Err = ValidationError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

define_id!(
    /// Legacy v1 Council coordination namespace retained for wire and snapshot compatibility.
    /// It does not identify an individual Cthulhu and must not key an ERC-8004 identity.
    CthulhuId, "cthulhu_", "cthulhuId"
);
define_id!(
    /// Stable identity of one independently operated Tentacle. It survives incarnation restarts
    /// and is the only protocol identifier that may key that Tentacle's ERC-8004 identity.
    TentacleId, "tentacle_", "tentacleId"
);
define_id!(CouncilId, "council_", "councilId");
define_id!(SessionId, "session_", "sessionId");
define_id!(RequestId, "request_", "requestId");
define_id!(LeaseId, "lease_", "leaseId");
define_id!(ProposalId, "proposal_", "proposalId");
define_id!(MessageId, "msg_", "messageId");
define_id!(
    /// One runtime generation of a durable Tentacle, never a new agent identity.
    IncarnationId, "incarnation_", "incarnationId"
);
define_id!(PropagationId, "propagation_", "propagationId");
define_id!(InvitationId, "invite_", "invitationId");
define_id!(AcknowledgementId, "ack_", "acknowledgementId");

/// A content-addressed SHA-256 digest. The wire form is `sha256:` plus 64 lowercase hex digits.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(ValidationError::new(
                "contentHash",
                ValidationErrorKind::InvalidFormat,
            ));
        };
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(ValidationError::new(
                "contentHash",
                ValidationErrorKind::InvalidFormat,
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for ContentHash {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for ContentHash {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// A transport-independent reference to an XMTP inbox ID.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct XmtpInboxRef(String);

impl XmtpInboxRef {
    pub const MIN_HEX_BYTES: usize = 12;
    pub const MAX_HEX_BYTES: usize = 128;

    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        if value.len() < Self::MIN_HEX_BYTES
            || value.len() > Self::MAX_HEX_BYTES
            || value.len() % 2 != 0
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(ValidationError::new(
                "xmtpInbox",
                ValidationErrorKind::InvalidFormat,
            ));
        }
        Ok(Self(value))
    }

    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        Self::new(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for XmtpInboxRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for XmtpInboxRef {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for XmtpInboxRef {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for XmtpInboxRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// A chain-, deployment-, ABI-, and registry-version-neutral public identity reference.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryRef {
    pub registry: String,
    pub reference: String,
}

impl RegistryRef {
    pub fn new(
        registry: impl Into<String>,
        reference: impl Into<String>,
    ) -> Result<Self, ValidationError> {
        let value = Self {
            registry: registry.into(),
            reference: reference.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_slug("registry.registry", &self.registry, 64)?;
        bounded_text("registry.reference", &self.reference, 256)?;
        if self.reference.chars().any(char::is_whitespace)
            || !self.reference.is_ascii()
            || self.reference.contains("..")
            || self.reference.contains(['\\', '\0'])
        {
            return Err(ValidationError::new(
                "registry.reference",
                ValidationErrorKind::InvalidFormat,
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_validate_prefix_grammar_and_bounds() {
        assert_eq!(
            CthulhuId::new("cthulhu_archivist").unwrap().as_str(),
            "cthulhu_archivist"
        );
        for invalid in [
            "tentacle_archivist",
            "cthulhu_",
            "cthulhu_Upper",
            "cthulhu_-bad",
            "cthulhu_bad__suffix",
            "cthulhu_bad/thing",
        ] {
            assert!(CthulhuId::new(invalid).is_err(), "accepted {invalid}");
        }
        let long = format!("cthulhu_{}", "a".repeat(MAX_SUFFIX_BYTES + 1));
        assert!(CthulhuId::new(long).is_err());
    }

    #[test]
    fn deserialization_does_not_bypass_validation() {
        let error = serde_json::from_str::<TentacleId>(r#""cthulhu_wrong""#).unwrap_err();
        assert!(error.to_string().contains("tentacleId"));
    }

    #[test]
    fn legacy_evolution_tentacle_ids_remain_the_same_durable_identity() {
        assert_eq!(
            TentacleId::new("tentacle-archive").unwrap().as_str(),
            "tentacle-archive"
        );
        assert_eq!(
            TentacleId::new("tentacle_archive").unwrap().as_str(),
            "tentacle_archive"
        );
        assert!(TentacleId::new("tentacle.bad").is_err());
    }

    #[test]
    fn xmtp_and_registry_references_are_bounded_and_neutral() {
        let inbox = XmtpInboxRef::new("012345abcdef").unwrap();
        assert_eq!(serde_json::to_string(&inbox).unwrap(), r#""012345abcdef""#);
        assert!(XmtpInboxRef::new("ABCDEF012345").is_err());
        assert!(XmtpInboxRef::new("../../escape").is_err());

        let registry = RegistryRef::new("erc-8004", "eip155:any:agent:42").unwrap();
        assert_eq!(registry.registry, "erc-8004");
        assert!(RegistryRef::new("erc-8004", "../private").is_err());
    }
}
