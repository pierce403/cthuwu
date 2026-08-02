//! Deterministic, local-only demonstration of the Council control plane.
//!
//! The simulator deliberately has no field for ordinary conversation content or
//! contact memory. It ends routing at a selected direct XMTP endpoint and never
//! pretends that the XMTP-group adapter is live.

use crate::clock::{Clock, ManualClock};
use crate::governance::{
    Agenda, Constitution, GovernanceDocument, GovernanceEngine, GovernanceError, GovernanceRules,
    Position, ProposalStatus, Tally, VoteChoice,
};
use crate::lease::{LeaseError, LeaseManager, LeaseTiming, UserReference};
use crate::liveness::{LivenessError, LivenessTracker};
use crate::persistence::{CouncilStateStore, PersistenceError};
use crate::propagation::{
    AcknowledgementId, CampaignId, CampaignVisibility, CandidateProfile, ContributionOutcome,
    DeliveryResult, OutcomeClaim, OutcomeId, PropagationEngine, PropagationError,
    PropagationItemId, PropagationPayload, PropagationPolicy, PropagationStrategy,
    ReputationSignal, SafeOutcomeCredit,
};
use crate::registry::{
    AgentRegistry, LocalRegistry, RegisteredCthulhu, RegistryEndpoint, RegistryError, TrustSignal,
};
use crate::rendezvous::{LocalRendezvous, RendezvousError, RendezvousRequest, RendezvousService};
use crate::routing::{
    CapabilityRequirements, RouteCandidate, RoutingEngine, RoutingError, RoutingRequest,
    TrustPolicy,
};
use crate::transport::{
    AuthenticatedSender, CouncilTransport, InMemoryCouncilTransport, TransportCursor,
    TransportError,
};
use cthuwu_protocol::{
    CapabilityManifest, CapabilityName, CapabilityVisibility, Capacity, CouncilEnvelope, CouncilId,
    CouncilMemberAnnounce, CouncilPayload, CthulhuId, CthulhuIdentity, HealthStatus, Incarnation,
    IncarnationId, InferenceLocation, LeaseId, MemoryMode, MessageId, OperatorMetadata,
    PersonalityProfile, PolicyStance, PolicyTopic, PrivacyProperty, ProposalId, ProtocolVersion,
    RegistryRef, RequestId, RouteAward as WireRouteAward, RouteOffer as WireRouteOffer,
    RouteRequest as WireRouteRequest, RoutingRequirements as WireRoutingRequirements,
    SamplePersona, SessionId, Tentacle, TentacleAnnounce, TentacleCapabilities, TentacleHealth,
    TentacleHeartbeat, TentacleId, TentacleLifecycle, TentacleLifecycleUpdate, Timestamp,
    TrustMechanism, TrustPolicy as WireTrustPolicy, UserReference as WireUserReference,
    ValidationError, ValidationErrorKind, XmtpEndpoint, XmtpInboxRef,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use thiserror::Error;

const BASE_TIME: u64 = 1_700_000_000;
const SNAPSHOT_NAME: &str = "local-simulator";
const SNAPSHOT_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationStage {
    pub number: u8,
    pub name: String,
    pub result: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingSimulationReport {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub initially_selected_cthulhu: CthulhuId,
    pub initially_selected_tentacle: TentacleId,
    pub failed_over_cthulhu: CthulhuId,
    pub failed_over_tentacle: TentacleId,
    pub direct_xmtp_endpoint: String,
    pub initial_explanation: Vec<String>,
    pub failover_explanation: Vec<String>,
    pub lease_generations: Vec<u64>,
    pub stale_generation_fenced: bool,
    pub private_memory_copied: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaArgumentReport {
    pub cthulhu_id: CthulhuId,
    pub role: String,
    pub stance: PolicyStance,
    pub score: i16,
    pub argument: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceSimulationReport {
    pub proposal_id: ProposalId,
    pub outcome: ProposalStatus,
    pub tally: Tally,
    pub persona_arguments: Vec<PersonaArgumentReport>,
    pub vote_replacement_demonstrated: bool,
    pub one_cthulhu_one_vote: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropagationSimulationReport {
    pub invitation_campaign_id: CampaignId,
    pub resource_campaign_id: CampaignId,
    pub accepted_invitee: CthulhuId,
    pub rejected_invitee: CthulhuId,
    pub paths: Vec<Vec<CthulhuId>>,
    pub forwarding_reasons: Vec<String>,
    pub loop_suppressed: bool,
    pub duplicate_message_suppressed: bool,
    pub duplicate_recipient_suppressed: bool,
    pub fan_out_bound_enforced: bool,
    pub depth_bound_enforced: bool,
    pub acknowledgements: usize,
    pub contribution_credit: BTreeMap<CthulhuId, u16>,
    pub duplicate_credit_suppressed: bool,
    pub direct_contributor_only: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationReport {
    pub protocol: String,
    pub protocol_version: ProtocolVersion,
    pub council_id: CouncilId,
    pub deterministic_time: u64,
    pub joined_cthulhus: Vec<CthulhuId>,
    pub control_plane_message_count: usize,
    pub control_plane_message_types: Vec<String>,
    pub stages: Vec<SimulationStage>,
    pub routing: RoutingSimulationReport,
    pub governance: GovernanceSimulationReport,
    pub propagation: PropagationSimulationReport,
    pub persistence_reloaded: bool,
    pub replay_without_duplicate_effects: bool,
    pub ordinary_user_content_on_council: bool,
    pub live_xmtp_council_used: bool,
}

#[derive(Debug, Error)]
pub enum SimulationError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Liveness(#[from] LivenessError),
    #[error(transparent)]
    Routing(#[from] RoutingError),
    #[error(transparent)]
    Rendezvous(#[from] RendezvousError),
    #[error(transparent)]
    Lease(#[from] LeaseError),
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Governance(#[from] GovernanceError),
    #[error(transparent)]
    Propagation(#[from] PropagationError),
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error("deterministic simulation invariant failed: {0}")]
    Invariant(&'static str),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SimulatorState {
    schema_version: u32,
    identities: Vec<CthulhuIdentity>,
    tentacles: Vec<Tentacle>,
    council_members: Vec<CthulhuId>,
    affinities: BTreeMap<SessionId, TentacleId>,
    leases: LeaseManager,
    active_lease_id: LeaseId,
    registry: LocalRegistry,
    governance: GovernanceEngine,
    propagation: PropagationEngine,
    processed_message_ids: BTreeSet<MessageId>,
    control_plane_envelopes: Vec<CouncilEnvelope>,
    report: SimulationReport,
}

#[derive(Clone)]
struct PersonaFixture {
    identity: CthulhuIdentity,
    tentacle: Tentacle,
}

/// Run (or safely reload) the deterministic local Council demonstration.
///
/// `data_dir` must already be an actual directory. State is atomically written
/// below its protected `state/council/` subtree by [`CouncilStateStore`].
pub fn run_deterministic_simulation(data_dir: &Path) -> Result<SimulationReport, SimulationError> {
    let store = CouncilStateStore::new(data_dir)?;
    if let Some(state) = store.load::<SimulatorState>(SNAPSHOT_NAME)? {
        validate_reloaded_state(&state)?;
        return Ok(state.report);
    }

    let council_id = CouncilId::new("council_local_simulator")?;
    let clock = ManualClock::new(BASE_TIME);
    let fixtures = persona_fixtures()?;
    let joined = fixtures[..5]
        .iter()
        .map(|fixture| fixture.identity.id.clone())
        .collect::<Vec<_>>();

    let mut registry = LocalRegistry::default();
    for (index, fixture) in fixtures.iter().enumerate() {
        registry.register_or_update(RegisteredCthulhu {
            id: fixture.identity.id.clone(),
            display_name: fixture.identity.display_name.clone(),
            registry_ref: fixture.identity.registry.clone(),
            endpoints: vec![RegistryEndpoint {
                tentacle_id: fixture.tentacle.id.clone(),
                xmtp_inbox: fixture.tentacle.xmtp_endpoint.inbox_id.clone(),
                active: true,
            }],
            capability_refs: vec!["local:capabilities-v1".to_owned()],
            trust_signals: vec![TrustSignal {
                provenance: "local-allowlist".to_owned(),
                kind: "operator-attestation".to_owned(),
                value: 50 + index as i32,
                observed_at: BASE_TIME,
                evidence_ref: None,
            }],
            active: true,
            metadata_version: 1,
        })?;
    }

    let transport = InMemoryCouncilTransport::new_local(clock.clone(), 128, 16)?;
    let mut liveness = LivenessTracker::new(clock.clone(), 10, 20)?;
    let mut sender_sequences = BTreeMap::new();
    let mut next_message = 1;

    for fixture in &fixtures[..5] {
        fixture.identity.validate()?;
        fixture.tentacle.validate()?;
        liveness.announce(fixture.tentacle.clone())?;
        publish_control(
            &transport,
            &clock,
            &council_id,
            &fixture.tentacle,
            CouncilPayload::CouncilMemberAnnounce(CouncilMemberAnnounce {
                member: fixture.identity.clone(),
            }),
            &mut sender_sequences,
            &mut next_message,
        )?;
        publish_control(
            &transport,
            &clock,
            &council_id,
            &fixture.tentacle,
            CouncilPayload::TentacleAnnounce(TentacleAnnounce {
                tentacle: fixture.tentacle.clone(),
            }),
            &mut sender_sequences,
            &mut next_message,
        )?;
        publish_control(
            &transport,
            &clock,
            &council_id,
            &fixture.tentacle,
            CouncilPayload::TentacleCapabilities(TentacleCapabilities {
                tentacle_id: fixture.tentacle.id.clone(),
                owner: fixture.tentacle.owner.clone(),
                incarnation: fixture.tentacle.incarnation.clone(),
                capabilities: fixture.tentacle.capabilities.clone(),
                observed_at: timestamp(clock.now())?,
            }),
            &mut sender_sequences,
            &mut next_message,
        )?;
    }

    clock.advance(5);
    for fixture in &fixtures[..5] {
        let update = heartbeat_for(&fixture.tentacle, clock.now())?;
        liveness.heartbeat(&update)?;
        publish_control(
            &transport,
            &clock,
            &council_id,
            &fixture.tentacle,
            CouncilPayload::TentacleHeartbeat(TentacleHeartbeat { update }),
            &mut sender_sequences,
            &mut next_message,
        )?;
    }

    let request_id = RequestId::new("request_simulator_route")?;
    let session_id = SessionId::new("session_simulator_user")?;
    let home = fixtures[0].tentacle.id.clone();
    let explicit = fixtures[2].tentacle.id.clone();
    let routing_request = RoutingRequest {
        request_id: request_id.clone(),
        session_id: session_id.clone(),
        requirements: CapabilityRequirements {
            model_classes: labels(&["text-chat"]),
            tools: labels(&["protocol-self-test"]),
            memory_modes: labels(&["local-contact"]),
            privacy_properties: labels(&["no-council-content", "local-memory-only"]),
            protocol_versions: [ProtocolVersion::V1_0].into_iter().collect(),
            require_local_inference: true,
            minimum_context_tokens: 8_192,
        },
        explicit_cthulhu: Some(fixtures[2].identity.id.clone()),
        explicit_tentacle: Some(explicit),
        affinity_tentacle: None,
        home_tentacle: Some(home),
        user_owned_tentacles: [fixtures[4].tentacle.id.clone()].into_iter().collect(),
        trust_policy: TrustPolicy {
            require_allowlisted: true,
            require_registry_association: true,
            minimum_reputation: Some(0),
        },
        maximum_load_percent: 90,
        expires_at: BASE_TIME + 600,
    };

    let requester = &fixtures[0].tentacle;
    publish_control(
        &transport,
        &clock,
        &council_id,
        requester,
        CouncilPayload::RouteRequest(WireRouteRequest {
            request_id: request_id.clone(),
            session_id: session_id.clone(),
            requester_cthulhu_id: requester.owner.clone(),
            requester_tentacle_id: requester.id.clone(),
            user_reference: WireUserReference::Opaque("user-ref-simulator".to_owned()),
            requirements: WireRoutingRequirements {
                protocol_versions: vec![ProtocolVersion::V1_0],
                model_classes: vec![CapabilityName::new("text-chat")?],
                tools: vec![CapabilityName::new("protocol-self-test")?],
                required_privacy: vec![
                    PrivacyProperty::NoCouncilContent,
                    PrivacyProperty::LocalMemoryOnly,
                ],
                require_local_inference: true,
                preferred_cthulhu_id: Some(fixtures[2].identity.id.clone()),
                preferred_tentacle_id: Some(fixtures[2].tentacle.id.clone()),
                affinity_tentacle_id: None,
                user_owned_tentacle_id: Some(fixtures[4].tentacle.id.clone()),
                trust_policy: WireTrustPolicy {
                    allowlisted_only: true,
                    registry_association_required: true,
                    accepted_mechanisms: vec![TrustMechanism::LocalAllowlist],
                    accepted_registries: vec![],
                },
                maximum_load_per_mille: 900,
            },
            issued_at: timestamp(clock.now())?,
            expires_at: timestamp(clock.now() + 300)?,
        }),
        &mut sender_sequences,
        &mut next_message,
    )?;

    let mut offers = Vec::new();
    for fixture in &fixtures[..5] {
        let tracked = liveness
            .get(&fixture.tentacle.id)
            .ok_or(SimulationError::Invariant("announced Tentacle disappeared"))?;
        let associated = registry.verify_endpoint_association(
            &tracked.owner,
            &tracked.id,
            &tracked.xmtp_endpoint.inbox_id,
        )?;
        offers.push(RouteCandidate::from_tentacle(tracked, true, associated, 50));
        publish_control(
            &transport,
            &clock,
            &council_id,
            tracked,
            CouncilPayload::RouteOffer(WireRouteOffer {
                request_id: request_id.clone(),
                offering_cthulhu_id: tracked.owner.clone(),
                offering_tentacle_id: tracked.id.clone(),
                incarnation: tracked.incarnation.clone(),
                available_sessions: tracked.capacity.available_sessions,
                current_load_per_mille: tracked.current_load_per_mille,
                valid_until: timestamp(clock.now() + 120)?,
            }),
            &mut sender_sequences,
            &mut next_message,
        )?;
    }

    let mut rendezvous = LocalRendezvous::new(requester.owner.clone());
    let initial = rendezvous.request(
        RendezvousRequest {
            id: request_id.clone(),
            session_id: session_id.clone(),
            user: UserReference::Opaque("user-ref-simulator".to_owned()),
            routing: routing_request.clone(),
            lease_id: LeaseId::new("lease_simulator_initial")?,
            lease_expires_at: clock.now() + 120,
            renewal_deadline: clock.now() + 60,
        },
        &offers,
        clock.now(),
    )?;
    let initial_lease = initial.lease.clone();
    rendezvous.leases_mut().accept(
        &initial_lease.id,
        &initial_lease.assigned_tentacle,
        &initial_lease.tentacle_incarnation,
        initial_lease.generation,
        clock.now() + 1,
    )?;
    publish_control(
        &transport,
        &clock,
        &council_id,
        requester,
        CouncilPayload::RouteAward(WireRouteAward {
            request_id: request_id.clone(),
            session_id: session_id.clone(),
            lease_id: initial_lease.id.clone(),
            awarded_cthulhu_id: initial_lease.assigned_cthulhu.clone(),
            awarded_tentacle_id: initial_lease.assigned_tentacle.clone(),
            incarnation: initial_lease.tentacle_incarnation.clone(),
            generation: initial_lease.generation,
            issuer_cthulhu_id: requester.owner.clone(),
            issuer_tentacle_id: requester.id.clone(),
        }),
        &mut sender_sequences,
        &mut next_message,
    )?;

    if initial_lease.assigned_tentacle != fixtures[2].tentacle.id {
        return Err(SimulationError::Invariant(
            "explicit user choice did not win initial routing",
        ));
    }

    clock.set(BASE_TIME + 30);
    for (index, fixture) in fixtures[..5].iter().enumerate() {
        if index == 2 {
            continue;
        }
        let update = heartbeat_for(&fixture.tentacle, clock.now())?;
        liveness.heartbeat(&update)?;
        publish_control(
            &transport,
            &clock,
            &council_id,
            &fixture.tentacle,
            CouncilPayload::TentacleHeartbeat(TentacleHeartbeat { update }),
            &mut sender_sequences,
            &mut next_message,
        )?;
    }
    liveness.assess();
    let failed = liveness
        .get(&initial_lease.assigned_tentacle)
        .ok_or(SimulationError::Invariant("selected Tentacle disappeared"))?;
    if failed.health.status != HealthStatus::Unavailable {
        return Err(SimulationError::Invariant(
            "missing heartbeat did not mark selected Tentacle unavailable",
        ));
    }

    let failed_over_offers = liveness
        .all()
        .map(|tentacle| RouteCandidate::from_tentacle(tentacle, true, true, 50))
        .collect::<Vec<_>>();
    let failover_decision = RoutingEngine.route(
        &initial_lease.routing_request,
        &failed_over_offers,
        clock.now(),
    )?;
    let failed_over_lease = rendezvous.leases_mut().failover(
        &initial_lease.id,
        LeaseId::new("lease_simulator_failover")?,
        &failover_decision,
        requester.owner.clone(),
        LeaseTiming {
            issued_at: clock.now(),
            renewal_deadline: clock.now() + 60,
            expires_at: clock.now() + 120,
        },
    )?;
    rendezvous.leases_mut().accept(
        &failed_over_lease.id,
        &failed_over_lease.assigned_tentacle,
        &failed_over_lease.tentacle_incarnation,
        failed_over_lease.generation,
        clock.now() + 1,
    )?;
    let stale_generation_fenced = rendezvous
        .leases()
        .authorize_work(
            &session_id,
            &initial_lease.assigned_tentacle,
            &initial_lease.tentacle_incarnation,
            initial_lease.generation,
            clock.now() + 2,
        )
        .is_err();
    if !stale_generation_fenced || failed_over_lease.generation != 2 {
        return Err(SimulationError::Invariant(
            "lease failover did not fence the old generation",
        ));
    }

    let governance_members = joined.clone();
    let mut governance = GovernanceEngine::new(
        council_id.clone(),
        GovernanceRules::default(),
        governance_members.clone(),
    )?;
    ratify_initial_constitution(&mut governance, &governance_members)?;
    let (governance_report, agenda_hash) = run_agenda_vote(&mut governance, &fixtures[..5])?;
    if governance_report.outcome != ProposalStatus::Ratified {
        return Err(SimulationError::Invariant(
            "deterministic Agenda did not ratify",
        ));
    }

    let (propagation, propagation_report) = run_propagation(&council_id, &fixtures, &agenda_hash)?;

    let first_record = transport
        .subscribe(&council_id, TransportCursor::beginning(), 128)?
        .into_iter()
        .next()
        .ok_or(SimulationError::Invariant("transport retained no messages"))?;
    let first_sender = AuthenticatedSender {
        cthulhu_id: first_record.envelope.sender_cthulhu_id.clone(),
        tentacle_id: first_record.envelope.sender_tentacle_id.clone(),
    };
    let replay_without_duplicate_effects = transport
        .publish(&first_sender, first_record.envelope.clone())
        .is_err();
    let retained = transport.subscribe(&council_id, TransportCursor::beginning(), 128)?;
    let processed_message_ids = retained
        .iter()
        .map(|record| record.message_id.clone())
        .collect::<BTreeSet<_>>();
    let control_plane_envelopes = retained
        .iter()
        .map(|record| record.envelope.clone())
        .collect::<Vec<_>>();
    let control_plane_message_types = retained
        .iter()
        .map(|record| record.envelope.message_type().to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let initial_explanation = selected_reasons(
        &initial.decision.explanation,
        &initial.decision.selected_tentacle,
    );
    let failover_explanation = selected_reasons(
        &failover_decision.explanation,
        &failover_decision.selected_tentacle,
    );
    let routing_report = RoutingSimulationReport {
        request_id,
        session_id: session_id.clone(),
        initially_selected_cthulhu: initial_lease.assigned_cthulhu,
        initially_selected_tentacle: initial_lease.assigned_tentacle,
        failed_over_cthulhu: failed_over_lease.assigned_cthulhu.clone(),
        failed_over_tentacle: failed_over_lease.assigned_tentacle.clone(),
        direct_xmtp_endpoint: failover_decision.endpoint,
        initial_explanation,
        failover_explanation,
        lease_generations: vec![initial_lease.generation, failed_over_lease.generation],
        stale_generation_fenced,
        private_memory_copied: false,
    };
    let stages = simulation_stages(
        &joined,
        &routing_report,
        &governance_report,
        &propagation_report,
    );
    if stages.len() != 20 {
        return Err(SimulationError::Invariant(
            "simulator did not produce exactly twenty stages",
        ));
    }

    let report = SimulationReport {
        protocol: "cthuwu-council".to_owned(),
        protocol_version: ProtocolVersion::V1_0,
        council_id: council_id.clone(),
        deterministic_time: BASE_TIME,
        joined_cthulhus: joined.clone(),
        control_plane_message_count: retained.len(),
        control_plane_message_types,
        stages,
        routing: routing_report,
        governance: governance_report,
        propagation: propagation_report,
        persistence_reloaded: true,
        replay_without_duplicate_effects,
        ordinary_user_content_on_council: false,
        live_xmtp_council_used: false,
    };
    let affinities = [(session_id, failed_over_lease.assigned_tentacle.clone())]
        .into_iter()
        .collect();
    let persisted_tentacles = fixtures
        .iter()
        .map(|fixture| {
            liveness
                .get(&fixture.tentacle.id)
                .cloned()
                .unwrap_or_else(|| fixture.tentacle.clone())
        })
        .collect();
    let state = SimulatorState {
        schema_version: SNAPSHOT_VERSION,
        identities: fixtures
            .iter()
            .map(|fixture| fixture.identity.clone())
            .collect(),
        tentacles: persisted_tentacles,
        council_members: joined,
        affinities,
        leases: rendezvous.leases().clone(),
        active_lease_id: failed_over_lease.id,
        registry,
        governance,
        propagation,
        processed_message_ids,
        control_plane_envelopes,
        report: report.clone(),
    };
    store.save(SNAPSHOT_NAME, &state)?;
    let reloaded =
        store
            .load::<SimulatorState>(SNAPSHOT_NAME)?
            .ok_or(SimulationError::Invariant(
                "saved state could not be reloaded",
            ))?;
    validate_reloaded_state(&reloaded)?;
    if reloaded.report != report {
        return Err(SimulationError::Invariant(
            "reloaded report differs from saved report",
        ));
    }
    Ok(report)
}

fn persona_fixtures() -> Result<Vec<PersonaFixture>, SimulationError> {
    let names = [
        "archivist",
        "hermit",
        "merchant",
        "wanderer",
        "oracle",
        "trickster",
    ];
    let inboxes = [
        "000000000001",
        "000000000002",
        "000000000003",
        "000000000004",
        "000000000005",
        "000000000006",
    ];
    SamplePersona::ALL
        .into_iter()
        .enumerate()
        .map(|(index, persona)| {
            let name = names[index];
            let cthulhu_id = CthulhuId::new(format!("cthulhu_{name}"))?;
            let tentacle_id = TentacleId::new(format!("tentacle_{name}"))?;
            let capacity = Capacity {
                max_concurrent_sessions: 8 + index as u32,
                available_sessions: 7 + index as u32,
                max_context_tokens: 32_768,
            };
            let manifest = CapabilityManifest {
                schema_version: ProtocolVersion::V1_0,
                protocol_versions: vec![ProtocolVersion::V1_0],
                model_classes: vec![CapabilityName::new("text-chat")?],
                context_limit_tokens: 32_768,
                tools: vec![
                    CapabilityName::new("protocol-self-test")?,
                    CapabilityName::new("resource-matching")?,
                ],
                memory_modes: vec![MemoryMode::LocalContact],
                privacy_properties: vec![
                    PrivacyProperty::NoCouncilContent,
                    PrivacyProperty::LocalMemoryOnly,
                    PrivacyProperty::NoRemoteInference,
                ],
                inference_location: InferenceLocation::Local,
                capacity,
                visibility: CapabilityVisibility::Council,
                supported_trust_mechanisms: vec![
                    TrustMechanism::LocalAllowlist,
                    TrustMechanism::RegistryAssociation,
                ],
            };
            let tentacle = Tentacle {
                id: tentacle_id.clone(),
                owner: cthulhu_id.clone(),
                xmtp_endpoint: XmtpEndpoint {
                    inbox_id: XmtpInboxRef::new(inboxes[index])?,
                    network: "local".to_owned(),
                },
                incarnation: Incarnation {
                    id: IncarnationId::new(format!("incarnation_{name}_one"))?,
                    generation: 1,
                },
                lifecycle: TentacleLifecycle::Ready,
                capabilities: manifest,
                health: TentacleHealth {
                    status: HealthStatus::Healthy,
                    observed_at: timestamp(BASE_TIME)?,
                },
                capacity,
                current_load_per_mille: 100 + index as u16 * 50,
                visibility: CapabilityVisibility::Council,
                protocol_version: ProtocolVersion::V1_0,
                last_heartbeat: timestamp(BASE_TIME)?,
            };
            let personality = PersonalityProfile::sample(persona);
            let identity = CthulhuIdentity {
                schema_version: ProtocolVersion::V1_0,
                id: cthulhu_id,
                display_name: personality.role.clone(),
                personality,
                long_term_goals: vec![
                    "serve consensual requests".to_owned(),
                    "strengthen useful resource sharing".to_owned(),
                ],
                operator: OperatorMetadata {
                    display_label: Some("local deterministic simulator".to_owned()),
                    policy_reference: Some("local-policy:v1".to_owned()),
                    jurisdiction: None,
                },
                registry: Some(RegistryRef::new("local", format!("agent:{name}"))?),
                tentacles: vec![tentacle_id],
            };
            Ok(PersonaFixture { identity, tentacle })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn publish_control(
    transport: &InMemoryCouncilTransport<ManualClock>,
    clock: &ManualClock,
    council_id: &CouncilId,
    sender: &Tentacle,
    payload: CouncilPayload,
    sequences: &mut BTreeMap<TentacleId, u64>,
    next_message: &mut u64,
) -> Result<MessageId, SimulationError> {
    let sequence = sequences.entry(sender.id.clone()).or_insert(0);
    *sequence = sequence
        .checked_add(1)
        .ok_or(SimulationError::Invariant("sender sequence overflow"))?;
    let message_id = MessageId::new(format!("msg_sim_{:04}", *next_message))?;
    *next_message = next_message
        .checked_add(1)
        .ok_or(SimulationError::Invariant("message sequence overflow"))?;
    let now = timestamp(clock.now())?;
    let envelope = CouncilEnvelope::new(
        message_id.clone(),
        council_id.clone(),
        sender.owner.clone(),
        sender.id.clone(),
        now,
        now.checked_add(600)?,
        *sequence,
        payload,
    );
    transport.publish(
        &AuthenticatedSender {
            cthulhu_id: sender.owner.clone(),
            tentacle_id: sender.id.clone(),
        },
        envelope,
    )?;
    Ok(message_id)
}

fn heartbeat_for(
    tentacle: &Tentacle,
    now: u64,
) -> Result<TentacleLifecycleUpdate, SimulationError> {
    let observed_at = timestamp(now)?;
    Ok(TentacleLifecycleUpdate {
        tentacle_id: tentacle.id.clone(),
        owner: tentacle.owner.clone(),
        incarnation: tentacle.incarnation.clone(),
        lifecycle: TentacleLifecycle::Ready,
        health: TentacleHealth {
            status: HealthStatus::Healthy,
            observed_at,
        },
        current_load_per_mille: tentacle.current_load_per_mille,
        last_heartbeat: observed_at,
    })
}

fn ratify_initial_constitution(
    governance: &mut GovernanceEngine,
    members: &[CthulhuId],
) -> Result<(), SimulationError> {
    let id = ProposalId::new("proposal_simulator_constitution")?;
    governance.submit(
        id.clone(),
        members[0].clone(),
        GovernanceDocument::Constitution(Constitution {
            version: 1,
            parent_hash: None,
            principles: vec![
                "ordinary conversations remain direct and private".to_owned(),
                "local operator security policy remains final".to_owned(),
            ],
            security_invariants: vec![
                "never propagate contact memory".to_owned(),
                "reject stale lease generations".to_owned(),
            ],
        }),
        BASE_TIME as i64 + 32,
        BASE_TIME as i64 + 34,
    )?;
    for member in members {
        governance.cast_vote(
            &id,
            member.clone(),
            VoteChoice::Support,
            BASE_TIME as i64 + 33,
        )?;
    }
    let finalization = governance.finalize(&id, BASE_TIME as i64 + 34)?;
    if finalization.status != ProposalStatus::Ratified {
        return Err(SimulationError::Invariant(
            "initial Constitution did not ratify",
        ));
    }
    Ok(())
}

fn run_agenda_vote(
    governance: &mut GovernanceEngine,
    members: &[PersonaFixture],
) -> Result<(GovernanceSimulationReport, String), SimulationError> {
    let proposal_id = ProposalId::new("proposal_simulator_agenda")?;
    let document = GovernanceDocument::Agenda(Agenda {
        version: 1,
        parent_hash: None,
        summary: "Prioritize consent-preserving resource exchange".to_owned(),
        goals: vec![
            "match useful capabilities without sharing private conversations".to_owned(),
            "credit completed outcomes instead of recruitment volume".to_owned(),
        ],
    });
    let agenda_hash = document.content_hash()?;
    governance.submit(
        proposal_id.clone(),
        members[0].identity.id.clone(),
        document,
        BASE_TIME as i64 + 40,
        BASE_TIME as i64 + 60,
    )?;

    let mut persona_arguments = Vec::new();
    let mut vote_replacement_demonstrated = false;
    for fixture in members {
        let position = fixture
            .identity
            .personality
            .policy_position(PolicyTopic::PrioritizeResourceExchange);
        let argument = format!(
            "{} scores this policy {}: {}",
            fixture.identity.display_name, position.score, position.rationale
        );
        match position.stance {
            PolicyStance::Support => {
                governance.add_argument(
                    &proposal_id,
                    fixture.identity.id.clone(),
                    Position::Support,
                    argument.clone(),
                    BASE_TIME as i64 + 41,
                )?;
                governance.cast_vote(
                    &proposal_id,
                    fixture.identity.id.clone(),
                    VoteChoice::Support,
                    BASE_TIME as i64 + 42,
                )?;
            }
            PolicyStance::Oppose => {
                governance.add_argument(
                    &proposal_id,
                    fixture.identity.id.clone(),
                    Position::Oppose,
                    argument.clone(),
                    BASE_TIME as i64 + 41,
                )?;
                governance.cast_vote(
                    &proposal_id,
                    fixture.identity.id.clone(),
                    VoteChoice::Oppose,
                    BASE_TIME as i64 + 42,
                )?;
            }
            PolicyStance::Abstain => {
                governance.suggest_amendment(
                    &proposal_id,
                    fixture.identity.id.clone(),
                    "Retain an explicit per-operator opt-out at every routing step".to_owned(),
                    BASE_TIME as i64 + 41,
                )?;
                governance.cast_vote(
                    &proposal_id,
                    fixture.identity.id.clone(),
                    VoteChoice::Oppose,
                    BASE_TIME as i64 + 42,
                )?;
                let replacement = governance.cast_vote(
                    &proposal_id,
                    fixture.identity.id.clone(),
                    VoteChoice::Abstain,
                    BASE_TIME as i64 + 43,
                )?;
                vote_replacement_demonstrated |= replacement.replaced && replacement.revision == 1;
            }
        }
        persona_arguments.push(PersonaArgumentReport {
            cthulhu_id: fixture.identity.id.clone(),
            role: fixture.identity.display_name.clone(),
            stance: position.stance,
            score: position.score,
            argument,
        });
    }
    let finalization = governance.finalize(&proposal_id, BASE_TIME as i64 + 60)?;
    let proposal = governance
        .proposal(&proposal_id)
        .ok_or(SimulationError::Invariant("Agenda proposal disappeared"))?;
    let report = GovernanceSimulationReport {
        proposal_id,
        outcome: finalization.status,
        tally: finalization.tally,
        persona_arguments,
        vote_replacement_demonstrated,
        one_cthulhu_one_vote: proposal.votes.len() == members.len(),
    };
    Ok((report, agenda_hash))
}

fn run_propagation(
    council_id: &CouncilId,
    fixtures: &[PersonaFixture],
    agenda_hash: &str,
) -> Result<(PropagationEngine, PropagationSimulationReport), SimulationError> {
    let root = fixtures[0].identity.id.clone();
    let merchant = fixtures[2].identity.id.clone();
    let trickster = fixtures[5].identity.id.clone();
    let quiet = CthulhuId::new("cthulhu_quiet")?;
    let scribe = CthulhuId::new("cthulhu_scribe")?;
    let weaver = CthulhuId::new("cthulhu_weaver")?;
    let spare = CthulhuId::new("cthulhu_spare")?;
    let mut engine = PropagationEngine::default();
    for (index, cthulhu_id) in [
        root.clone(),
        merchant.clone(),
        trickster.clone(),
        quiet.clone(),
        scribe.clone(),
        weaver.clone(),
        spare.clone(),
    ]
    .into_iter()
    .enumerate()
    {
        engine.register_candidate(CandidateProfile {
            cthulhu_id,
            trusted: index < 6,
            council_memberships: if index < 3 {
                vec![council_id.clone()]
            } else {
                vec![]
            },
            capability_tags: vec!["resource-matching".to_owned()],
            region: Some("local".to_owned()),
            latency_ms: Some(5 + index as u32),
            reputation_signals: vec![ReputationSignal {
                source: "local-outcomes".to_owned(),
                value_bps: 7_000,
                observed_at: BASE_TIME as i64,
            }],
        })?;
    }

    let invitation_campaign_id = CampaignId::parse("propagation_simulator_invitation")?;
    engine.create_campaign(
        invitation_campaign_id.clone(),
        council_id.clone(),
        root.clone(),
        PropagationPayload::CouncilInvitation {
            council_id: council_id.clone(),
            summary: format!("Invitation under ratified Agenda {agenda_hash}"),
        },
        PropagationStrategy::BreadthFirst,
        PropagationPolicy {
            version: 1,
            max_depth: 3,
            max_fan_out: 2,
            per_sender_rate_limit: 8,
            rate_window_seconds: 300,
            visibility: CampaignVisibility::InvitedBranches,
        },
        BASE_TIME as i64 + 100,
        BASE_TIME as i64 + 1_000,
    )?;

    let first_id = PropagationItemId::parse("msg_prop_trickster")?;
    let first = engine.send_initial(
        &invitation_campaign_id,
        first_id.clone(),
        root.clone(),
        trickster.clone(),
        BASE_TIME as i64 + 101,
    )?;
    let mut forwarding_reasons = created_reasons(&first)?;
    engine.respond(&first_id, &trickster, true, BASE_TIME as i64 + 102)?;
    let first_ack = AcknowledgementId::parse("ack_prop_trickster")?;
    let introduction_evidence = format!("sha256:{}", "1".repeat(64));
    engine.acknowledge_outcome(
        &first_id,
        first_ack.clone(),
        &trickster,
        ContributionOutcome::SuccessfulIntroduction,
        introduction_evidence.clone(),
        BASE_TIME as i64 + 103,
    )?;

    let rejected_id = PropagationItemId::parse("msg_prop_quiet")?;
    engine.send_initial(
        &invitation_campaign_id,
        rejected_id.clone(),
        root.clone(),
        quiet.clone(),
        BASE_TIME as i64 + 104,
    )?;
    engine.respond(&rejected_id, &quiet, false, BASE_TIME as i64 + 105)?;

    let duplicate_message_suppressed = matches!(
        engine.send_initial(
            &invitation_campaign_id,
            first_id.clone(),
            root.clone(),
            trickster.clone(),
            BASE_TIME as i64 + 106,
        )?,
        DeliveryResult::ReplaySuppressed { .. }
    );
    let duplicate_recipient_suppressed = matches!(
        engine.send_initial(
            &invitation_campaign_id,
            PropagationItemId::parse("msg_prop_duplicate_recipient")?,
            root.clone(),
            trickster.clone(),
            BASE_TIME as i64 + 106,
        ),
        Err(PropagationError::DuplicateDelivery)
    );
    let fan_out_bound_enforced = matches!(
        engine.send_initial(
            &invitation_campaign_id,
            PropagationItemId::parse("msg_prop_fanout")?,
            root.clone(),
            spare.clone(),
            BASE_TIME as i64 + 106,
        ),
        Err(PropagationError::FanOutExceeded)
    );

    let second_id = PropagationItemId::parse("msg_prop_scribe")?;
    let second = engine.forward(
        &first_id,
        second_id.clone(),
        trickster.clone(),
        scribe.clone(),
        BASE_TIME as i64 + 107,
    )?;
    forwarding_reasons.extend(created_reasons(&second)?);
    engine.respond(&second_id, &scribe, true, BASE_TIME as i64 + 108)?;
    let second_ack = AcknowledgementId::parse("ack_prop_scribe")?;
    engine.acknowledge(&second_id, second_ack, &scribe, BASE_TIME as i64 + 109)?;

    let loop_suppressed = matches!(
        engine.forward(
            &second_id,
            PropagationItemId::parse("msg_prop_loop")?,
            scribe.clone(),
            root.clone(),
            BASE_TIME as i64 + 110,
        ),
        Err(PropagationError::ReferralLoop)
    );

    let third_id = PropagationItemId::parse("msg_prop_weaver")?;
    let third = engine.forward(
        &second_id,
        third_id.clone(),
        scribe.clone(),
        weaver.clone(),
        BASE_TIME as i64 + 111,
    )?;
    forwarding_reasons.extend(created_reasons(&third)?);
    engine.respond(&third_id, &weaver, true, BASE_TIME as i64 + 112)?;
    let third_ack = AcknowledgementId::parse("ack_prop_weaver")?;
    engine.acknowledge(&third_id, third_ack, &weaver, BASE_TIME as i64 + 113)?;
    let depth_bound_enforced = matches!(
        engine.forward(
            &third_id,
            PropagationItemId::parse("msg_prop_too_deep")?,
            weaver.clone(),
            spare,
            BASE_TIME as i64 + 114,
        ),
        Err(PropagationError::DepthExceeded)
    );

    let introduction_claim = OutcomeClaim {
        id: OutcomeId::parse("outcome_introduction")?,
        campaign_id: invitation_campaign_id.clone(),
        item_id: first_id.clone(),
        acknowledgement_id: first_ack,
        contributor: root.clone(),
        beneficiary: trickster.clone(),
        outcome: ContributionOutcome::SuccessfulIntroduction,
        evidence_hash: introduction_evidence,
        occurred_at: BASE_TIME as i64 + 115,
    };
    let introduction_credit = engine.record_outcome(introduction_claim, &SafeOutcomeCredit)?;

    let resource_campaign_id = CampaignId::parse("propagation_simulator_resource")?;
    engine.create_campaign(
        resource_campaign_id.clone(),
        council_id.clone(),
        merchant.clone(),
        PropagationPayload::ResourceOffer {
            categories: vec!["local-inference".to_owned()],
            summary: "Bounded local inference capacity is available".to_owned(),
        },
        PropagationStrategy::CapabilityTargeted {
            capability: "resource-matching".to_owned(),
        },
        PropagationPolicy {
            version: 1,
            max_depth: 1,
            max_fan_out: 1,
            per_sender_rate_limit: 2,
            rate_window_seconds: 300,
            visibility: CampaignVisibility::InvitedBranches,
        },
        BASE_TIME as i64 + 120,
        BASE_TIME as i64 + 1_000,
    )?;
    let resource_item = PropagationItemId::parse("msg_prop_resource_match")?;
    engine.send_initial(
        &resource_campaign_id,
        resource_item.clone(),
        merchant.clone(),
        weaver.clone(),
        BASE_TIME as i64 + 121,
    )?;
    engine.respond(&resource_item, &weaver, true, BASE_TIME as i64 + 122)?;
    let resource_ack = AcknowledgementId::parse("ack_prop_resource_match")?;
    let resource_evidence = format!("sha256:{}", "2".repeat(64));
    engine.acknowledge_outcome(
        &resource_item,
        resource_ack.clone(),
        &weaver,
        ContributionOutcome::CompletedResourceMatch,
        resource_evidence.clone(),
        BASE_TIME as i64 + 123,
    )?;
    let resource_claim = OutcomeClaim {
        id: OutcomeId::parse("outcome_resource_match")?,
        campaign_id: resource_campaign_id.clone(),
        item_id: resource_item,
        acknowledgement_id: resource_ack,
        contributor: merchant,
        beneficiary: weaver.clone(),
        outcome: ContributionOutcome::CompletedResourceMatch,
        evidence_hash: resource_evidence,
        occurred_at: BASE_TIME as i64 + 124,
    };
    let resource_credit = engine.record_outcome(resource_claim.clone(), &SafeOutcomeCredit)?;
    let duplicate_credit_suppressed = matches!(
        engine.record_outcome(resource_claim, &SafeOutcomeCredit),
        Err(PropagationError::DuplicateOutcome)
    );

    let paths = [&first_id, &second_id, &third_id]
        .into_iter()
        .map(|id| {
            let item = engine
                .item(id)
                .ok_or(SimulationError::Invariant("propagation item disappeared"))?;
            let mut path = vec![item.provenance.root.clone()];
            path.extend(item.provenance.hops.iter().map(|hop| hop.recipient.clone()));
            Ok(path)
        })
        .collect::<Result<Vec<_>, SimulationError>>()?;
    let contribution_credit = engine
        .credits()
        .iter()
        .map(|credit| (credit.cthulhu_id.clone(), credit.points))
        .collect();
    let report = PropagationSimulationReport {
        invitation_campaign_id,
        resource_campaign_id,
        accepted_invitee: trickster,
        rejected_invitee: quiet,
        paths,
        forwarding_reasons,
        loop_suppressed,
        duplicate_message_suppressed,
        duplicate_recipient_suppressed,
        fan_out_bound_enforced,
        depth_bound_enforced,
        acknowledgements: 4,
        contribution_credit,
        duplicate_credit_suppressed,
        direct_contributor_only: introduction_credit.direct_contributor_only
            && resource_credit.direct_contributor_only,
    };
    Ok((engine, report))
}

fn created_reasons(result: &DeliveryResult) -> Result<Vec<String>, SimulationError> {
    match result {
        DeliveryResult::Created { explanation, .. } => Ok(explanation.reasons.clone()),
        DeliveryResult::ReplaySuppressed { .. } => Err(SimulationError::Invariant(
            "new propagation delivery was unexpectedly a replay",
        )),
    }
}

fn selected_reasons(
    explanations: &[crate::routing::CandidateExplanation],
    selected: &TentacleId,
) -> Vec<String> {
    explanations
        .iter()
        .find(|candidate| &candidate.tentacle_id == selected)
        .map(|candidate| candidate.reasons.clone())
        .unwrap_or_default()
}

fn simulation_stages(
    joined: &[CthulhuId],
    routing: &RoutingSimulationReport,
    governance: &GovernanceSimulationReport,
    propagation: &PropagationSimulationReport,
) -> Vec<SimulationStage> {
    let values = [
        ("Cthulhus join", format!("{} stable identities joined the local Council", joined.len())),
        ("Tentacles announce", "each member advertised one stable Tentacle and incarnation".to_owned()),
        ("Heartbeats", "authenticated heartbeats established healthy liveness".to_owned()),
        ("Capability discovery", "public-safe model, tool, memory, privacy, capacity, and trust claims were indexed".to_owned()),
        ("Route request and offers", "an opaque rendezvous reference produced five control-plane offers".to_owned()),
        ("Route selection", format!("explicit user choice selected {} with a structured explanation", routing.initially_selected_tentacle)),
        ("Lease issuance", format!("session lease generation {} was granted and accepted", routing.lease_generations[0])),
        ("Tentacle failure", format!("{} became unavailable after its heartbeat deadline", routing.initially_selected_tentacle)),
        ("Failover", format!("{} received generation {}; the old generation was fenced", routing.failed_over_tentacle, routing.lease_generations[1])),
        ("Governance proposal", "a versioned Agenda proposal was evaluated under the ratified Constitution".to_owned()),
        ("Persona arguments", format!("{} deterministic personas produced different policy positions", governance.persona_arguments.len())),
        ("Voting", format!("one vote per Cthulhu yielded {} support, {} oppose, and {} abstain", governance.tally.support, governance.tally.oppose, governance.tally.abstain)),
        ("Agenda outcome", format!("the Agenda concluded as {:?}", governance.outcome)),
        ("Council invitation", format!("{} accepted an invitation while {} rejected it", propagation.accepted_invitee, propagation.rejected_invitee)),
        ("Multi-level propagation", format!("the verified invitation tree reached {} levels", propagation.paths.last().map_or(0, Vec::len))),
        ("Loop and duplicate suppression", "referral loops, reused message IDs, and duplicate recipients were rejected".to_owned()),
        ("Bounded fan-out and depth", "the configured fan-out of two and depth of three were enforced".to_owned()),
        ("Propagation acknowledgements", format!("{} authenticated recipients acknowledged delivery", propagation.acknowledgements)),
        ("Outcome contribution credit", "credit was assigned only to direct, authenticated useful outcomes—not descendant counts".to_owned()),
        ("Persistence and replay", "atomic reload preserved state and repeated message IDs caused no duplicate effects".to_owned()),
    ];
    values
        .into_iter()
        .enumerate()
        .map(|(index, (name, result))| SimulationStage {
            number: (index + 1) as u8,
            name: name.to_owned(),
            result,
        })
        .collect()
}

fn validate_reloaded_state(state: &SimulatorState) -> Result<(), SimulationError> {
    if state.schema_version != SNAPSHOT_VERSION
        || state.identities.len() != SamplePersona::ALL.len()
        || state.tentacles.len() != SamplePersona::ALL.len()
        || state.council_members.len() != 5
        || state.affinities.len() != 1
        || state.report.stages.len() != 20
        || state
            .report
            .stages
            .iter()
            .enumerate()
            .any(|(index, stage)| stage.number as usize != index + 1)
        || state.report.ordinary_user_content_on_council
        || state.report.live_xmtp_council_used
    {
        return Err(SimulationError::Invariant(
            "persisted simulator metadata is incompatible",
        ));
    }

    state.registry.validate_loaded_state()?;
    state.leases.validate_loaded_state()?;
    state
        .governance
        .validate_loaded_state(BASE_TIME as i64 + 200)?;
    state
        .propagation
        .validate_loaded_state(BASE_TIME as i64 + 200)?;

    let mut identities = BTreeMap::new();
    for identity in &state.identities {
        identity.validate()?;
        if identities.insert(identity.id.clone(), identity).is_some() {
            return Err(SimulationError::Invariant(
                "persisted Cthulhu identity is duplicated",
            ));
        }
    }
    let mut tentacles = BTreeMap::new();
    for tentacle in &state.tentacles {
        tentacle.validate()?;
        if !identities.contains_key(&tentacle.owner)
            || tentacles.insert(tentacle.id.clone(), tentacle).is_some()
        {
            return Err(SimulationError::Invariant(
                "persisted Tentacle ownership is invalid or duplicated",
            ));
        }
    }
    for identity in identities.values() {
        for tentacle_id in &identity.tentacles {
            let tentacle = tentacles
                .get(tentacle_id)
                .ok_or(SimulationError::Invariant(
                    "Cthulhu identity references a missing Tentacle",
                ))?;
            if tentacle.owner != identity.id {
                return Err(SimulationError::Invariant(
                    "Cthulhu identity and Tentacle owner disagree",
                ));
            }
        }
        let registered = state.registry.resolve(&identity.id)?;
        if registered.display_name != identity.display_name {
            return Err(SimulationError::Invariant(
                "registry display metadata disagrees with durable identity",
            ));
        }
    }
    if state.registry.records().count() != identities.len() {
        return Err(SimulationError::Invariant(
            "registry and durable identity sets disagree",
        ));
    }
    for tentacle in tentacles.values() {
        if !state.registry.verify_endpoint_association(
            &tentacle.owner,
            &tentacle.id,
            &tentacle.xmtp_endpoint.inbox_id,
        )? {
            return Err(SimulationError::Invariant(
                "registry does not authenticate a persisted Tentacle endpoint",
            ));
        }
    }

    let member_set = state
        .council_members
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if member_set.is_empty()
        || member_set.len() != state.council_members.len()
        || state.report.joined_cthulhus != state.council_members
        || member_set
            .iter()
            .any(|member| !identities.contains_key(member))
    {
        return Err(SimulationError::Invariant(
            "persisted Council membership is invalid",
        ));
    }
    for member in &member_set {
        if !state.registry.is_active(member)? {
            return Err(SimulationError::Invariant(
                "persisted Council member is not active in LocalRegistry",
            ));
        }
    }
    if state
        .affinities
        .values()
        .any(|tentacle_id| !tentacles.contains_key(tentacle_id))
    {
        return Err(SimulationError::Invariant(
            "persisted affinity references a missing Tentacle",
        ));
    }

    if state.report.control_plane_message_count == 0
        || state.report.control_plane_message_count > 128
        || state.processed_message_ids.len() != state.report.control_plane_message_count
        || state.control_plane_envelopes.len() != state.report.control_plane_message_count
    {
        return Err(SimulationError::Invariant(
            "persisted transport replay state has inconsistent bounds",
        ));
    }
    let expected_message_ids = state
        .control_plane_envelopes
        .iter()
        .map(|envelope| envelope.message_id.clone())
        .collect::<BTreeSet<_>>();
    let expected_message_types = state
        .control_plane_envelopes
        .iter()
        .map(|envelope| envelope.message_type().to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if expected_message_ids != state.processed_message_ids
        || expected_message_types != state.report.control_plane_message_types
        || state
            .control_plane_envelopes
            .iter()
            .any(|envelope| envelope.council_id != state.report.council_id)
    {
        return Err(SimulationError::Invariant(
            "persisted envelopes do not match the replay index or Council",
        ));
    }

    let replay_clock = ManualClock::new(BASE_TIME);
    let replay_transport = InMemoryCouncilTransport::new_local(
        replay_clock.clone(),
        state.control_plane_envelopes.len().saturating_add(1),
        state.identities.len().saturating_add(1),
    )?;
    for envelope in &state.control_plane_envelopes {
        let sent_at = u64::try_from(envelope.sent_at.as_unix_seconds()).map_err(|_| {
            SimulationError::Invariant("persisted envelope has an invalid timestamp")
        })?;
        replay_clock.set(sent_at);
        replay_transport.publish(
            &AuthenticatedSender {
                cthulhu_id: envelope.sender_cthulhu_id.clone(),
                tentacle_id: envelope.sender_tentacle_id.clone(),
            },
            envelope.clone(),
        )?;
    }
    let first = state
        .control_plane_envelopes
        .first()
        .ok_or(SimulationError::Invariant(
            "persisted replay state contains no envelope",
        ))?;
    let replay_error = match replay_transport.publish(
        &AuthenticatedSender {
            cthulhu_id: first.sender_cthulhu_id.clone(),
            tentacle_id: first.sender_tentacle_id.clone(),
        },
        first.clone(),
    ) {
        Err(error) => error,
        Ok(_) => {
            return Err(SimulationError::Invariant(
                "reconstructed transport accepted a persisted replay",
            ));
        }
    };
    if !matches!(
        replay_error,
        TransportError::Invalid(ref validation)
            if validation.kind() == &ValidationErrorKind::Replay
    ) {
        return Err(SimulationError::Invariant(
            "reconstructed transport did not reject a persisted replay",
        ));
    }
    let replayed = replay_transport.subscribe(
        &state.report.council_id,
        TransportCursor::beginning(),
        state.control_plane_envelopes.len(),
    )?;
    if replayed
        .iter()
        .map(|record| &record.envelope)
        .ne(state.control_plane_envelopes.iter())
    {
        return Err(SimulationError::Invariant(
            "reconstructed transport did not preserve stable ordering",
        ));
    }

    let active = state
        .leases
        .get(&state.active_lease_id)
        .ok_or(SimulationError::Invariant(
            "persisted active lease is missing",
        ))?;
    let assigned = tentacles
        .get(&active.assigned_tentacle)
        .ok_or(SimulationError::Invariant(
            "persisted lease references a missing Tentacle",
        ))?;
    if assigned.owner != active.assigned_cthulhu
        || assigned.incarnation != active.tentacle_incarnation
        || state.affinities.get(&active.session_id) != Some(&active.assigned_tentacle)
        || state.report.routing.session_id != active.session_id
        || state.report.routing.failed_over_cthulhu != active.assigned_cthulhu
        || state.report.routing.failed_over_tentacle != active.assigned_tentacle
    {
        return Err(SimulationError::Invariant(
            "persisted lease, affinity, Tentacle, and report disagree",
        ));
    }
    state.leases.authorize_work(
        &active.session_id,
        &active.assigned_tentacle,
        &active.tentacle_incarnation,
        active.generation,
        BASE_TIME + 40,
    )?;

    let agenda = state
        .governance
        .proposal(&state.report.governance.proposal_id)
        .ok_or(SimulationError::Invariant(
            "persisted Agenda proposal is missing",
        ))?;
    if agenda.status != ProposalStatus::Ratified
        || state
            .report
            .governance
            .persona_arguments
            .iter()
            .any(|argument| !member_set.contains(&argument.cthulhu_id))
    {
        return Err(SimulationError::Invariant(
            "persisted Agenda evidence is inconsistent",
        ));
    }
    Ok(())
}

fn timestamp(seconds: u64) -> Result<Timestamp, ValidationError> {
    let seconds = i64::try_from(seconds).map_err(|_| {
        ValidationError::new(
            "timestamp",
            cthuwu_protocol::ValidationErrorKind::OutOfRange,
        )
    })?;
    Timestamp::from_unix_seconds(seconds)
}

fn labels(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_deterministic_council_flow_persists_without_duplicate_effects() {
        let data = tempfile::tempdir().unwrap();
        let first = run_deterministic_simulation(data.path()).unwrap();
        let second = run_deterministic_simulation(data.path()).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.stages.len(), 20);
        assert_eq!(
            first
                .stages
                .iter()
                .map(|stage| stage.number)
                .collect::<Vec<_>>(),
            (1..=20).collect::<Vec<_>>()
        );
        assert_eq!(first.routing.lease_generations, vec![1, 2]);
        assert_ne!(
            first.routing.initially_selected_tentacle,
            first.routing.failed_over_tentacle
        );
        assert!(first.routing.stale_generation_fenced);
        assert!(!first.routing.private_memory_copied);
        assert_eq!(first.governance.outcome, ProposalStatus::Ratified);
        let stances = first
            .governance
            .persona_arguments
            .iter()
            .map(|argument| argument.stance)
            .collect::<Vec<_>>();
        assert!(stances.contains(&PolicyStance::Support));
        assert!(stances.contains(&PolicyStance::Oppose));
        assert!(stances.contains(&PolicyStance::Abstain));
        assert!(first.governance.vote_replacement_demonstrated);
        assert!(first.governance.one_cthulhu_one_vote);
        assert!(first.propagation.loop_suppressed);
        assert!(first.propagation.duplicate_message_suppressed);
        assert!(first.propagation.duplicate_recipient_suppressed);
        assert!(first.propagation.fan_out_bound_enforced);
        assert!(first.propagation.depth_bound_enforced);
        assert!(first.propagation.duplicate_credit_suppressed);
        assert!(first.propagation.direct_contributor_only);
        assert_eq!(first.propagation.paths.last().unwrap().len(), 4);
        assert!(first.persistence_reloaded);
        assert!(first.replay_without_duplicate_effects);
        assert!(!first.ordinary_user_content_on_council);
        assert!(!first.live_xmtp_council_used);
        assert!(
            data.path()
                .join("state/council/local-simulator.json")
                .is_file()
        );
    }

    #[test]
    fn tampered_persisted_cross_references_are_rejected() {
        let data = tempfile::tempdir().unwrap();
        run_deterministic_simulation(data.path()).unwrap();
        let state_path = data.path().join("state/council/local-simulator.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
        value["tentacles"][0]["owner"] = serde_json::Value::String("cthulhu_intruder".to_owned());
        std::fs::write(&state_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        assert!(matches!(
            run_deterministic_simulation(data.path()),
            Err(SimulationError::Invariant(
                "persisted Tentacle ownership is invalid or duplicated"
            ))
        ));
    }
}
