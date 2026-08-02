use cthuwu_protocol::{CthulhuId, RegistryRef, TentacleId, XmtpInboxRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

const MAX_ENDPOINTS: usize = 32;
const MAX_AGENTS: usize = 4_096;
const MAX_CAPABILITY_REFS: usize = 64;
const MAX_SIGNALS: usize = 64;
const MAX_REFERENCE_LENGTH: usize = 512;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEndpoint {
    pub tentacle_id: TentacleId,
    pub xmtp_inbox: XmtpInboxRef,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustSignal {
    /// Names the source or method; it is never interpreted as universal truth.
    pub provenance: String,
    pub kind: String,
    pub value: i32,
    pub observed_at: u64,
    pub evidence_ref: Option<String>,
}

impl TrustSignal {
    pub fn validate(&self) -> Result<(), RegistryError> {
        validate_label(&self.provenance, "trust provenance", 128)?;
        validate_label(&self.kind, "trust signal kind", 64)?;
        if !(-1_000..=1_000).contains(&self.value) {
            return Err(RegistryError::Invalid("trust value is out of bounds"));
        }
        if self
            .evidence_ref
            .as_ref()
            .is_some_and(|value| value.len() > MAX_REFERENCE_LENGTH)
        {
            return Err(RegistryError::Invalid(
                "trust evidence reference is too long",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredCthulhu {
    pub id: CthulhuId,
    pub display_name: String,
    pub registry_ref: Option<RegistryRef>,
    pub endpoints: Vec<RegistryEndpoint>,
    pub capability_refs: Vec<String>,
    pub trust_signals: Vec<TrustSignal>,
    pub active: bool,
    pub metadata_version: u64,
}

impl RegisteredCthulhu {
    pub fn validate(&self) -> Result<(), RegistryError> {
        validate_label(&self.display_name, "display name", 128)?;
        if self.metadata_version == 0 {
            return Err(RegistryError::Invalid("metadata version must be positive"));
        }
        if let Some(reference) = &self.registry_ref {
            reference
                .validate()
                .map_err(|_| RegistryError::Invalid("registry reference is invalid"))?;
        }
        if self.endpoints.len() > MAX_ENDPOINTS {
            return Err(RegistryError::Invalid("too many registered endpoints"));
        }
        if self.capability_refs.len() > MAX_CAPABILITY_REFS {
            return Err(RegistryError::Invalid("too many capability references"));
        }
        if self.trust_signals.len() > MAX_SIGNALS {
            return Err(RegistryError::Invalid("too many trust signals"));
        }
        let mut tentacles = BTreeSet::new();
        let mut inboxes = BTreeSet::new();
        for endpoint in &self.endpoints {
            if !tentacles.insert(endpoint.tentacle_id.clone())
                || !inboxes.insert(endpoint.xmtp_inbox.clone())
            {
                return Err(RegistryError::Invalid("duplicate registry endpoint"));
            }
        }
        for reference in &self.capability_refs {
            validate_label(reference, "capability reference", MAX_REFERENCE_LENGTH)?;
        }
        for signal in &self.trust_signals {
            signal.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("invalid registry data: {0}")]
    Invalid(&'static str),
    #[error("Cthulhu is not registered")]
    NotFound,
    #[error("registry update is stale")]
    StaleUpdate,
    #[error("ERC-8004 adapter is not configured; LocalRegistry remains available")]
    AdapterUnavailable,
}

pub trait AgentRegistry {
    fn resolve(&self, id: &CthulhuId) -> Result<RegisteredCthulhu, RegistryError>;
    fn register_or_update(&mut self, agent: RegisteredCthulhu) -> Result<(), RegistryError>;
    fn endpoints(&self, id: &CthulhuId) -> Result<Vec<RegistryEndpoint>, RegistryError>;
    fn capability_references(&self, id: &CthulhuId) -> Result<Vec<String>, RegistryError>;
    fn trust_signals(&self, id: &CthulhuId) -> Result<Vec<TrustSignal>, RegistryError>;
    fn verify_endpoint_association(
        &self,
        id: &CthulhuId,
        tentacle_id: &TentacleId,
        inbox: &XmtpInboxRef,
    ) -> Result<bool, RegistryError>;
    fn is_active(&self, id: &CthulhuId) -> Result<bool, RegistryError>;
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LocalRegistry {
    agents: BTreeMap<CthulhuId, RegisteredCthulhu>,
}

impl LocalRegistry {
    pub fn records(&self) -> impl Iterator<Item = &RegisteredCthulhu> {
        self.agents.values()
    }

    /// Revalidates all invariants after loading untrusted durable state.
    pub fn validate_loaded_state(&self) -> Result<(), RegistryError> {
        if self.agents.len() > MAX_AGENTS {
            return Err(RegistryError::Invalid("too many registered Cthulhus"));
        }
        let mut tentacle_owners = BTreeMap::new();
        let mut inbox_owners = BTreeMap::new();
        for (key, agent) in &self.agents {
            if key != &agent.id {
                return Err(RegistryError::Invalid("registry key and Cthulhu ID differ"));
            }
            agent.validate()?;
            for endpoint in &agent.endpoints {
                if tentacle_owners
                    .insert(endpoint.tentacle_id.clone(), agent.id.clone())
                    .is_some()
                    || inbox_owners
                        .insert(endpoint.xmtp_inbox.clone(), agent.id.clone())
                        .is_some()
                {
                    return Err(RegistryError::Invalid(
                        "Tentacle and XMTP endpoint ownership must be unique",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl AgentRegistry for LocalRegistry {
    fn resolve(&self, id: &CthulhuId) -> Result<RegisteredCthulhu, RegistryError> {
        self.agents.get(id).cloned().ok_or(RegistryError::NotFound)
    }

    fn register_or_update(&mut self, agent: RegisteredCthulhu) -> Result<(), RegistryError> {
        agent.validate()?;
        if self.agents.len() >= MAX_AGENTS && !self.agents.contains_key(&agent.id) {
            return Err(RegistryError::Invalid("too many registered Cthulhus"));
        }
        if self
            .agents
            .get(&agent.id)
            .is_some_and(|current| current.metadata_version >= agent.metadata_version)
        {
            return Err(RegistryError::StaleUpdate);
        }
        if self.agents.values().any(|other| {
            other.id != agent.id
                && other.endpoints.iter().any(|existing| {
                    agent.endpoints.iter().any(|incoming| {
                        existing.tentacle_id == incoming.tentacle_id
                            || existing.xmtp_inbox == incoming.xmtp_inbox
                    })
                })
        }) {
            return Err(RegistryError::Invalid(
                "Tentacle or XMTP inbox is already associated with another Cthulhu",
            ));
        }
        self.agents.insert(agent.id.clone(), agent);
        Ok(())
    }

    fn endpoints(&self, id: &CthulhuId) -> Result<Vec<RegistryEndpoint>, RegistryError> {
        Ok(self.resolve(id)?.endpoints)
    }

    fn capability_references(&self, id: &CthulhuId) -> Result<Vec<String>, RegistryError> {
        Ok(self.resolve(id)?.capability_refs)
    }

    fn trust_signals(&self, id: &CthulhuId) -> Result<Vec<TrustSignal>, RegistryError> {
        Ok(self.resolve(id)?.trust_signals)
    }

    fn verify_endpoint_association(
        &self,
        id: &CthulhuId,
        tentacle_id: &TentacleId,
        inbox: &XmtpInboxRef,
    ) -> Result<bool, RegistryError> {
        Ok(self.resolve(id)?.endpoints.iter().any(|endpoint| {
            endpoint.active && &endpoint.tentacle_id == tentacle_id && &endpoint.xmtp_inbox == inbox
        }))
    }

    fn is_active(&self, id: &CthulhuId) -> Result<bool, RegistryError> {
        Ok(self.resolve(id)?.active)
    }
}

/// Adapter boundary only. Domain types deliberately contain no chain, deployment, ABI, or draft
/// revision. A future implementation must be configured explicitly and tested against a selected
/// ERC-8004 deployment before this type performs network I/O.
#[derive(Clone, Debug, Default)]
pub struct Erc8004Registry;

impl AgentRegistry for Erc8004Registry {
    fn resolve(&self, _id: &CthulhuId) -> Result<RegisteredCthulhu, RegistryError> {
        Err(RegistryError::AdapterUnavailable)
    }
    fn register_or_update(&mut self, _agent: RegisteredCthulhu) -> Result<(), RegistryError> {
        Err(RegistryError::AdapterUnavailable)
    }
    fn endpoints(&self, _id: &CthulhuId) -> Result<Vec<RegistryEndpoint>, RegistryError> {
        Err(RegistryError::AdapterUnavailable)
    }
    fn capability_references(&self, _id: &CthulhuId) -> Result<Vec<String>, RegistryError> {
        Err(RegistryError::AdapterUnavailable)
    }
    fn trust_signals(&self, _id: &CthulhuId) -> Result<Vec<TrustSignal>, RegistryError> {
        Err(RegistryError::AdapterUnavailable)
    }
    fn verify_endpoint_association(
        &self,
        _id: &CthulhuId,
        _tentacle_id: &TentacleId,
        _inbox: &XmtpInboxRef,
    ) -> Result<bool, RegistryError> {
        Err(RegistryError::AdapterUnavailable)
    }
    fn is_active(&self, _id: &CthulhuId) -> Result<bool, RegistryError> {
        Err(RegistryError::AdapterUnavailable)
    }
}

fn validate_label(value: &str, name: &'static str, maximum: usize) -> Result<(), RegistryError> {
    if value.trim().is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(RegistryError::Invalid(name));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(version: u64) -> RegisteredCthulhu {
        RegisteredCthulhu {
            id: CthulhuId::new("cthulhu_archivist").unwrap(),
            display_name: "Archivist".to_owned(),
            registry_ref: Some(RegistryRef::new("erc-8004", "agent:42").unwrap()),
            endpoints: vec![RegistryEndpoint {
                tentacle_id: TentacleId::new("tentacle_archive").unwrap(),
                xmtp_inbox: XmtpInboxRef::new("012345abcdef").unwrap(),
                active: true,
            }],
            capability_refs: vec!["ipfs:capabilities-v1".to_owned()],
            trust_signals: vec![TrustSignal {
                provenance: "local-allowlist".to_owned(),
                kind: "operator-attestation".to_owned(),
                value: 50,
                observed_at: 100,
                evidence_ref: None,
            }],
            active: true,
            metadata_version: version,
        }
    }

    #[test]
    fn local_registry_resolves_and_verifies_endpoints() {
        let mut registry = LocalRegistry::default();
        let agent = record(1);
        registry.register_or_update(agent.clone()).unwrap();
        assert_eq!(registry.resolve(&agent.id).unwrap(), agent);
        assert!(
            registry
                .verify_endpoint_association(
                    &agent.id,
                    &agent.endpoints[0].tentacle_id,
                    &agent.endpoints[0].xmtp_inbox,
                )
                .unwrap()
        );
        assert!(registry.is_active(&agent.id).unwrap());
    }

    #[test]
    fn local_registry_rejects_stale_metadata() {
        let mut registry = LocalRegistry::default();
        registry.register_or_update(record(2)).unwrap();
        assert_eq!(
            registry.register_or_update(record(1)).unwrap_err(),
            RegistryError::StaleUpdate
        );
    }

    #[test]
    fn erc8004_boundary_does_not_fake_network_support() {
        let registry = Erc8004Registry;
        assert_eq!(
            registry
                .resolve(&CthulhuId::new("cthulhu_unknown").unwrap())
                .unwrap_err(),
            RegistryError::AdapterUnavailable
        );
    }

    #[test]
    fn local_registry_revalidates_loaded_key_identity_and_bounds() {
        let mut registry = LocalRegistry::default();
        registry.register_or_update(record(1)).unwrap();
        let encoded = serde_json::to_vec(&registry).unwrap();
        let restored: LocalRegistry = serde_json::from_slice(&encoded).unwrap();
        restored.validate_loaded_state().unwrap();

        let mut value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        let records = value["agents"].as_object_mut().unwrap();
        let agent = records.remove("cthulhu_archivist").unwrap();
        records.insert("cthulhu_intruder".to_owned(), agent);
        let corrupt: LocalRegistry = serde_json::from_value(value).unwrap();
        assert!(corrupt.validate_loaded_state().is_err());
    }

    #[test]
    fn endpoint_associations_are_unique_across_cthulhus() {
        let mut registry = LocalRegistry::default();
        registry.register_or_update(record(1)).unwrap();
        let mut conflict = record(1);
        conflict.id = CthulhuId::new("cthulhu_intruder").unwrap();
        assert!(registry.register_or_update(conflict).is_err());
    }
}
