use crate::{
    COUNCIL_PROTOCOL_NAME, CouncilId, CouncilPayload, CouncilSigner, CouncilVerifier,
    EnvelopeDecodeError, MAX_ENVELOPE_BYTES, MessageId, MessageType, ProtocolVersion, Signature,
    TentacleId, Timestamp, ValidationError, ValidationErrorKind,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_ENVELOPE_LIFETIME_SECONDS: i64 = 3_600;
pub const MAX_FUTURE_CLOCK_SKEW_SECONDS: i64 = 300;

/// A validated Council wire message. The payload enum serializes as sibling `messageType` and
/// `payload` fields, preserving the documented JSON envelope shape.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CouncilEnvelope {
    pub protocol: String,
    pub version: ProtocolVersion,
    pub message_id: MessageId,
    pub council_id: CouncilId,
    pub sender_cthulhu_id: crate::CthulhuId,
    pub sender_tentacle_id: TentacleId,
    pub sent_at: Timestamp,
    pub expires_at: Timestamp,
    pub sequence: u64,
    #[serde(flatten)]
    pub payload: CouncilPayload,
    pub signature: Option<Signature>,
}

impl CouncilEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        message_id: MessageId,
        council_id: CouncilId,
        sender_cthulhu_id: crate::CthulhuId,
        sender_tentacle_id: TentacleId,
        sent_at: Timestamp,
        expires_at: Timestamp,
        sequence: u64,
        payload: CouncilPayload,
    ) -> Self {
        Self {
            protocol: COUNCIL_PROTOCOL_NAME.to_owned(),
            version: ProtocolVersion::V1_0,
            message_id,
            council_id,
            sender_cthulhu_id,
            sender_tentacle_id,
            sent_at,
            expires_at,
            sequence,
            payload,
            signature: None,
        }
    }

    pub const fn message_type(&self) -> MessageType {
        self.payload.message_type()
    }

    pub fn validate_at(&self, now: Timestamp) -> Result<(), ValidationError> {
        if self.protocol != COUNCIL_PROTOCOL_NAME {
            return Err(ValidationError::new(
                "protocol",
                ValidationErrorKind::Unsupported,
            ));
        }
        self.version.require_supported()?;
        if self.sequence == 0 {
            return Err(ValidationError::new(
                "sequence",
                ValidationErrorKind::OutOfRange,
            ));
        }
        let lifetime = self
            .expires_at
            .as_unix_seconds()
            .checked_sub(self.sent_at.as_unix_seconds())
            .ok_or_else(|| ValidationError::new("expiresAt", ValidationErrorKind::OutOfRange))?;
        if lifetime <= 0 || lifetime > MAX_ENVELOPE_LIFETIME_SECONDS {
            return Err(ValidationError::new(
                "expiresAt",
                ValidationErrorKind::OutOfRange,
            ));
        }
        if self.expires_at <= now {
            return Err(ValidationError::new(
                "expiresAt",
                ValidationErrorKind::Expired,
            ));
        }
        let latest_sent_at = now
            .as_unix_seconds()
            .checked_add(MAX_FUTURE_CLOCK_SKEW_SECONDS)
            .unwrap_or(i64::MAX);
        if self.sent_at.as_unix_seconds() > latest_sent_at {
            return Err(ValidationError::new(
                "sentAt",
                ValidationErrorKind::OutOfRange,
            ));
        }
        self.payload.validate_at(now)?;
        self.payload
            .validate_sender(&self.sender_cthulhu_id, &self.sender_tentacle_id)?;
        if let Some(signature) = &self.signature {
            signature.validate()?;
        }

        let actual = serde_json::to_vec(self)
            .map_err(|_| ValidationError::new("envelope", ValidationErrorKind::InvalidFormat))?
            .len();
        if actual > MAX_ENVELOPE_BYTES {
            return Err(ValidationError::new(
                "envelope",
                ValidationErrorKind::TooLong {
                    max: MAX_ENVELOPE_BYTES,
                    actual,
                },
            ));
        }
        Ok(())
    }

    pub fn from_json_at(bytes: &[u8], now: Timestamp) -> Result<Self, EnvelopeDecodeError> {
        if bytes.len() > MAX_ENVELOPE_BYTES {
            return Err(EnvelopeDecodeError::Oversized {
                max: MAX_ENVELOPE_BYTES,
                actual: bytes.len(),
            });
        }
        let value: Self = serde_json::from_slice(bytes)?;
        value.validate_at(now)?;
        Ok(value)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, EnvelopeDecodeError> {
        let encoded = serde_json::to_vec(self)?;
        if encoded.len() > MAX_ENVELOPE_BYTES {
            return Err(EnvelopeDecodeError::Oversized {
                max: MAX_ENVELOPE_BYTES,
                actual: encoded.len(),
            });
        }
        Ok(encoded)
    }

    pub fn canonical_signing_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut unsigned = self.clone();
        unsigned.signature = None;
        serde_json::to_vec(&unsigned)
    }

    pub fn attach_signature<S: CouncilSigner>(
        &mut self,
        signer: &S,
    ) -> Result<(), ValidationError> {
        if signer.signer_cthulhu_id() != &self.sender_cthulhu_id {
            return Err(ValidationError::new(
                "signature.sender",
                ValidationErrorKind::SenderMismatch,
            ));
        }
        let canonical = self
            .canonical_signing_bytes()
            .map_err(|_| ValidationError::new("signature", ValidationErrorKind::InvalidFormat))?;
        let signature = signer.sign(&canonical).map_err(|_| {
            ValidationError::new("signature", ValidationErrorKind::SignatureInvalid)
        })?;
        signature.validate()?;
        self.signature = Some(signature);
        Ok(())
    }

    pub fn verify_signature<V: CouncilVerifier + ?Sized>(
        &self,
        verifier: &V,
    ) -> Result<(), ValidationError> {
        let signature = self.signature.as_ref().ok_or_else(|| {
            ValidationError::new("signature", ValidationErrorKind::SignatureMissing)
        })?;
        signature.validate()?;
        let canonical = self
            .canonical_signing_bytes()
            .map_err(|_| ValidationError::new("signature", ValidationErrorKind::InvalidFormat))?;
        verifier
            .verify(&self.sender_cthulhu_id, &canonical, signature)
            .map_err(|_| ValidationError::new("signature", ValidationErrorKind::SignatureInvalid))
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct SenderSequenceKey {
    council_id: CouncilId,
    cthulhu_id: crate::CthulhuId,
    tentacle_id: TentacleId,
}

/// A bounded in-memory replay/ordering validator. Durable transports should persist equivalent
/// state before applying effects.
#[derive(Clone, Debug)]
pub struct ReplayGuard {
    max_messages: usize,
    max_senders: usize,
    seen_message_ids: BTreeSet<MessageId>,
    highest_sequence: BTreeMap<SenderSequenceKey, u64>,
}

impl ReplayGuard {
    pub fn new(max_messages: usize, max_senders: usize) -> Result<Self, ValidationError> {
        if max_messages == 0 || max_senders == 0 {
            return Err(ValidationError::new(
                "replayGuard",
                ValidationErrorKind::OutOfRange,
            ));
        }
        Ok(Self {
            max_messages,
            max_senders,
            seen_message_ids: BTreeSet::new(),
            highest_sequence: BTreeMap::new(),
        })
    }

    pub fn check_and_record(
        &mut self,
        envelope: &CouncilEnvelope,
        now: Timestamp,
    ) -> Result<(), ValidationError> {
        envelope.validate_at(now)?;
        if self.seen_message_ids.contains(&envelope.message_id) {
            return Err(ValidationError::new(
                "messageId",
                ValidationErrorKind::Replay,
            ));
        }
        let sender = SenderSequenceKey {
            council_id: envelope.council_id.clone(),
            cthulhu_id: envelope.sender_cthulhu_id.clone(),
            tentacle_id: envelope.sender_tentacle_id.clone(),
        };
        if self
            .highest_sequence
            .get(&sender)
            .is_some_and(|highest| envelope.sequence <= *highest)
        {
            return Err(ValidationError::new(
                "sequence",
                ValidationErrorKind::NonMonotonicSequence,
            ));
        }
        if self.seen_message_ids.len() == self.max_messages {
            return Err(ValidationError::new(
                "replayGuard.messages",
                ValidationErrorKind::TooMany {
                    max: self.max_messages,
                    actual: self.seen_message_ids.len() + 1,
                },
            ));
        }
        if !self.highest_sequence.contains_key(&sender)
            && self.highest_sequence.len() == self.max_senders
        {
            return Err(ValidationError::new(
                "replayGuard.senders",
                ValidationErrorKind::TooMany {
                    max: self.max_senders,
                    actual: self.highest_sequence.len() + 1,
                },
            ));
        }
        self.seen_message_ids.insert(envelope.message_id.clone());
        self.highest_sequence.insert(sender, envelope.sequence);
        Ok(())
    }

    pub fn contains(&self, message_id: &MessageId) -> bool {
        self.seen_message_ids.contains(message_id)
    }

    pub fn len(&self) -> usize {
        self.seen_message_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen_message_ids.is_empty()
    }
}

impl<'de> Deserialize<'de> for CouncilEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct WireEnvelope {
            protocol: String,
            version: ProtocolVersion,
            message_id: MessageId,
            council_id: CouncilId,
            sender_cthulhu_id: crate::CthulhuId,
            sender_tentacle_id: TentacleId,
            sent_at: Timestamp,
            expires_at: Timestamp,
            sequence: u64,
            #[serde(flatten)]
            payload: CouncilPayload,
            signature: Option<Signature>,
        }

        const KEYS: [&str; 12] = [
            "protocol",
            "version",
            "messageId",
            "messageType",
            "councilId",
            "senderCthulhuId",
            "senderTentacleId",
            "sentAt",
            "expiresAt",
            "sequence",
            "payload",
            "signature",
        ];

        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("Council envelope must be an object"))?;
        if object.len() != KEYS.len() || KEYS.iter().any(|key| !object.contains_key(*key)) {
            return Err(serde::de::Error::custom(
                "Council envelope contains missing or unsupported fields",
            ));
        }
        let wire: WireEnvelope = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            protocol: wire.protocol,
            version: wire.version,
            message_id: wire.message_id,
            council_id: wire.council_id,
            sender_cthulhu_id: wire.sender_cthulhu_id,
            sender_tentacle_id: wire.sender_tentacle_id,
            sent_at: wire.sent_at,
            expires_at: wire.expires_at,
            sequence: wire.sequence,
            payload: wire.payload,
            signature: wire.signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signing::test_signing::DeterministicTestSigner;
    use crate::{CouncilMemberWithdraw, CthulhuId};

    fn at(value: i64) -> Timestamp {
        Timestamp::from_unix_seconds(value).unwrap()
    }

    fn envelope(sequence: u64, message: &str) -> CouncilEnvelope {
        let cthulhu = CthulhuId::new("cthulhu_archivist").unwrap();
        CouncilEnvelope::new(
            MessageId::new(message).unwrap(),
            CouncilId::new("council_local").unwrap(),
            cthulhu.clone(),
            TentacleId::new("tentacle_home").unwrap(),
            at(100),
            at(200),
            sequence,
            CouncilPayload::CouncilMemberWithdraw(CouncilMemberWithdraw {
                cthulhu_id: cthulhu,
                reason: Some("test shutdown".into()),
            }),
        )
    }

    #[test]
    fn envelope_has_documented_flattened_shape_and_round_trips() {
        let envelope = envelope(42, "msg_one");
        envelope.validate_at(at(101)).unwrap();
        let json = envelope.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["protocol"], "cthuwu-council");
        assert_eq!(value["version"], "1.0");
        assert_eq!(value["messageType"], "council.member.withdraw");
        assert_eq!(value["payload"]["cthulhuId"], "cthulhu_archivist");
        assert!(value["signature"].is_null());
        assert_eq!(
            CouncilEnvelope::from_json_at(&json, at(101)).unwrap(),
            envelope
        );
    }

    #[test]
    fn envelope_rejects_unsupported_type_version_expiry_and_sender_mismatch() {
        let mut value = serde_json::to_value(envelope(1, "msg_one")).unwrap();
        value["messageType"] = serde_json::Value::String("user.private-message".into());
        assert!(
            CouncilEnvelope::from_json_at(&serde_json::to_vec(&value).unwrap(), at(101)).is_err()
        );

        let mut wrong = envelope(1, "msg_two");
        wrong.version = ProtocolVersion::new(2, 0);
        assert!(wrong.validate_at(at(101)).is_err());
        wrong.version = ProtocolVersion::V1_0;
        assert!(wrong.validate_at(at(200)).is_err());
        assert!(wrong.validate_at(at(201)).is_err());
        wrong.expires_at = at(200);
        wrong.sender_cthulhu_id = CthulhuId::new("cthulhu_intruder").unwrap();
        assert_eq!(
            wrong.validate_at(at(101)).unwrap_err().kind(),
            &ValidationErrorKind::SenderMismatch
        );
    }

    #[test]
    fn replay_guard_rejects_duplicate_ids_and_non_monotonic_sequences() {
        let mut guard = ReplayGuard::new(8, 2).unwrap();
        let first = envelope(10, "msg_first");
        guard.check_and_record(&first, at(101)).unwrap();
        assert_eq!(
            guard.check_and_record(&first, at(101)).unwrap_err().kind(),
            &ValidationErrorKind::Replay
        );
        let older = envelope(9, "msg_second");
        assert_eq!(
            guard.check_and_record(&older, at(101)).unwrap_err().kind(),
            &ValidationErrorKind::NonMonotonicSequence
        );
        assert_eq!(guard.len(), 1);
    }

    #[test]
    fn explicit_test_signer_round_trips_and_detects_tampering() {
        let mut envelope = envelope(1, "msg_signed");
        let signer = DeterministicTestSigner::new(
            envelope.sender_cthulhu_id.clone(),
            "fixture-key",
            b"not-secret-test-key",
        );
        envelope.attach_signature(&signer).unwrap();
        envelope.verify_signature(&signer).unwrap();
        envelope.sequence = 2;
        assert_eq!(
            envelope.verify_signature(&signer).unwrap_err().kind(),
            &ValidationErrorKind::SignatureInvalid
        );
    }

    #[test]
    fn oversized_json_is_rejected_before_deserialization() {
        let input = vec![b' '; MAX_ENVELOPE_BYTES + 1];
        assert!(matches!(
            CouncilEnvelope::from_json_at(&input, at(1)),
            Err(EnvelopeDecodeError::Oversized { .. })
        ));
    }

    #[test]
    fn unsupported_top_level_fields_are_rejected() {
        let mut value = serde_json::to_value(envelope(1, "msg_unknown_field")).unwrap();
        value["privateMessage"] = serde_json::Value::String("must not pass".into());
        assert!(
            CouncilEnvelope::from_json_at(&serde_json::to_vec(&value).unwrap(), at(101)).is_err()
        );
    }
}
