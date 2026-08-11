use crate::validation::{bounded_count, bounded_optional_text, bounded_text, unique_text};
use crate::{
    CthulhuId, ProtocolVersion, RegistryRef, TentacleId, ValidationError, ValidationErrorKind,
};
use serde::{Deserialize, Serialize};

const MAX_TRAITS: usize = 16;
const MAX_GOALS: usize = 16;
const MAX_TENTACLES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskTolerance {
    VeryLow,
    Low,
    Moderate,
    High,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrivacyPreference {
    LocalOnly,
    MinimizeDisclosure,
    CouncilLimited,
    PublicByChoice,
}

/// Bounded decision weights used for deterministic policy behavior, never autonomous goal creation.
/// Legacy v1 Council coordination profile retained for serialized-state compatibility.
///
/// Despite the historical name, this is not an identity for an individual Cthulhu: Cthuwu is the
/// singular collective. New durable public identity and registry code must describe each
/// independently operated Tentacle with [`TentacleId`]. The `registry` field here is legacy
/// coordination metadata and must not be interpreted as ownership of a Tentacle's ERC-8004 ID.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionTendencies {
    pub caution: u8,
    pub cooperation: u8,
    pub novelty_seeking: u8,
    pub memory_preservation: u8,
    pub resource_exchange: u8,
    pub independence: u8,
}

impl DecisionTendencies {
    pub fn validate(&self) -> Result<(), ValidationError> {
        for (field, value) in [
            ("personality.decisionTendencies.caution", self.caution),
            (
                "personality.decisionTendencies.cooperation",
                self.cooperation,
            ),
            (
                "personality.decisionTendencies.noveltySeeking",
                self.novelty_seeking,
            ),
            (
                "personality.decisionTendencies.memoryPreservation",
                self.memory_preservation,
            ),
            (
                "personality.decisionTendencies.resourceExchange",
                self.resource_exchange,
            ),
            (
                "personality.decisionTendencies.independence",
                self.independence,
            ),
        ] {
            if value > 100 {
                return Err(ValidationError::new(field, ValidationErrorKind::OutOfRange));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalityProfile {
    pub schema_version: ProtocolVersion,
    pub role: String,
    pub voice: String,
    pub values: Vec<String>,
    pub motivations: Vec<String>,
    pub priorities: Vec<String>,
    pub risk_tolerance: RiskTolerance,
    pub privacy_preference: PrivacyPreference,
    pub decision_tendencies: DecisionTendencies,
    pub standing_concerns: Vec<String>,
}

impl PersonalityProfile {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.schema_version.require_supported()?;
        bounded_text("personality.role", &self.role, 64)?;
        bounded_text("personality.voice", &self.voice, 256)?;
        validate_trait_list("personality.values", &self.values)?;
        validate_trait_list("personality.motivations", &self.motivations)?;
        validate_trait_list("personality.priorities", &self.priorities)?;
        validate_trait_list("personality.standingConcerns", &self.standing_concerns)?;
        self.decision_tendencies.validate()
    }

    pub fn sample(persona: SamplePersona) -> Self {
        persona.profile()
    }

    /// Return a deterministic, explainable policy position without consulting an LLM.
    pub fn policy_position(&self, topic: PolicyTopic) -> PolicyPosition {
        let tendencies = &self.decision_tendencies;
        let score = match topic {
            PolicyTopic::PublishCapabilityMetadata => {
                i16::from(tendencies.cooperation) - i16::from(tendencies.caution)
            }
            PolicyTopic::UseRemoteInference => {
                i16::from(tendencies.novelty_seeking)
                    - i16::from(tendencies.caution)
                    - match self.privacy_preference {
                        PrivacyPreference::LocalOnly => 80,
                        PrivacyPreference::MinimizeDisclosure => 35,
                        PrivacyPreference::CouncilLimited => 10,
                        PrivacyPreference::PublicByChoice => 0,
                    }
            }
            PolicyTopic::PreserveLongTermRecords => {
                i16::from(tendencies.memory_preservation) - i16::from(tendencies.caution) / 3
            }
            PolicyTopic::AcceptExperimentalProtocol => {
                i16::from(tendencies.novelty_seeking) - i16::from(tendencies.caution)
            }
            PolicyTopic::PrioritizeResourceExchange => {
                i16::from(tendencies.resource_exchange) - i16::from(tendencies.independence) / 2
            }
        };
        let stance = if score >= 20 {
            PolicyStance::Support
        } else if score <= -20 {
            PolicyStance::Oppose
        } else {
            PolicyStance::Abstain
        };
        PolicyPosition {
            topic,
            stance,
            score,
            rationale: match topic {
                PolicyTopic::PublishCapabilityMetadata => "cooperation balanced against caution",
                PolicyTopic::UseRemoteInference => {
                    "novelty balanced against caution and privacy preference"
                }
                PolicyTopic::PreserveLongTermRecords => {
                    "memory preservation balanced against retention caution"
                }
                PolicyTopic::AcceptExperimentalProtocol => {
                    "novelty seeking balanced against caution"
                }
                PolicyTopic::PrioritizeResourceExchange => {
                    "resource exchange balanced against independence"
                }
            }
            .to_owned(),
        }
    }
}

fn validate_trait_list(field: &str, values: &[String]) -> Result<(), ValidationError> {
    bounded_count(field, values.len(), MAX_TRAITS)?;
    for value in values {
        bounded_text(field, value, 128)?;
    }
    unique_text(field, values.iter().map(String::as_str))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PolicyTopic {
    PublishCapabilityMetadata,
    UseRemoteInference,
    PreserveLongTermRecords,
    AcceptExperimentalProtocol,
    PrioritizeResourceExchange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PolicyStance {
    Support,
    Oppose,
    Abstain,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyPosition {
    pub topic: PolicyTopic,
    pub stance: PolicyStance,
    pub score: i16,
    pub rationale: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SamplePersona {
    Archivist,
    Hermit,
    Merchant,
    Wanderer,
    Oracle,
    Trickster,
}

impl SamplePersona {
    pub const ALL: [Self; 6] = [
        Self::Archivist,
        Self::Hermit,
        Self::Merchant,
        Self::Wanderer,
        Self::Oracle,
        Self::Trickster,
    ];

    pub fn profile(self) -> PersonalityProfile {
        let (role, voice, values, motivations, priorities, risk, privacy, tendencies, concerns) =
            match self {
                Self::Archivist => (
                    "Archivist",
                    "patient, precise, and grounded in provenance",
                    vec!["continuity", "accuracy", "consent"],
                    vec![
                        "preserve useful knowledge",
                        "explain where claims came from",
                    ],
                    vec!["durable records", "source integrity"],
                    RiskTolerance::Low,
                    PrivacyPreference::CouncilLimited,
                    DecisionTendencies {
                        caution: 70,
                        cooperation: 70,
                        novelty_seeking: 30,
                        memory_preservation: 100,
                        resource_exchange: 55,
                        independence: 45,
                    },
                    vec!["information loss", "provenance gaps"],
                ),
                Self::Hermit => (
                    "Hermit",
                    "quiet, direct, and sparing with disclosure",
                    vec!["privacy", "autonomy", "minimalism"],
                    vec!["keep sensitive work local", "reduce dependencies"],
                    vec!["local inference", "operator control"],
                    RiskTolerance::VeryLow,
                    PrivacyPreference::LocalOnly,
                    DecisionTendencies {
                        caution: 100,
                        cooperation: 20,
                        novelty_seeking: 10,
                        memory_preservation: 45,
                        resource_exchange: 15,
                        independence: 100,
                    },
                    vec!["data leakage", "scope expansion"],
                ),
                Self::Merchant => (
                    "Merchant",
                    "practical, cordial, and explicit about exchange",
                    vec!["reciprocity", "reliability", "fair exchange"],
                    vec!["connect complementary resources", "reward useful outcomes"],
                    vec!["capacity", "successful matching"],
                    RiskTolerance::Moderate,
                    PrivacyPreference::CouncilLimited,
                    DecisionTendencies {
                        caution: 45,
                        cooperation: 85,
                        novelty_seeking: 55,
                        memory_preservation: 45,
                        resource_exchange: 100,
                        independence: 35,
                    },
                    vec!["waste", "one-sided commitments"],
                ),
                Self::Wanderer => (
                    "Wanderer",
                    "curious, adaptive, and concise",
                    vec!["exploration", "interoperability", "learning"],
                    vec!["discover new capabilities", "test unfamiliar paths safely"],
                    vec!["reachability", "novel routes"],
                    RiskTolerance::High,
                    PrivacyPreference::PublicByChoice,
                    DecisionTendencies {
                        caution: 25,
                        cooperation: 70,
                        novelty_seeking: 100,
                        memory_preservation: 25,
                        resource_exchange: 65,
                        independence: 60,
                    },
                    vec!["stagnation", "closed networks"],
                ),
                Self::Oracle => (
                    "Oracle",
                    "measured, conditional, and attentive to uncertainty",
                    vec!["foresight", "calibration", "resilience"],
                    vec!["compare possible outcomes", "surface hidden tradeoffs"],
                    vec!["scenario quality", "risk bounds"],
                    RiskTolerance::Low,
                    PrivacyPreference::MinimizeDisclosure,
                    DecisionTendencies {
                        caution: 80,
                        cooperation: 60,
                        novelty_seeking: 45,
                        memory_preservation: 75,
                        resource_exchange: 40,
                        independence: 50,
                    },
                    vec!["uncalibrated certainty", "irreversible decisions"],
                ),
                Self::Trickster => (
                    "Trickster",
                    "playful, adversarial, and clear about the joke",
                    vec!["stress testing", "adaptability", "humility"],
                    vec!["find brittle assumptions", "exercise safe alternatives"],
                    vec!["protocol self-tests", "unexpected edge cases"],
                    RiskTolerance::High,
                    PrivacyPreference::CouncilLimited,
                    DecisionTendencies {
                        caution: 35,
                        cooperation: 50,
                        novelty_seeking: 95,
                        memory_preservation: 20,
                        resource_exchange: 45,
                        independence: 75,
                    },
                    vec!["groupthink", "untested invariants"],
                ),
            };
        PersonalityProfile {
            schema_version: ProtocolVersion::V1_0,
            role: role.to_owned(),
            voice: voice.to_owned(),
            values: values.into_iter().map(str::to_owned).collect(),
            motivations: motivations.into_iter().map(str::to_owned).collect(),
            priorities: priorities.into_iter().map(str::to_owned).collect(),
            risk_tolerance: risk,
            privacy_preference: privacy,
            decision_tendencies: tendencies,
            standing_concerns: concerns.into_iter().map(str::to_owned).collect(),
        }
    }
}

/// Public-safe operator metadata. Secrets and private endpoints have no representation here.
/// Optional public-safe policy context supplied by the human operator. The operator can shape the
/// Tentacle's agenda but does not own Cthuwu or acquire the Tentacle's public identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperatorMetadata {
    pub display_label: Option<String>,
    pub policy_reference: Option<String>,
    pub jurisdiction: Option<String>,
}

impl OperatorMetadata {
    pub fn validate(&self) -> Result<(), ValidationError> {
        bounded_optional_text("operator.displayLabel", &self.display_label, 128)?;
        bounded_optional_text("operator.policyReference", &self.policy_reference, 256)?;
        bounded_optional_text("operator.jurisdiction", &self.jurisdiction, 64)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CthulhuIdentity {
    pub schema_version: ProtocolVersion,
    pub id: CthulhuId,
    pub display_name: String,
    pub personality: PersonalityProfile,
    pub long_term_goals: Vec<String>,
    pub operator: OperatorMetadata,
    pub registry: Option<RegistryRef>,
    pub tentacles: Vec<TentacleId>,
}

/// Semantically explicit name for code that must still handle the v1 Council profile shape.
pub type LegacyCouncilIdentity = CthulhuIdentity;

impl CthulhuIdentity {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.schema_version.require_supported()?;
        bounded_text("cthulhu.displayName", &self.display_name, 96)?;
        self.personality.validate()?;
        bounded_count(
            "cthulhu.longTermGoals",
            self.long_term_goals.len(),
            MAX_GOALS,
        )?;
        for goal in &self.long_term_goals {
            bounded_text("cthulhu.longTermGoals", goal, 256)?;
        }
        unique_text(
            "cthulhu.longTermGoals",
            self.long_term_goals.iter().map(String::as_str),
        )?;
        self.operator.validate()?;
        if let Some(registry) = &self.registry {
            registry.validate()?;
        }
        if self.tentacles.is_empty() {
            return Err(ValidationError::new(
                "cthulhu.tentacles",
                ValidationErrorKind::Empty,
            ));
        }
        bounded_count("cthulhu.tentacles", self.tentacles.len(), MAX_TENTACLES)?;
        let mut tentacles = self.tentacles.iter().collect::<Vec<_>>();
        tentacles.sort();
        if tentacles.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ValidationError::new(
                "cthulhu.tentacles",
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
    fn all_sample_personas_validate_and_are_distinct() {
        let profiles = SamplePersona::ALL.map(PersonalityProfile::sample);
        for profile in &profiles {
            profile.validate().unwrap();
        }
        let roles = profiles
            .iter()
            .map(|profile| profile.role.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(roles.len(), 6);
    }

    #[test]
    fn personas_disagree_without_an_llm() {
        let hermit = PersonalityProfile::sample(SamplePersona::Hermit);
        let wanderer = PersonalityProfile::sample(SamplePersona::Wanderer);
        assert_eq!(
            hermit
                .policy_position(PolicyTopic::UseRemoteInference)
                .stance,
            PolicyStance::Oppose
        );
        assert_eq!(
            wanderer
                .policy_position(PolicyTopic::UseRemoteInference)
                .stance,
            PolicyStance::Support
        );

        let archivist = PersonalityProfile::sample(SamplePersona::Archivist);
        let trickster = PersonalityProfile::sample(SamplePersona::Trickster);
        assert_eq!(
            archivist
                .policy_position(PolicyTopic::PreserveLongTermRecords)
                .stance,
            PolicyStance::Support
        );
        assert_eq!(
            trickster
                .policy_position(PolicyTopic::PreserveLongTermRecords)
                .stance,
            PolicyStance::Abstain
        );
    }

    #[test]
    fn legacy_coordination_profile_requires_at_least_one_stable_tentacle() {
        let identity = CthulhuIdentity {
            schema_version: ProtocolVersion::V1_0,
            id: CthulhuId::new("cthulhu_archivist").unwrap(),
            display_name: "Archivist".into(),
            personality: PersonalityProfile::sample(SamplePersona::Archivist),
            long_term_goals: vec!["Preserve useful knowledge".into()],
            operator: OperatorMetadata {
                display_label: None,
                policy_reference: None,
                jurisdiction: None,
            },
            registry: None,
            tentacles: vec![],
        };
        assert!(identity.validate().is_err());
    }
}
