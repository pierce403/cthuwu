use crate::{ValidationError, ValidationErrorKind};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};

/// UTC Unix time in whole seconds, supplied by the caller's injected clock.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(i64);

impl Timestamp {
    pub const MIN: Self = Self(0);

    pub fn from_unix_seconds(seconds: i64) -> Result<Self, ValidationError> {
        if seconds < 0 {
            return Err(ValidationError::new(
                "timestamp",
                ValidationErrorKind::OutOfRange,
            ));
        }
        Ok(Self(seconds))
    }

    pub const fn from_unix_seconds_unchecked(seconds: i64) -> Self {
        Self(seconds)
    }

    pub const fn as_unix_seconds(self) -> i64 {
        self.0
    }

    pub fn checked_add(self, seconds: i64) -> Result<Self, ValidationError> {
        let value = self
            .0
            .checked_add(seconds)
            .ok_or_else(|| ValidationError::new("timestamp", ValidationErrorKind::OutOfRange))?;
        Self::from_unix_seconds(value)
    }
}

impl Serialize for Timestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i64(self.0)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = i64::deserialize(deserializer)?;
        Self::from_unix_seconds(value).map_err(serde::de::Error::custom)
    }
}

/// A semantic Council protocol version serialized as `major.minor`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const V1_0: Self = Self { major: 1, minor: 0 };

    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    pub fn require_supported(self) -> Result<(), ValidationError> {
        if self != Self::V1_0 {
            return Err(ValidationError::new(
                "version",
                ValidationErrorKind::Unsupported,
            ));
        }
        Ok(())
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for ProtocolVersion {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (major, minor) = value
            .split_once('.')
            .ok_or_else(|| ValidationError::new("version", ValidationErrorKind::InvalidFormat))?;
        if minor.contains('.') || major.is_empty() || minor.is_empty() {
            return Err(ValidationError::new(
                "version",
                ValidationErrorKind::InvalidFormat,
            ));
        }
        let major = major
            .parse::<u16>()
            .map_err(|_| ValidationError::new("version", ValidationErrorKind::InvalidFormat))?;
        let minor = minor
            .parse::<u16>()
            .map_err(|_| ValidationError::new("version", ValidationErrorKind::InvalidFormat))?;
        Ok(Self { major, minor })
    }
}

impl Serialize for ProtocolVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_has_stable_string_wire_format() {
        let encoded = serde_json::to_string(&ProtocolVersion::V1_0).unwrap();
        assert_eq!(encoded, r#""1.0""#);
        assert_eq!(
            serde_json::from_str::<ProtocolVersion>(&encoded).unwrap(),
            ProtocolVersion::V1_0
        );
        assert!("1".parse::<ProtocolVersion>().is_err());
        assert!(ProtocolVersion::new(2, 0).require_supported().is_err());
    }

    #[test]
    fn timestamps_are_non_negative_and_checked() {
        assert!(Timestamp::from_unix_seconds(-1).is_err());
        let time = Timestamp::from_unix_seconds(42).unwrap();
        assert_eq!(time.checked_add(8).unwrap().as_unix_seconds(), 50);
    }
}
