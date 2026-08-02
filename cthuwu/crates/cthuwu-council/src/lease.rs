use crate::routing::{RoutingDecision, RoutingRequest};
use cthuwu_protocol::{CthulhuId, Incarnation, LeaseId, SessionId, TentacleId, XmtpInboxRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

const MAX_LEASES: usize = 16_384;
const MAX_SESSIONS: usize = 16_384;
const MAX_LEASE_LIFETIME_SECONDS: u64 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum UserReference {
    XmtpInbox(XmtpInboxRef),
    /// Privacy-preserving rendezvous reference with no conversation content.
    Opaque(String),
}

impl UserReference {
    fn validate(&self) -> Result<(), LeaseError> {
        if let Self::Opaque(value) = self
            && (value.is_empty()
                || value.len() > 256
                || !value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
                }))
        {
            return Err(LeaseError::Invalid("invalid opaque user reference"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LeaseStatus {
    Granted,
    Active,
    Released,
    Revoked,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub id: LeaseId,
    pub session_id: SessionId,
    pub user: UserReference,
    pub assigned_cthulhu: CthulhuId,
    pub assigned_tentacle: TentacleId,
    pub tentacle_incarnation: Incarnation,
    pub generation: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub renewal_deadline: u64,
    pub routing_request: RoutingRequest,
    pub issuer: CthulhuId,
    pub status: LeaseStatus,
}

impl Lease {
    pub fn validate(&self) -> Result<(), LeaseError> {
        self.user.validate()?;
        self.tentacle_incarnation
            .validate()
            .map_err(|_| LeaseError::Invalid("Tentacle incarnation is invalid"))?;
        if self.generation == 0
            || self.issued_at >= self.expires_at
            || self.expires_at.saturating_sub(self.issued_at) > MAX_LEASE_LIFETIME_SECONDS
            || self.renewal_deadline <= self.issued_at
            || self.renewal_deadline > self.expires_at
            || self.routing_request.session_id != self.session_id
        {
            return Err(LeaseError::Invalid("lease bounds are inconsistent"));
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LeaseError {
    #[error("invalid lease: {0}")]
    Invalid(&'static str),
    #[error("lease is not active")]
    NotActive,
    #[error("lease operation came from the wrong Tentacle or incarnation")]
    WrongAssignee,
    #[error("lease generation is stale")]
    StaleGeneration,
    #[error("lease has expired or missed its renewal deadline")]
    Expired,
    #[error("lease operation predates the lease issue time")]
    NotYetActive,
    #[error("session already has an active lease")]
    ActiveLeaseExists,
    #[error("lease was not found")]
    NotFound,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LeaseManager {
    leases: BTreeMap<LeaseId, Lease>,
    active_by_session: BTreeMap<SessionId, LeaseId>,
    generations: BTreeMap<SessionId, u64>,
}

#[derive(Clone, Copy, Debug)]
pub struct LeaseTiming {
    pub issued_at: u64,
    pub expires_at: u64,
    pub renewal_deadline: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct LeaseRenewal {
    pub now: u64,
    pub renewal_deadline: u64,
    pub expires_at: u64,
}

impl LeaseManager {
    pub fn grant(
        &mut self,
        lease_id: LeaseId,
        request: RoutingRequest,
        decision: &RoutingDecision,
        user: UserReference,
        issuer: CthulhuId,
        timing: LeaseTiming,
    ) -> Result<Lease, LeaseError> {
        if self.leases.len() >= MAX_LEASES {
            return Err(LeaseError::Invalid("lease store is full"));
        }
        request
            .validate(timing.issued_at)
            .map_err(|_| LeaseError::Invalid("routing request is invalid or expired"))?;
        decision
            .selected_incarnation
            .validate()
            .map_err(|_| LeaseError::Invalid("routing decision incarnation is invalid"))?;
        if request.request_id != decision.request_id {
            return Err(LeaseError::Invalid("routing request and decision mismatch"));
        }
        if self
            .active_by_session
            .get(&request.session_id)
            .and_then(|id| self.leases.get(id))
            .is_some_and(|lease| matches!(lease.status, LeaseStatus::Granted | LeaseStatus::Active))
        {
            return Err(LeaseError::ActiveLeaseExists);
        }
        if self.leases.contains_key(&lease_id) {
            return Err(LeaseError::Invalid("duplicate lease ID"));
        }
        if self.generations.len() >= MAX_SESSIONS
            && !self.generations.contains_key(&request.session_id)
        {
            return Err(LeaseError::Invalid("session generation store is full"));
        }
        let generation = self
            .generations
            .get(&request.session_id)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(LeaseError::Invalid("lease generation overflow"))?;
        let lease = Lease {
            id: lease_id,
            session_id: request.session_id.clone(),
            user,
            assigned_cthulhu: decision.selected_cthulhu.clone(),
            assigned_tentacle: decision.selected_tentacle.clone(),
            tentacle_incarnation: decision.selected_incarnation.clone(),
            generation,
            issued_at: timing.issued_at,
            expires_at: timing.expires_at,
            renewal_deadline: timing.renewal_deadline,
            routing_request: request,
            issuer,
            status: LeaseStatus::Granted,
        };
        lease.validate()?;
        self.generations
            .insert(lease.session_id.clone(), generation);
        self.active_by_session
            .insert(lease.session_id.clone(), lease.id.clone());
        self.leases.insert(lease.id.clone(), lease.clone());
        Ok(lease)
    }

    pub fn accept(
        &mut self,
        id: &LeaseId,
        tentacle: &TentacleId,
        incarnation: &Incarnation,
        generation: u64,
        now: u64,
    ) -> Result<(), LeaseError> {
        let lease = self.active_mut(id, tentacle, incarnation, generation, now)?;
        if lease.status != LeaseStatus::Granted {
            return Err(LeaseError::NotActive);
        }
        lease.status = LeaseStatus::Active;
        Ok(())
    }

    pub fn renew(
        &mut self,
        id: &LeaseId,
        tentacle: &TentacleId,
        incarnation: &Incarnation,
        generation: u64,
        renewal: LeaseRenewal,
    ) -> Result<(), LeaseError> {
        let lease = self.active_mut(id, tentacle, incarnation, generation, renewal.now)?;
        if lease.status != LeaseStatus::Active
            || renewal.now > lease.renewal_deadline
            || renewal.renewal_deadline <= renewal.now
            || renewal.expires_at <= renewal.renewal_deadline
            || renewal.expires_at.saturating_sub(renewal.now) > MAX_LEASE_LIFETIME_SECONDS
        {
            return Err(LeaseError::Expired);
        }
        lease.renewal_deadline = renewal.renewal_deadline;
        lease.expires_at = renewal.expires_at;
        Ok(())
    }

    pub fn release(
        &mut self,
        id: &LeaseId,
        tentacle: &TentacleId,
        incarnation: &Incarnation,
        generation: u64,
        now: u64,
    ) -> Result<(), LeaseError> {
        if let Some(lease) = self.leases.get(id)
            && lease.status == LeaseStatus::Released
        {
            if generation != lease.generation {
                return Err(LeaseError::StaleGeneration);
            }
            if &lease.assigned_tentacle != tentacle || &lease.tentacle_incarnation != incarnation {
                return Err(LeaseError::WrongAssignee);
            }
            return Ok(());
        }
        let session = {
            let lease = self.active_mut(id, tentacle, incarnation, generation, now)?;
            lease.status = LeaseStatus::Released;
            lease.session_id.clone()
        };
        self.active_by_session.remove(&session);
        Ok(())
    }

    pub fn revoke(&mut self, id: &LeaseId) -> Result<(), LeaseError> {
        let lease = self.leases.get_mut(id).ok_or(LeaseError::NotFound)?;
        if !matches!(lease.status, LeaseStatus::Granted | LeaseStatus::Active) {
            return Err(LeaseError::NotActive);
        }
        lease.status = LeaseStatus::Revoked;
        self.active_by_session.remove(&lease.session_id);
        Ok(())
    }

    pub fn expire(&mut self, now: u64) -> Vec<LeaseId> {
        let mut expired = Vec::new();
        for lease in self.leases.values_mut() {
            if matches!(lease.status, LeaseStatus::Granted | LeaseStatus::Active)
                && (now >= lease.expires_at || now > lease.renewal_deadline)
            {
                lease.status = LeaseStatus::Expired;
                self.active_by_session.remove(&lease.session_id);
                expired.push(lease.id.clone());
            }
        }
        expired
    }

    /// Revoke the active generation and issue the next generation. Only opaque routing and user
    /// references carry over; private memory is deliberately absent from the lease model.
    pub fn failover(
        &mut self,
        old_lease_id: &LeaseId,
        new_lease_id: LeaseId,
        decision: &RoutingDecision,
        issuer: CthulhuId,
        timing: LeaseTiming,
    ) -> Result<Lease, LeaseError> {
        let old = self
            .leases
            .get(old_lease_id)
            .cloned()
            .ok_or(LeaseError::NotFound)?;
        let mut staged = self.clone();
        staged.revoke(old_lease_id)?;
        let replacement = staged.grant(
            new_lease_id,
            old.routing_request,
            decision,
            old.user,
            issuer,
            timing,
        )?;
        *self = staged;
        Ok(replacement)
    }

    pub fn authorize_work(
        &self,
        session: &SessionId,
        tentacle: &TentacleId,
        incarnation: &Incarnation,
        generation: u64,
        now: u64,
    ) -> Result<&Lease, LeaseError> {
        let id = self
            .active_by_session
            .get(session)
            .ok_or(LeaseError::NotActive)?;
        let lease = self.leases.get(id).ok_or(LeaseError::NotFound)?;
        verify_assignee(lease, tentacle, incarnation, generation, now)?;
        if lease.status != LeaseStatus::Active {
            return Err(LeaseError::NotActive);
        }
        Ok(lease)
    }

    pub fn get(&self, id: &LeaseId) -> Option<&Lease> {
        self.leases.get(id)
    }

    /// Revalidate all indexes and generation fences after loading durable state.
    pub fn validate_loaded_state(&self) -> Result<(), LeaseError> {
        if self.leases.len() > MAX_LEASES
            || self.active_by_session.len() > MAX_SESSIONS
            || self.generations.len() > MAX_SESSIONS
        {
            return Err(LeaseError::Invalid("persisted lease state is unbounded"));
        }

        let mut generations_by_session: BTreeMap<SessionId, BTreeSet<u64>> = BTreeMap::new();
        for (id, lease) in &self.leases {
            if id != &lease.id {
                return Err(LeaseError::Invalid("lease map key and lease ID differ"));
            }
            lease.validate()?;
            lease
                .routing_request
                .validate(lease.issued_at)
                .map_err(|_| LeaseError::Invalid("persisted routing request is invalid"))?;
            generations_by_session
                .entry(lease.session_id.clone())
                .or_default()
                .insert(lease.generation);

            let indexed = self.active_by_session.get(&lease.session_id) == Some(id);
            let active = matches!(lease.status, LeaseStatus::Granted | LeaseStatus::Active);
            if indexed != active {
                return Err(LeaseError::Invalid("active lease index is inconsistent"));
            }
        }

        for (session, id) in &self.active_by_session {
            let lease = self.leases.get(id).ok_or(LeaseError::Invalid(
                "active lease index references a missing lease",
            ))?;
            if &lease.session_id != session {
                return Err(LeaseError::Invalid("active lease session is inconsistent"));
            }
        }

        if self.generations.len() != generations_by_session.len() {
            return Err(LeaseError::Invalid(
                "session generation index is inconsistent",
            ));
        }
        for (session, generations) in generations_by_session {
            let maximum = generations.iter().next_back().copied().unwrap_or(0);
            if maximum > MAX_LEASES as u64
                || self.generations.get(&session) != Some(&maximum)
                || generations.len() != maximum as usize
                || generations.iter().copied().ne(1..=maximum)
            {
                return Err(LeaseError::Invalid(
                    "lease generation history is inconsistent",
                ));
            }
        }
        Ok(())
    }

    fn active_mut(
        &mut self,
        id: &LeaseId,
        tentacle: &TentacleId,
        incarnation: &Incarnation,
        generation: u64,
        now: u64,
    ) -> Result<&mut Lease, LeaseError> {
        let lease = self.leases.get_mut(id).ok_or(LeaseError::NotFound)?;
        verify_assignee(lease, tentacle, incarnation, generation, now)?;
        Ok(lease)
    }
}

fn verify_assignee(
    lease: &Lease,
    tentacle: &TentacleId,
    incarnation: &Incarnation,
    generation: u64,
    now: u64,
) -> Result<(), LeaseError> {
    if generation != lease.generation {
        return Err(LeaseError::StaleGeneration);
    }
    if &lease.assigned_tentacle != tentacle || &lease.tentacle_incarnation != incarnation {
        return Err(LeaseError::WrongAssignee);
    }
    if now < lease.issued_at {
        return Err(LeaseError::NotYetActive);
    }
    if now >= lease.expires_at || now > lease.renewal_deadline {
        return Err(LeaseError::Expired);
    }
    if !matches!(lease.status, LeaseStatus::Granted | LeaseStatus::Active) {
        return Err(LeaseError::NotActive);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{CapabilityRequirements, TrustPolicy};
    use cthuwu_protocol::{ProtocolVersion, RequestId};
    use std::collections::BTreeSet;

    fn request() -> RoutingRequest {
        RoutingRequest {
            request_id: RequestId::new("request_lease").unwrap(),
            session_id: SessionId::new("session_lease").unwrap(),
            requirements: CapabilityRequirements {
                protocol_versions: [ProtocolVersion::V1_0].into_iter().collect(),
                ..CapabilityRequirements::default()
            },
            explicit_cthulhu: None,
            explicit_tentacle: None,
            affinity_tentacle: None,
            home_tentacle: None,
            user_owned_tentacles: BTreeSet::new(),
            trust_policy: TrustPolicy::default(),
            maximum_load_percent: 100,
            expires_at: 500,
        }
    }

    fn decision(request: &RoutingRequest, name: &str) -> RoutingDecision {
        RoutingDecision {
            request_id: request.request_id.clone(),
            selected_cthulhu: CthulhuId::new(format!("cthulhu_{name}")).unwrap(),
            selected_tentacle: TentacleId::new(format!("tentacle_{name}")).unwrap(),
            selected_incarnation: Incarnation {
                id: cthuwu_protocol::IncarnationId::new(format!("incarnation_{name}")).unwrap(),
                generation: 1,
            },
            endpoint: format!("xmtp:{name}"),
            explanation: vec![],
        }
    }

    fn timing(now: u64) -> LeaseTiming {
        LeaseTiming {
            issued_at: now,
            renewal_deadline: now + 20,
            expires_at: now + 30,
        }
    }

    #[test]
    fn grant_accept_renew_and_release() {
        let mut manager = LeaseManager::default();
        let request = request();
        let decision = decision(&request, "home");
        let id = LeaseId::new("lease_first").unwrap();
        let lease = manager
            .grant(
                id.clone(),
                request,
                &decision,
                UserReference::Opaque("user-ref-1".to_owned()),
                CthulhuId::new("cthulhu_issuer").unwrap(),
                timing(100),
            )
            .unwrap();
        manager
            .accept(
                &id,
                &lease.assigned_tentacle,
                &lease.tentacle_incarnation,
                lease.generation,
                101,
            )
            .unwrap();
        manager
            .renew(
                &id,
                &lease.assigned_tentacle,
                &lease.tentacle_incarnation,
                lease.generation,
                LeaseRenewal {
                    now: 110,
                    renewal_deadline: 140,
                    expires_at: 160,
                },
            )
            .unwrap();
        assert!(
            manager
                .authorize_work(
                    &lease.session_id,
                    &lease.assigned_tentacle,
                    &lease.tentacle_incarnation,
                    lease.generation,
                    120,
                )
                .is_ok()
        );
        manager
            .release(
                &id,
                &lease.assigned_tentacle,
                &lease.tentacle_incarnation,
                lease.generation,
                121,
            )
            .unwrap();
        manager
            .release(
                &id,
                &lease.assigned_tentacle,
                &lease.tentacle_incarnation,
                lease.generation,
                122,
            )
            .unwrap();
        assert!(
            manager
                .authorize_work(
                    &lease.session_id,
                    &lease.assigned_tentacle,
                    &lease.tentacle_incarnation,
                    lease.generation,
                    123,
                )
                .is_err()
        );
    }

    #[test]
    fn failover_fences_old_generation_and_incarnation() {
        let mut manager = LeaseManager::default();
        let request = request();
        let first_decision = decision(&request, "first");
        let first_id = LeaseId::new("lease_first").unwrap();
        let first = manager
            .grant(
                first_id.clone(),
                request,
                &first_decision,
                UserReference::Opaque("user-ref-1".to_owned()),
                CthulhuId::new("cthulhu_issuer").unwrap(),
                timing(100),
            )
            .unwrap();
        manager
            .accept(
                &first_id,
                &first.assigned_tentacle,
                &first.tentacle_incarnation,
                1,
                101,
            )
            .unwrap();

        let second_decision = decision(&first.routing_request, "second");
        let second = manager
            .failover(
                &first_id,
                LeaseId::new("lease_second").unwrap(),
                &second_decision,
                CthulhuId::new("cthulhu_issuer").unwrap(),
                timing(110),
            )
            .unwrap();
        assert_eq!(second.generation, 2);
        assert_eq!(
            manager
                .authorize_work(
                    &first.session_id,
                    &first.assigned_tentacle,
                    &first.tentacle_incarnation,
                    1,
                    111,
                )
                .unwrap_err(),
            LeaseError::StaleGeneration
        );
        assert!(
            serde_json::to_string(&second)
                .unwrap()
                .find("message")
                .is_none()
        );
    }

    #[test]
    fn failed_failover_is_atomic_and_loaded_indexes_are_revalidated() {
        let mut manager = LeaseManager::default();
        let request = request();
        let first_decision = decision(&request, "first");
        let first_id = LeaseId::new("lease_atomic_first").unwrap();
        let first = manager
            .grant(
                first_id.clone(),
                request,
                &first_decision,
                UserReference::Opaque("user-ref-atomic".to_owned()),
                CthulhuId::new("cthulhu_issuer").unwrap(),
                timing(100),
            )
            .unwrap();
        manager
            .accept(
                &first_id,
                &first.assigned_tentacle,
                &first.tentacle_incarnation,
                first.generation,
                101,
            )
            .unwrap();

        let invalid_timing = LeaseTiming {
            issued_at: 110,
            renewal_deadline: 109,
            expires_at: 120,
        };
        assert!(
            manager
                .failover(
                    &first_id,
                    LeaseId::new("lease_atomic_second").unwrap(),
                    &decision(&first.routing_request, "second"),
                    CthulhuId::new("cthulhu_issuer").unwrap(),
                    invalid_timing,
                )
                .is_err()
        );
        assert!(
            manager
                .authorize_work(
                    &first.session_id,
                    &first.assigned_tentacle,
                    &first.tentacle_incarnation,
                    first.generation,
                    105,
                )
                .is_ok()
        );
        manager.validate_loaded_state().unwrap();

        let mut value = serde_json::to_value(&manager).unwrap();
        value["generations"]["session_lease"] = serde_json::json!(99);
        let corrupt: LeaseManager = serde_json::from_value(value).unwrap();
        assert!(corrupt.validate_loaded_state().is_err());
    }

    #[test]
    fn revoke_and_missed_renewal_deadline_are_terminal() {
        let mut manager = LeaseManager::default();
        let request = request();
        let selected = decision(&request, "terminal");
        let first_id = LeaseId::new("lease_revoked").unwrap();
        manager
            .grant(
                first_id.clone(),
                request.clone(),
                &selected,
                UserReference::Opaque("user-ref-terminal".to_owned()),
                CthulhuId::new("cthulhu_issuer").unwrap(),
                timing(100),
            )
            .unwrap();
        manager.revoke(&first_id).unwrap();
        assert_eq!(manager.get(&first_id).unwrap().status, LeaseStatus::Revoked);

        let second_id = LeaseId::new("lease_expired").unwrap();
        let second = manager
            .grant(
                second_id.clone(),
                request,
                &selected,
                UserReference::Opaque("user-ref-terminal".to_owned()),
                CthulhuId::new("cthulhu_issuer").unwrap(),
                timing(110),
            )
            .unwrap();
        manager
            .accept(
                &second_id,
                &second.assigned_tentacle,
                &second.tentacle_incarnation,
                second.generation,
                111,
            )
            .unwrap();
        assert_eq!(manager.expire(131), vec![second_id.clone()]);
        assert_eq!(
            manager.get(&second_id).unwrap().status,
            LeaseStatus::Expired
        );
        assert!(
            manager
                .authorize_work(
                    &second.session_id,
                    &second.assigned_tentacle,
                    &second.tentacle_incarnation,
                    second.generation,
                    131,
                )
                .is_err()
        );
    }
}
