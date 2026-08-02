use crate::validation::validate_slug;
use crate::{
    CapabilityManifest, CapabilityVisibility, Capacity, CthulhuId, IncarnationId, ProtocolVersion,
    TentacleId, Timestamp, ValidationError, ValidationErrorKind, XmtpInboxRef,
};
use serde::{Deserialize, Serialize};

pub type TentacleVisibility = CapabilityVisibility;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct XmtpEndpoint {
    pub inbox_id: XmtpInboxRef,
    pub network: String,
}

impl XmtpEndpoint {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_slug("tentacle.xmtpEndpoint.network", &self.network, 32)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Incarnation {
    pub id: IncarnationId,
    pub generation: u64,
}

impl Incarnation {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.generation == 0 {
            return Err(ValidationError::new(
                "tentacle.incarnation.generation",
                ValidationErrorKind::OutOfRange,
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TentacleLifecycle {
    Starting,
    Ready,
    Draining,
    Unavailable,
    Stopped,
}

impl TentacleLifecycle {
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Starting,
                Self::Ready | Self::Unavailable | Self::Stopped
            ) | (
                Self::Ready,
                Self::Draining | Self::Unavailable | Self::Stopped
            ) | (Self::Draining, Self::Unavailable | Self::Stopped)
                | (Self::Unavailable, Self::Ready | Self::Stopped)
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HealthStatus {
    Healthy,
    Suspect,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TentacleHealth {
    pub status: HealthStatus,
    pub observed_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tentacle {
    pub id: TentacleId,
    pub owner: CthulhuId,
    pub xmtp_endpoint: XmtpEndpoint,
    pub incarnation: Incarnation,
    pub lifecycle: TentacleLifecycle,
    pub capabilities: CapabilityManifest,
    pub health: TentacleHealth,
    pub capacity: Capacity,
    /// Current load in thousandths, from 0 (idle) through 1,000 (fully allocated).
    pub current_load_per_mille: u16,
    pub visibility: TentacleVisibility,
    pub protocol_version: ProtocolVersion,
    pub last_heartbeat: Timestamp,
}

impl Tentacle {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.xmtp_endpoint.validate()?;
        self.incarnation.validate()?;
        self.capabilities.validate()?;
        self.capacity.validate()?;
        self.protocol_version.require_supported()?;
        if self.current_load_per_mille > 1_000 {
            return Err(ValidationError::new(
                "tentacle.currentLoadPerMille",
                ValidationErrorKind::OutOfRange,
            ));
        }
        if self.capacity != self.capabilities.capacity {
            return Err(ValidationError::new(
                "tentacle.capacity",
                ValidationErrorKind::InvalidFormat,
            ));
        }
        if self.visibility != self.capabilities.visibility {
            return Err(ValidationError::new(
                "tentacle.visibility",
                ValidationErrorKind::InvalidFormat,
            ));
        }
        if self.health.observed_at > self.last_heartbeat {
            return Err(ValidationError::new(
                "tentacle.health.observedAt",
                ValidationErrorKind::OutOfRange,
            ));
        }
        Ok(())
    }

    pub fn transition(
        &mut self,
        next: TentacleLifecycle,
        at: Timestamp,
    ) -> Result<(), ValidationError> {
        if !self.lifecycle.can_transition_to(next) || at < self.last_heartbeat {
            return Err(ValidationError::new(
                "tentacle.lifecycle",
                ValidationErrorKind::InvalidLifecycleTransition,
            ));
        }
        self.lifecycle = next;
        self.last_heartbeat = at;
        Ok(())
    }

    /// Apply authenticated lifecycle data while rejecting older or conflicting incarnations.
    pub fn apply_update(
        &mut self,
        update: &TentacleLifecycleUpdate,
    ) -> Result<(), ValidationError> {
        update.validate()?;
        if update.tentacle_id != self.id || update.owner != self.owner {
            return Err(ValidationError::new(
                "tentacle.update.sender",
                ValidationErrorKind::SenderMismatch,
            ));
        }
        if update.incarnation.generation < self.incarnation.generation
            || (update.incarnation.generation == self.incarnation.generation
                && update.incarnation.id != self.incarnation.id)
            || update.last_heartbeat < self.last_heartbeat
        {
            return Err(ValidationError::new(
                "tentacle.update.incarnation",
                ValidationErrorKind::StaleIncarnation,
            ));
        }

        if update.incarnation.generation > self.incarnation.generation {
            if update.lifecycle != TentacleLifecycle::Starting {
                return Err(ValidationError::new(
                    "tentacle.update.lifecycle",
                    ValidationErrorKind::InvalidLifecycleTransition,
                ));
            }
            self.incarnation = update.incarnation.clone();
            self.lifecycle = update.lifecycle;
        } else if update.lifecycle != self.lifecycle {
            if !self.lifecycle.can_transition_to(update.lifecycle) {
                return Err(ValidationError::new(
                    "tentacle.update.lifecycle",
                    ValidationErrorKind::InvalidLifecycleTransition,
                ));
            }
            self.lifecycle = update.lifecycle;
        }

        self.health = update.health;
        self.current_load_per_mille = update.current_load_per_mille;
        self.last_heartbeat = update.last_heartbeat;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TentacleLifecycleUpdate {
    pub tentacle_id: TentacleId,
    pub owner: CthulhuId,
    pub incarnation: Incarnation,
    pub lifecycle: TentacleLifecycle,
    pub health: TentacleHealth,
    pub current_load_per_mille: u16,
    pub last_heartbeat: Timestamp,
}

impl TentacleLifecycleUpdate {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.incarnation.validate()?;
        if self.current_load_per_mille > 1_000 || self.health.observed_at > self.last_heartbeat {
            return Err(ValidationError::new(
                "tentacle.update",
                ValidationErrorKind::OutOfRange,
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CapabilityName, InferenceLocation, MemoryMode, PrivacyProperty, TrustMechanism};

    fn time(seconds: i64) -> Timestamp {
        Timestamp::from_unix_seconds(seconds).unwrap()
    }

    fn tentacle() -> Tentacle {
        let capacity = Capacity {
            max_concurrent_sessions: 4,
            available_sessions: 4,
            max_context_tokens: 16_384,
        };
        Tentacle {
            id: TentacleId::new("tentacle_home").unwrap(),
            owner: CthulhuId::new("cthulhu_archivist").unwrap(),
            xmtp_endpoint: XmtpEndpoint {
                inbox_id: XmtpInboxRef::new("012345abcdef").unwrap(),
                network: "dev".into(),
            },
            incarnation: Incarnation {
                id: IncarnationId::new("incarnation_first").unwrap(),
                generation: 1,
            },
            lifecycle: TentacleLifecycle::Starting,
            capabilities: CapabilityManifest {
                schema_version: ProtocolVersion::V1_0,
                protocol_versions: vec![ProtocolVersion::V1_0],
                model_classes: vec![CapabilityName::new("text-chat").unwrap()],
                context_limit_tokens: 16_384,
                tools: vec![],
                memory_modes: vec![MemoryMode::LocalContact],
                privacy_properties: vec![PrivacyProperty::NoCouncilContent],
                inference_location: InferenceLocation::Local,
                capacity,
                visibility: CapabilityVisibility::Council,
                supported_trust_mechanisms: vec![TrustMechanism::LocalAllowlist],
            },
            health: TentacleHealth {
                status: HealthStatus::Healthy,
                observed_at: time(100),
            },
            capacity,
            current_load_per_mille: 0,
            visibility: CapabilityVisibility::Council,
            protocol_version: ProtocolVersion::V1_0,
            last_heartbeat: time(100),
        }
    }

    #[test]
    fn lifecycle_accepts_only_explicit_transitions() {
        let mut tentacle = tentacle();
        tentacle.validate().unwrap();
        tentacle
            .transition(TentacleLifecycle::Ready, time(101))
            .unwrap();
        tentacle
            .transition(TentacleLifecycle::Draining, time(102))
            .unwrap();
        assert!(
            tentacle
                .transition(TentacleLifecycle::Ready, time(103))
                .is_err()
        );
        tentacle
            .transition(TentacleLifecycle::Stopped, time(104))
            .unwrap();
        assert!(
            tentacle
                .transition(TentacleLifecycle::Starting, time(105))
                .is_err()
        );
    }

    #[test]
    fn stale_incarnation_cannot_revive_tentacle() {
        let mut tentacle = tentacle();
        let replacement = TentacleLifecycleUpdate {
            tentacle_id: tentacle.id.clone(),
            owner: tentacle.owner.clone(),
            incarnation: Incarnation {
                id: IncarnationId::new("incarnation_second").unwrap(),
                generation: 2,
            },
            lifecycle: TentacleLifecycle::Starting,
            health: TentacleHealth {
                status: HealthStatus::Healthy,
                observed_at: time(200),
            },
            current_load_per_mille: 0,
            last_heartbeat: time(200),
        };
        tentacle.apply_update(&replacement).unwrap();

        let stale = TentacleLifecycleUpdate {
            incarnation: Incarnation {
                id: IncarnationId::new("incarnation_first").unwrap(),
                generation: 1,
            },
            lifecycle: TentacleLifecycle::Ready,
            last_heartbeat: time(201),
            ..replacement
        };
        let error = tentacle.apply_update(&stale).unwrap_err();
        assert_eq!(error.kind(), &ValidationErrorKind::StaleIncarnation);
        assert_eq!(tentacle.incarnation.generation, 2);
        assert_eq!(tentacle.owner.as_str(), "cthulhu_archivist");
    }
}
