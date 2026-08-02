use crate::liveness::HealthStatus;
use cthuwu_protocol::{
    CthulhuId, Incarnation, InferenceLocation, MemoryMode, PrivacyProperty, ProtocolVersion,
    RequestId, SessionId, Tentacle, TentacleId, TentacleLifecycle,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

const MAX_REQUIREMENTS: usize = 64;
const MAX_CANDIDATES: usize = 1_024;
const MAX_ADVERTISED_VALUES: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRequirements {
    pub model_classes: BTreeSet<String>,
    pub tools: BTreeSet<String>,
    pub memory_modes: BTreeSet<String>,
    pub privacy_properties: BTreeSet<String>,
    pub protocol_versions: BTreeSet<ProtocolVersion>,
    pub require_local_inference: bool,
    pub minimum_context_tokens: u32,
}

impl CapabilityRequirements {
    pub fn validate(&self) -> Result<(), RoutingError> {
        for collection in [
            &self.model_classes,
            &self.tools,
            &self.memory_modes,
            &self.privacy_properties,
        ] {
            if collection.len() > MAX_REQUIREMENTS
                || collection
                    .iter()
                    .any(|value| !valid_capability_label(value))
            {
                return Err(RoutingError::InvalidRequest(
                    "capability requirement is invalid or unbounded",
                ));
            }
        }
        if self.protocol_versions.len() > 16 {
            return Err(RoutingError::InvalidRequest(
                "too many protocol requirements",
            ));
        }
        if self
            .protocol_versions
            .iter()
            .any(|version| version.require_supported().is_err())
            || self.minimum_context_tokens > 100_000_000
        {
            return Err(RoutingError::InvalidRequest(
                "protocol or context requirement is outside its bounds",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustPolicy {
    pub require_allowlisted: bool,
    pub require_registry_association: bool,
    pub minimum_reputation: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingRequest {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub requirements: CapabilityRequirements,
    pub explicit_cthulhu: Option<CthulhuId>,
    pub explicit_tentacle: Option<TentacleId>,
    pub affinity_tentacle: Option<TentacleId>,
    pub home_tentacle: Option<TentacleId>,
    pub user_owned_tentacles: BTreeSet<TentacleId>,
    pub trust_policy: TrustPolicy,
    pub maximum_load_percent: u8,
    pub expires_at: u64,
}

impl RoutingRequest {
    pub fn validate(&self, now: u64) -> Result<(), RoutingError> {
        self.requirements.validate()?;
        if self.expires_at <= now {
            return Err(RoutingError::Expired);
        }
        if self.maximum_load_percent > 100 || self.user_owned_tentacles.len() > 128 {
            return Err(RoutingError::InvalidRequest(
                "routing limits are outside their bounds",
            ));
        }
        if self
            .trust_policy
            .minimum_reputation
            .is_some_and(|value| !(-1_000..=1_000).contains(&value))
        {
            return Err(RoutingError::InvalidRequest(
                "reputation requirement is outside its provenance scale",
            ));
        }
        Ok(())
    }
}

/// A normalized, non-secret routing view derived from an advertised capability manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteCandidate {
    pub cthulhu_id: CthulhuId,
    pub tentacle_id: TentacleId,
    pub incarnation: Incarnation,
    pub endpoint: String,
    pub health: HealthStatus,
    pub ready: bool,
    pub model_classes: BTreeSet<String>,
    pub tools: BTreeSet<String>,
    pub memory_modes: BTreeSet<String>,
    pub privacy_properties: BTreeSet<String>,
    pub protocol_versions: BTreeSet<ProtocolVersion>,
    pub local_inference: bool,
    pub context_tokens: u32,
    pub capacity: u32,
    pub current_load: u32,
    pub allowlisted: bool,
    pub registry_associated: bool,
    /// A bounded, provenance-selected signal. It is not a universal trust score.
    pub selected_reputation: i32,
}

impl RouteCandidate {
    pub fn from_tentacle(
        tentacle: &Tentacle,
        allowlisted: bool,
        registry_associated: bool,
        selected_reputation: i32,
    ) -> Self {
        let capacity = tentacle.capacity.max_concurrent_sessions;
        let load_from_capacity = capacity.saturating_sub(tentacle.capacity.available_sessions);
        let load_from_ratio = (u64::from(capacity)
            .saturating_mul(u64::from(tentacle.current_load_per_mille))
            / 1_000) as u32;
        Self {
            cthulhu_id: tentacle.owner.clone(),
            tentacle_id: tentacle.id.clone(),
            incarnation: tentacle.incarnation.clone(),
            endpoint: format!(
                "xmtp:{}:{}",
                tentacle.xmtp_endpoint.network, tentacle.xmtp_endpoint.inbox_id
            ),
            health: tentacle.health.status,
            ready: tentacle.lifecycle == TentacleLifecycle::Ready,
            model_classes: tentacle
                .capabilities
                .model_classes
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
            tools: tentacle
                .capabilities
                .tools
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
            memory_modes: tentacle
                .capabilities
                .memory_modes
                .iter()
                .map(|value| memory_mode_label(*value).to_owned())
                .collect(),
            privacy_properties: tentacle
                .capabilities
                .privacy_properties
                .iter()
                .map(|value| privacy_label(*value).to_owned())
                .collect(),
            protocol_versions: tentacle
                .capabilities
                .protocol_versions
                .iter()
                .copied()
                .collect(),
            local_inference: tentacle.capabilities.inference_location == InferenceLocation::Local,
            context_tokens: tentacle
                .capabilities
                .context_limit_tokens
                .min(u64::from(u32::MAX)) as u32,
            capacity,
            current_load: load_from_capacity.max(load_from_ratio).min(capacity),
            allowlisted,
            registry_associated,
            selected_reputation,
        }
    }

    pub fn load_percent(&self) -> u8 {
        if self.capacity == 0 {
            return 100;
        }
        ((self.current_load.saturating_mul(100) / self.capacity).min(100)) as u8
    }
}

fn memory_mode_label(mode: MemoryMode) -> &'static str {
    match mode {
        MemoryMode::None => "none",
        MemoryMode::Session => "session",
        MemoryMode::LocalContact => "local-contact",
        MemoryMode::LocalSemantic => "local-semantic",
    }
}

fn privacy_label(property: PrivacyProperty) -> &'static str {
    match property {
        PrivacyProperty::NoCouncilContent => "no-council-content",
        PrivacyProperty::LocalMemoryOnly => "local-memory-only",
        PrivacyProperty::NoRemoteInference => "no-remote-inference",
        PrivacyProperty::EphemeralSession => "ephemeral-session",
        PrivacyProperty::OperatorControlledRetention => "operator-controlled-retention",
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateExplanation {
    pub cthulhu_id: CthulhuId,
    pub tentacle_id: TentacleId,
    pub eligible: bool,
    pub score: i64,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingDecision {
    pub request_id: RequestId,
    pub selected_cthulhu: CthulhuId,
    pub selected_tentacle: TentacleId,
    pub selected_incarnation: Incarnation,
    pub endpoint: String,
    pub explanation: Vec<CandidateExplanation>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RoutingError {
    #[error("routing request is invalid: {0}")]
    InvalidRequest(&'static str),
    #[error("routing request has expired")]
    Expired,
    #[error("too many routing candidates")]
    TooManyCandidates,
    #[error("the same Cthulhu/Tentacle candidate was advertised more than once")]
    DuplicateCandidate,
    #[error("no candidate satisfies the hard routing requirements")]
    NoEligibleCandidate,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RoutingEngine;

impl RoutingEngine {
    pub fn route(
        &self,
        request: &RoutingRequest,
        candidates: &[RouteCandidate],
        now: u64,
    ) -> Result<RoutingDecision, RoutingError> {
        request.validate(now)?;
        if candidates.len() > MAX_CANDIDATES {
            return Err(RoutingError::TooManyCandidates);
        }
        let mut identities = BTreeSet::new();
        for candidate in candidates {
            if !identities.insert((candidate.cthulhu_id.clone(), candidate.tentacle_id.clone())) {
                return Err(RoutingError::DuplicateCandidate);
            }
        }

        let mut explanations = candidates
            .iter()
            .map(|candidate| evaluate_candidate(request, candidate))
            .collect::<Vec<_>>();
        explanations.sort_by(|left, right| {
            right
                .eligible
                .cmp(&left.eligible)
                .then_with(|| right.score.cmp(&left.score))
                .then_with(|| left.cthulhu_id.cmp(&right.cthulhu_id))
                .then_with(|| left.tentacle_id.cmp(&right.tentacle_id))
        });
        let selected = explanations
            .iter()
            .find(|candidate| candidate.eligible)
            .ok_or(RoutingError::NoEligibleCandidate)?;
        let candidate = candidates
            .iter()
            .find(|candidate| {
                candidate.cthulhu_id == selected.cthulhu_id
                    && candidate.tentacle_id == selected.tentacle_id
            })
            .expect("routing explanation must correspond to a candidate");

        Ok(RoutingDecision {
            request_id: request.request_id.clone(),
            selected_cthulhu: candidate.cthulhu_id.clone(),
            selected_tentacle: candidate.tentacle_id.clone(),
            selected_incarnation: candidate.incarnation.clone(),
            endpoint: candidate.endpoint.clone(),
            explanation: explanations,
        })
    }
}

fn evaluate_candidate(
    request: &RoutingRequest,
    candidate: &RouteCandidate,
) -> CandidateExplanation {
    let mut hard_failures = Vec::new();
    for (values, label) in [
        (&candidate.model_classes, "model capabilities"),
        (&candidate.tools, "tools"),
        (&candidate.memory_modes, "memory modes"),
        (&candidate.privacy_properties, "privacy properties"),
    ] {
        if values.len() > MAX_ADVERTISED_VALUES
            || values.iter().any(|value| !valid_capability_label(value))
        {
            hard_failures.push(format!("advertised {label} are invalid or unbounded"));
        }
    }
    if candidate.protocol_versions.len() > 16
        || candidate
            .protocol_versions
            .iter()
            .any(|version| version.require_supported().is_err())
    {
        hard_failures.push("advertised protocol versions are invalid or unbounded".to_owned());
    }
    if candidate.health != HealthStatus::Healthy || !candidate.ready {
        hard_failures.push("Tentacle is not healthy and ready".to_owned());
    }
    if candidate.endpoint.len() > 512
        || candidate.endpoint.trim().is_empty()
        || !candidate.endpoint.starts_with("xmtp:")
        || candidate.endpoint.chars().any(char::is_control)
    {
        hard_failures.push("endpoint is invalid".to_owned());
    }
    if candidate.capacity == 0
        || candidate.capacity > 1_000_000
        || candidate.current_load > candidate.capacity
        || candidate.context_tokens > 100_000_000
        || !(-1_000..=1_000).contains(&candidate.selected_reputation)
    {
        hard_failures.push("capacity advertisement is invalid".to_owned());
    }
    if candidate.load_percent() > request.maximum_load_percent {
        hard_failures.push("load exceeds the request maximum".to_owned());
    }
    if request.requirements.require_local_inference && !candidate.local_inference {
        hard_failures.push("local inference is required".to_owned());
    }
    if candidate.context_tokens < request.requirements.minimum_context_tokens {
        hard_failures.push("context limit is too small".to_owned());
    }
    for (required, offered, label) in [
        (
            &request.requirements.model_classes,
            &candidate.model_classes,
            "model capability",
        ),
        (&request.requirements.tools, &candidate.tools, "tool"),
        (
            &request.requirements.memory_modes,
            &candidate.memory_modes,
            "memory mode",
        ),
        (
            &request.requirements.privacy_properties,
            &candidate.privacy_properties,
            "privacy property",
        ),
    ] {
        if !required.is_subset(offered) {
            hard_failures.push(format!("missing required {label}"));
        }
    }
    if !request
        .requirements
        .protocol_versions
        .is_subset(&candidate.protocol_versions)
    {
        hard_failures.push("protocol version is incompatible".to_owned());
    }
    if request.trust_policy.require_allowlisted && !candidate.allowlisted {
        hard_failures.push("allowlist membership is required".to_owned());
    }
    if request.trust_policy.require_registry_association && !candidate.registry_associated {
        hard_failures.push("verified registry association is required".to_owned());
    }
    if request
        .trust_policy
        .minimum_reputation
        .is_some_and(|minimum| candidate.selected_reputation < minimum)
    {
        hard_failures.push("selected reputation signal is below policy".to_owned());
    }

    if !hard_failures.is_empty() {
        return CandidateExplanation {
            cthulhu_id: candidate.cthulhu_id.clone(),
            tentacle_id: candidate.tentacle_id.clone(),
            eligible: false,
            score: 0,
            reasons: hard_failures,
        };
    }

    let mut score = 0_i64;
    let mut reasons = vec!["all hard requirements satisfied".to_owned()];
    if request.explicit_tentacle.as_ref() == Some(&candidate.tentacle_id) {
        score += 1_000_000_000_000;
        reasons.push("explicit user Tentacle choice".to_owned());
    } else if request.explicit_cthulhu.as_ref() == Some(&candidate.cthulhu_id) {
        score += 500_000_000_000;
        reasons.push("explicit user Cthulhu choice".to_owned());
    }
    if request.affinity_tentacle.as_ref() == Some(&candidate.tentacle_id) {
        score += 10_000_000_000;
        reasons.push("valid session affinity".to_owned());
    }
    if request.home_tentacle.as_ref() == Some(&candidate.tentacle_id) {
        score += 1_000_000_000;
        reasons.push("healthy home Tentacle".to_owned());
    }
    if request
        .user_owned_tentacles
        .contains(&candidate.tentacle_id)
    {
        score += 100_000_000;
        reasons.push("user-owned Tentacle".to_owned());
    }
    let headroom = candidate
        .capacity
        .saturating_sub(candidate.current_load)
        .min(10_000);
    score += i64::from(headroom) * 100_000;
    reasons.push(format!("capacity headroom {headroom}"));
    score += candidate.protocol_versions.len().min(16) as i64 * 10_000;
    if candidate.registry_associated {
        score += 5_000;
        reasons.push("verified registry association".to_owned());
    }
    if candidate.allowlisted {
        score += 5_000;
        reasons.push("operator allowlist".to_owned());
    }
    score += i64::from(candidate.selected_reputation.clamp(-1_000, 1_000)) * 10;
    score += i64::from(100_u8.saturating_sub(candidate.load_percent()));
    reasons.push(format!("current load {}%", candidate.load_percent()));

    CandidateExplanation {
        cthulhu_id: candidate.cthulhu_id.clone(),
        tentacle_id: candidate.tentacle_id.clone(),
        eligible: true,
        score,
        reasons,
    }
}

fn valid_capability_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b':')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn request() -> RoutingRequest {
        RoutingRequest {
            request_id: RequestId::new("request_route_1").unwrap(),
            session_id: SessionId::new("session_user_1").unwrap(),
            requirements: CapabilityRequirements {
                model_classes: labels(&["text-chat"]),
                tools: labels(&["protocol-self-test"]),
                memory_modes: labels(&["local-contact"]),
                privacy_properties: labels(&["no-council-content"]),
                protocol_versions: [ProtocolVersion::V1_0].into_iter().collect(),
                require_local_inference: true,
                minimum_context_tokens: 8_192,
            },
            explicit_cthulhu: None,
            explicit_tentacle: None,
            affinity_tentacle: None,
            home_tentacle: None,
            user_owned_tentacles: BTreeSet::new(),
            trust_policy: TrustPolicy::default(),
            maximum_load_percent: 90,
            expires_at: 200,
        }
    }

    fn candidate(name: &str, load: u32) -> RouteCandidate {
        RouteCandidate {
            cthulhu_id: CthulhuId::new(format!("cthulhu_{name}")).unwrap(),
            tentacle_id: TentacleId::new(format!("tentacle_{name}")).unwrap(),
            incarnation: Incarnation {
                id: cthuwu_protocol::IncarnationId::new(format!("incarnation_{name}")).unwrap(),
                generation: 1,
            },
            endpoint: format!("xmtp:{name}"),
            health: HealthStatus::Healthy,
            ready: true,
            model_classes: labels(&["text-chat"]),
            tools: labels(&["protocol-self-test"]),
            memory_modes: labels(&["local-contact"]),
            privacy_properties: labels(&["no-council-content", "local-memory-only"]),
            protocol_versions: [ProtocolVersion::V1_0].into_iter().collect(),
            local_inference: true,
            context_tokens: 32_768,
            capacity: 10,
            current_load: load,
            allowlisted: true,
            registry_associated: true,
            selected_reputation: 10,
        }
    }

    #[test]
    fn hard_requirements_filter_before_scoring() {
        let mut impossible = candidate("fast", 0);
        impossible.local_inference = false;
        let valid = candidate("private", 5);
        let decision = RoutingEngine
            .route(&request(), &[impossible, valid.clone()], 100)
            .unwrap();
        assert_eq!(decision.selected_tentacle, valid.tentacle_id);
        assert!(decision.explanation.iter().any(|item| {
            !item.eligible
                && item
                    .reasons
                    .contains(&"local inference is required".to_owned())
        }));
    }

    #[test]
    fn explicit_choice_precedes_affinity_and_load() {
        let explicit = candidate("explicit", 8);
        let affinity = candidate("affinity", 0);
        let mut request = request();
        request.explicit_tentacle = Some(explicit.tentacle_id.clone());
        request.affinity_tentacle = Some(affinity.tentacle_id.clone());
        let decision = RoutingEngine
            .route(&request, &[affinity, explicit.clone()], 100)
            .unwrap();
        assert_eq!(decision.selected_tentacle, explicit.tentacle_id);
        assert!(
            decision.explanation[0]
                .reasons
                .iter()
                .any(|reason| reason == "explicit user Tentacle choice")
        );
    }

    #[test]
    fn deterministic_tie_breaker_uses_ids() {
        let alpha = candidate("alpha", 1);
        let beta = candidate("beta", 1);
        let decision = RoutingEngine
            .route(&request(), &[beta, alpha.clone()], 100)
            .unwrap();
        assert_eq!(decision.selected_tentacle, alpha.tentacle_id);
    }

    #[test]
    fn affinity_is_ignored_when_candidate_is_unhealthy() {
        let mut stale = candidate("stale", 0);
        stale.health = HealthStatus::Suspect;
        let healthy = candidate("healthy", 4);
        let mut request = request();
        request.affinity_tentacle = Some(stale.tentacle_id.clone());
        let decision = RoutingEngine
            .route(&request, &[stale, healthy.clone()], 100)
            .unwrap();
        assert_eq!(decision.selected_tentacle, healthy.tentacle_id);
    }

    #[test]
    fn duplicate_or_unbounded_candidate_advertisements_do_not_route() {
        let duplicate = candidate("same", 1);
        assert_eq!(
            RoutingEngine
                .route(&request(), &[duplicate.clone(), duplicate.clone()], 100)
                .unwrap_err(),
            RoutingError::DuplicateCandidate
        );

        let mut malformed = duplicate;
        malformed
            .tools
            .extend((0..65).map(|index| format!("tool-{index}")));
        let error = RoutingEngine
            .route(&request(), &[malformed], 100)
            .unwrap_err();
        assert_eq!(error, RoutingError::NoEligibleCandidate);
    }
}
