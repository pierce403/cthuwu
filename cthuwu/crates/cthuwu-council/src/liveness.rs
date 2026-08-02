use crate::clock::Clock;
pub use cthuwu_protocol::HealthStatus;
use cthuwu_protocol::{
    Tentacle, TentacleId, TentacleLifecycle, TentacleLifecycleUpdate, Timestamp, ValidationError,
};
use std::collections::BTreeMap;
use thiserror::Error;

const MAX_FUTURE_HEARTBEAT_SKEW_SECONDS: u64 = 300;

#[derive(Debug, Error)]
pub enum LivenessError {
    #[error("invalid Tentacle update: {0}")]
    Invalid(#[from] ValidationError),
    #[error("Tentacle is not known")]
    UnknownTentacle,
    #[error("liveness thresholds are invalid")]
    InvalidThresholds,
}

/// Tracks authenticated Tentacle announcements and heartbeats against an injected clock.
pub struct LivenessTracker<C: Clock> {
    clock: C,
    suspect_after_seconds: u64,
    unavailable_after_seconds: u64,
    tentacles: BTreeMap<TentacleId, Tentacle>,
}

impl<C: Clock> LivenessTracker<C> {
    pub fn new(
        clock: C,
        suspect_after_seconds: u64,
        unavailable_after_seconds: u64,
    ) -> Result<Self, LivenessError> {
        if suspect_after_seconds == 0 || unavailable_after_seconds <= suspect_after_seconds {
            return Err(LivenessError::InvalidThresholds);
        }
        Ok(Self {
            clock,
            suspect_after_seconds,
            unavailable_after_seconds,
            tentacles: BTreeMap::new(),
        })
    }

    pub fn announce(&mut self, announced: Tentacle) -> Result<(), LivenessError> {
        announced.validate()?;
        self.validate_heartbeat_time(announced.last_heartbeat)?;
        if let Some(current) = self.tentacles.get_mut(&announced.id) {
            let update = TentacleLifecycleUpdate {
                tentacle_id: announced.id.clone(),
                owner: announced.owner.clone(),
                incarnation: announced.incarnation.clone(),
                lifecycle: announced.lifecycle,
                health: announced.health,
                current_load_per_mille: announced.current_load_per_mille,
                last_heartbeat: announced.last_heartbeat,
            };
            current.apply_update(&update)?;
            current.xmtp_endpoint = announced.xmtp_endpoint;
            current.capabilities = announced.capabilities;
            current.capacity = announced.capacity;
            current.visibility = announced.visibility;
            current.protocol_version = announced.protocol_version;
            current.validate()?;
        } else {
            self.tentacles.insert(announced.id.clone(), announced);
        }
        Ok(())
    }

    pub fn heartbeat(&mut self, update: &TentacleLifecycleUpdate) -> Result<(), LivenessError> {
        self.validate_heartbeat_time(update.last_heartbeat)?;
        self.tentacles
            .get_mut(&update.tentacle_id)
            .ok_or(LivenessError::UnknownTentacle)?
            .apply_update(update)?;
        Ok(())
    }

    pub fn assess(&mut self) {
        let now = self.clock.now();
        for tentacle in self.tentacles.values_mut() {
            if tentacle.lifecycle == TentacleLifecycle::Stopped {
                tentacle.health.status = HealthStatus::Unavailable;
                continue;
            }
            let heartbeat = tentacle.last_heartbeat.as_unix_seconds() as u64;
            let age = now.saturating_sub(heartbeat);
            if age >= self.unavailable_after_seconds {
                tentacle.health.status = HealthStatus::Unavailable;
                if tentacle.lifecycle != TentacleLifecycle::Unavailable
                    && tentacle
                        .lifecycle
                        .can_transition_to(TentacleLifecycle::Unavailable)
                {
                    tentacle.lifecycle = TentacleLifecycle::Unavailable;
                }
            } else if age >= self.suspect_after_seconds {
                tentacle.health.status = HealthStatus::Suspect;
            }
        }
    }

    pub fn get(&self, id: &TentacleId) -> Option<&Tentacle> {
        self.tentacles.get(id)
    }

    pub fn all(&self) -> impl Iterator<Item = &Tentacle> {
        self.tentacles.values()
    }

    fn validate_heartbeat_time(&self, heartbeat: Timestamp) -> Result<(), LivenessError> {
        let heartbeat = u64::try_from(heartbeat.as_unix_seconds()).map_err(|_| {
            ValidationError::new(
                "heartbeat",
                cthuwu_protocol::ValidationErrorKind::OutOfRange,
            )
        })?;
        if heartbeat
            > self
                .clock
                .now()
                .saturating_add(MAX_FUTURE_HEARTBEAT_SKEW_SECONDS)
        {
            return Err(ValidationError::new(
                "heartbeat",
                cthuwu_protocol::ValidationErrorKind::OutOfRange,
            )
            .into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use cthuwu_protocol::{
        CapabilityManifest, CapabilityName, CapabilityVisibility, Capacity, CthulhuId, Incarnation,
        IncarnationId, InferenceLocation, MemoryMode, PrivacyProperty, ProtocolVersion,
        TentacleHealth, TrustMechanism, XmtpEndpoint, XmtpInboxRef,
    };

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
            owner: CthulhuId::new("cthulhu_home").unwrap(),
            xmtp_endpoint: XmtpEndpoint {
                inbox_id: XmtpInboxRef::new("012345abcdef").unwrap(),
                network: "dev".to_owned(),
            },
            incarnation: Incarnation {
                id: IncarnationId::new("incarnation_first").unwrap(),
                generation: 1,
            },
            lifecycle: TentacleLifecycle::Ready,
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
    fn injected_clock_marks_suspect_then_unavailable() {
        let clock = ManualClock::new(100);
        let mut tracker = LivenessTracker::new(clock.clone(), 10, 20).unwrap();
        tracker.announce(tentacle()).unwrap();

        clock.set(111);
        tracker.assess();
        assert_eq!(
            tracker
                .get(&TentacleId::new("tentacle_home").unwrap())
                .unwrap()
                .health
                .status,
            HealthStatus::Suspect
        );
        clock.set(121);
        tracker.assess();
        let tracked = tracker
            .get(&TentacleId::new("tentacle_home").unwrap())
            .unwrap();
        assert_eq!(tracked.health.status, HealthStatus::Unavailable);
        assert_eq!(tracked.lifecycle, TentacleLifecycle::Unavailable);
    }

    #[test]
    fn old_incarnation_cannot_revive_unavailable_tentacle() {
        let clock = ManualClock::new(121);
        let mut tracker = LivenessTracker::new(clock, 10, 20).unwrap();
        let original = tentacle();
        tracker.announce(original.clone()).unwrap();
        tracker.assess();

        let stale = TentacleLifecycleUpdate {
            tentacle_id: original.id.clone(),
            owner: original.owner.clone(),
            incarnation: original.incarnation,
            lifecycle: TentacleLifecycle::Ready,
            health: TentacleHealth {
                status: HealthStatus::Healthy,
                observed_at: time(99),
            },
            current_load_per_mille: 0,
            last_heartbeat: time(99),
        };
        assert!(tracker.heartbeat(&stale).is_err());
        assert_eq!(
            tracker.get(&stale.tentacle_id).unwrap().health.status,
            HealthStatus::Unavailable
        );
    }

    #[test]
    fn future_heartbeat_cannot_poison_liveness() {
        let clock = ManualClock::new(100);
        let mut tracker = LivenessTracker::new(clock, 10, 20).unwrap();
        let mut future = tentacle();
        future.last_heartbeat = time(10_000);
        future.health.observed_at = time(10_000);
        assert!(tracker.announce(future).is_err());
        assert!(tracker.all().next().is_none());
    }
}
