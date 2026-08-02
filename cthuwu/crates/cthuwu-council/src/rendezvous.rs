use crate::lease::{Lease, LeaseError, LeaseManager, LeaseTiming, UserReference};
use crate::routing::{
    RouteCandidate, RoutingDecision, RoutingEngine, RoutingError, RoutingRequest,
};
use cthuwu_protocol::{CthulhuId, LeaseId, RequestId, SessionId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Control-plane-only request. There is deliberately no field for a user message or contact memory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RendezvousRequest {
    pub id: RequestId,
    pub session_id: SessionId,
    pub user: UserReference,
    pub routing: RoutingRequest,
    pub lease_id: LeaseId,
    pub lease_expires_at: u64,
    pub renewal_deadline: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RendezvousResponse {
    pub decision: RoutingDecision,
    pub lease: Lease,
    /// The selected Tentacle's direct XMTP endpoint. Normal conversation continues there.
    pub direct_xmtp_endpoint: String,
}

#[derive(Debug, Error)]
pub enum RendezvousError {
    #[error("invalid rendezvous request: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Routing(#[from] RoutingError),
    #[error(transparent)]
    Lease(#[from] LeaseError),
}

pub trait RendezvousService {
    fn request(
        &mut self,
        request: RendezvousRequest,
        offers: &[RouteCandidate],
        now: u64,
    ) -> Result<RendezvousResponse, RendezvousError>;
}

#[derive(Clone, Debug)]
pub struct LocalRendezvous {
    issuer: CthulhuId,
    routing: RoutingEngine,
    leases: LeaseManager,
}

impl LocalRendezvous {
    pub fn new(issuer: CthulhuId) -> Self {
        Self {
            issuer,
            routing: RoutingEngine,
            leases: LeaseManager::default(),
        }
    }

    pub fn leases(&self) -> &LeaseManager {
        &self.leases
    }

    pub fn leases_mut(&mut self) -> &mut LeaseManager {
        &mut self.leases
    }
}

impl RendezvousService for LocalRendezvous {
    fn request(
        &mut self,
        request: RendezvousRequest,
        offers: &[RouteCandidate],
        now: u64,
    ) -> Result<RendezvousResponse, RendezvousError> {
        if request.id != request.routing.request_id
            || request.session_id != request.routing.session_id
            || request.renewal_deadline <= now
            || request.lease_expires_at <= request.renewal_deadline
        {
            return Err(RendezvousError::Invalid(
                "identifiers or lease timing do not match",
            ));
        }
        let decision = self.routing.route(&request.routing, offers, now)?;
        let endpoint = decision.endpoint.clone();
        let lease = self.leases.grant(
            request.lease_id,
            request.routing,
            &decision,
            request.user,
            self.issuer.clone(),
            LeaseTiming {
                issued_at: now,
                expires_at: request.lease_expires_at,
                renewal_deadline: request.renewal_deadline,
            },
        )?;
        Ok(RendezvousResponse {
            decision,
            lease,
            direct_xmtp_endpoint: endpoint,
        })
    }
}
