use crate::clock::Clock;
use cthuwu_protocol::{
    CouncilEnvelope, CouncilId, CouncilVerifier, CthulhuId, MessageId, ReplayGuard, TentacleId,
    Timestamp, ValidationError,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use thiserror::Error;

const MAX_SUBSCRIPTION_BATCH: usize = 256;
const SENDER_RATE_WINDOW_SECONDS: i64 = 60;
const MAX_MESSAGES_PER_SENDER_PER_WINDOW: usize = 128;

/// Identity established by the transport before a Council envelope reaches the protocol engine.
///
/// The in-memory implementation accepts this value from its trusted caller. A real transport
/// adapter must construct it from the authenticated XMTP sender, never from envelope fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedSender {
    pub cthulhu_id: CthulhuId,
    pub tentacle_id: TentacleId,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TransportCursor(u64);

impl TransportCursor {
    pub const fn beginning() -> Self {
        Self(0)
    }

    pub const fn order(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedCouncilMessage {
    pub order: u64,
    pub message_id: MessageId,
    pub envelope: CouncilEnvelope,
}

impl PublishedCouncilMessage {
    pub const fn cursor(&self) -> TransportCursor {
        TransportCursor(self.order)
    }
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("invalid Council envelope: {0}")]
    Invalid(#[from] ValidationError),
    #[error("the Council envelope sender does not match the authenticated transport sender")]
    SenderMismatch,
    #[error("the configured Council signature verifier rejected the message")]
    SignatureRejected,
    #[error("the in-memory Council transport reached its configured retention limit")]
    RetentionLimit,
    #[error("subscription batch size must be between 1 and {MAX_SUBSCRIPTION_BATCH}")]
    InvalidBatchSize,
    #[error("Council transport state is unavailable")]
    StateUnavailable,
    #[error("the Council sender exceeded the bounded publish rate")]
    RateLimited,
    #[error("the injected Council clock moved backwards")]
    ClockMovedBackwards,
    #[error("live XMTP Council groups are an adapter boundary and are not implemented")]
    XmtpAdapterUnavailable,
}

/// Council control-plane transport. Implementations must authenticate senders independently of
/// message payloads, preserve stable message IDs, and expose replayable ordering metadata.
pub trait CouncilTransport: Send + Sync {
    fn publish(
        &self,
        authenticated_sender: &AuthenticatedSender,
        envelope: CouncilEnvelope,
    ) -> Result<PublishedCouncilMessage, TransportError>;

    fn subscribe(
        &self,
        council_id: &CouncilId,
        after: TransportCursor,
        limit: usize,
    ) -> Result<Vec<PublishedCouncilMessage>, TransportError>;
}

struct TransportState {
    next_order: u64,
    replay_guard: ReplayGuard,
    retained: Vec<PublishedCouncilMessage>,
    sender_rates: BTreeMap<(CthulhuId, TentacleId), SenderRateWindow>,
}

struct SenderRateWindow {
    started_at: i64,
    messages: usize,
}

/// Deterministic local transport with bounded retention and replay suppression.
///
/// `new_local` relies on the caller-provided authenticated sender, which is suitable for tests and
/// the local simulator. `with_required_signatures` additionally verifies every envelope through a
/// caller-supplied verifier. No production signer or trust mechanism is invented here.
pub struct InMemoryCouncilTransport<C: Clock> {
    clock: C,
    max_retained: usize,
    required_verifier: Option<Arc<dyn CouncilVerifier + Send + Sync>>,
    state: Mutex<TransportState>,
}

impl<C: Clock> InMemoryCouncilTransport<C> {
    pub fn new_local(
        clock: C,
        max_retained: usize,
        max_senders: usize,
    ) -> Result<Self, TransportError> {
        if max_retained == 0 {
            return Err(TransportError::RetentionLimit);
        }
        Ok(Self {
            clock,
            max_retained,
            required_verifier: None,
            state: Mutex::new(TransportState {
                next_order: 1,
                replay_guard: ReplayGuard::new(max_retained, max_senders)?,
                retained: Vec::new(),
                sender_rates: BTreeMap::new(),
            }),
        })
    }

    pub fn with_required_signatures(
        mut self,
        verifier: Arc<dyn CouncilVerifier + Send + Sync>,
    ) -> Self {
        self.required_verifier = Some(verifier);
        self
    }

    fn now(&self) -> Timestamp {
        let seconds = self.clock.now().min(i64::MAX as u64) as i64;
        Timestamp::from_unix_seconds_unchecked(seconds)
    }
}

impl<C: Clock> CouncilTransport for InMemoryCouncilTransport<C> {
    fn publish(
        &self,
        authenticated_sender: &AuthenticatedSender,
        envelope: CouncilEnvelope,
    ) -> Result<PublishedCouncilMessage, TransportError> {
        if authenticated_sender.cthulhu_id != envelope.sender_cthulhu_id
            || authenticated_sender.tentacle_id != envelope.sender_tentacle_id
        {
            return Err(TransportError::SenderMismatch);
        }
        let now = self.now();
        envelope.validate_at(now)?;
        if let Some(verifier) = &self.required_verifier {
            envelope
                .verify_signature(verifier.as_ref())
                .map_err(|_| TransportError::SignatureRejected)?;
        }

        let mut state = self
            .state
            .lock()
            .map_err(|_| TransportError::StateUnavailable)?;
        if state.retained.len() >= self.max_retained {
            return Err(TransportError::RetentionLimit);
        }
        let sender_key = (
            authenticated_sender.cthulhu_id.clone(),
            authenticated_sender.tentacle_id.clone(),
        );
        if let Some(rate) = state.sender_rates.get(&sender_key) {
            if now.as_unix_seconds() < rate.started_at {
                return Err(TransportError::ClockMovedBackwards);
            }
            if now.as_unix_seconds() - rate.started_at < SENDER_RATE_WINDOW_SECONDS
                && rate.messages >= MAX_MESSAGES_PER_SENDER_PER_WINDOW
            {
                return Err(TransportError::RateLimited);
            }
        }
        state.replay_guard.check_and_record(&envelope, now)?;
        let rate = state
            .sender_rates
            .entry(sender_key)
            .or_insert(SenderRateWindow {
                started_at: now.as_unix_seconds(),
                messages: 0,
            });
        if now.as_unix_seconds() - rate.started_at >= SENDER_RATE_WINDOW_SECONDS {
            rate.started_at = now.as_unix_seconds();
            rate.messages = 0;
        }
        rate.messages += 1;
        let published = PublishedCouncilMessage {
            order: state.next_order,
            message_id: envelope.message_id.clone(),
            envelope,
        };
        state.next_order = state
            .next_order
            .checked_add(1)
            .ok_or(TransportError::RetentionLimit)?;
        state.retained.push(published.clone());
        Ok(published)
    }

    fn subscribe(
        &self,
        council_id: &CouncilId,
        after: TransportCursor,
        limit: usize,
    ) -> Result<Vec<PublishedCouncilMessage>, TransportError> {
        if limit == 0 || limit > MAX_SUBSCRIPTION_BATCH {
            return Err(TransportError::InvalidBatchSize);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| TransportError::StateUnavailable)?;
        Ok(state
            .retained
            .iter()
            .filter(|record| {
                record.order > after.order() && &record.envelope.council_id == council_id
            })
            .take(limit)
            .cloned()
            .collect())
    }
}

/// Boundary for a future official XMTP group implementation.
///
/// It deliberately implements no network behavior, and all operations return an explicit error.
pub struct XmtpGroupCouncilTransport;

impl CouncilTransport for XmtpGroupCouncilTransport {
    fn publish(
        &self,
        _authenticated_sender: &AuthenticatedSender,
        _envelope: CouncilEnvelope,
    ) -> Result<PublishedCouncilMessage, TransportError> {
        Err(TransportError::XmtpAdapterUnavailable)
    }

    fn subscribe(
        &self,
        _council_id: &CouncilId,
        _after: TransportCursor,
        _limit: usize,
    ) -> Result<Vec<PublishedCouncilMessage>, TransportError> {
        Err(TransportError::XmtpAdapterUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use cthuwu_protocol::{
        CouncilMemberWithdraw, CouncilPayload, MessageId, ValidationErrorKind,
        test_signing::DeterministicTestSigner,
    };

    fn envelope(sequence: u64, message_id: &str) -> CouncilEnvelope {
        let cthulhu_id = CthulhuId::new("cthulhu_archivist").unwrap();
        CouncilEnvelope::new(
            MessageId::new(message_id).unwrap(),
            CouncilId::new("council_local").unwrap(),
            cthulhu_id.clone(),
            TentacleId::new("tentacle_archivist").unwrap(),
            Timestamp::from_unix_seconds(100).unwrap(),
            Timestamp::from_unix_seconds(200).unwrap(),
            sequence,
            CouncilPayload::CouncilMemberWithdraw(CouncilMemberWithdraw {
                cthulhu_id,
                reason: Some("fixture".into()),
            }),
        )
    }

    fn sender() -> AuthenticatedSender {
        AuthenticatedSender {
            cthulhu_id: CthulhuId::new("cthulhu_archivist").unwrap(),
            tentacle_id: TentacleId::new("tentacle_archivist").unwrap(),
        }
    }

    #[test]
    fn publishes_and_replays_in_stable_transport_order() {
        let transport = InMemoryCouncilTransport::new_local(ManualClock::new(110), 8, 4).unwrap();
        let first = transport
            .publish(&sender(), envelope(1, "msg_first"))
            .unwrap();
        let second = transport
            .publish(&sender(), envelope(2, "msg_second"))
            .unwrap();
        assert_eq!((first.order, second.order), (1, 2));

        let council = CouncilId::new("council_local").unwrap();
        let replay = transport
            .subscribe(&council, TransportCursor::beginning(), 8)
            .unwrap();
        assert_eq!(
            replay.iter().map(|entry| entry.order).collect::<Vec<_>>(),
            vec![1, 2]
        );
        let tail = transport.subscribe(&council, first.cursor(), 8).unwrap();
        assert_eq!(tail, vec![second]);
    }

    #[test]
    fn rejects_transport_sender_mismatch_and_replay() {
        let transport = InMemoryCouncilTransport::new_local(ManualClock::new(110), 8, 4).unwrap();
        let message = envelope(1, "msg_once");
        let wrong = AuthenticatedSender {
            cthulhu_id: CthulhuId::new("cthulhu_intruder").unwrap(),
            tentacle_id: sender().tentacle_id,
        };
        assert!(matches!(
            transport.publish(&wrong, message.clone()),
            Err(TransportError::SenderMismatch)
        ));
        transport.publish(&sender(), message.clone()).unwrap();
        let error = transport.publish(&sender(), message).unwrap_err();
        assert!(matches!(
            error,
            TransportError::Invalid(ref validation)
                if validation.kind() == &ValidationErrorKind::Replay
        ));
    }

    #[test]
    fn xmtp_group_boundary_never_claims_live_support() {
        let adapter = XmtpGroupCouncilTransport;
        assert!(matches!(
            adapter.publish(&sender(), envelope(1, "msg_xmtp")),
            Err(TransportError::XmtpAdapterUnavailable)
        ));
    }

    #[test]
    fn configured_verifier_requires_a_valid_signature() {
        let signer = Arc::new(DeterministicTestSigner::new(
            sender().cthulhu_id,
            "fixture-key",
            b"deliberately-not-production",
        ));
        let transport = InMemoryCouncilTransport::new_local(ManualClock::new(110), 8, 4)
            .unwrap()
            .with_required_signatures(signer.clone());
        assert!(matches!(
            transport.publish(&sender(), envelope(1, "msg_unsigned")),
            Err(TransportError::SignatureRejected)
        ));

        let mut signed = envelope(2, "msg_signed_transport");
        signed.attach_signature(signer.as_ref()).unwrap();
        transport.publish(&sender(), signed).unwrap();
    }

    #[test]
    fn per_sender_publish_rate_is_bounded_and_recovers_next_window() {
        let clock = ManualClock::new(110);
        let transport = InMemoryCouncilTransport::new_local(clock.clone(), 130, 4).unwrap();
        for sequence in 1..=MAX_MESSAGES_PER_SENDER_PER_WINDOW as u64 {
            transport
                .publish(
                    &sender(),
                    envelope(sequence, &format!("msg_rate_{sequence}")),
                )
                .unwrap();
        }
        assert!(matches!(
            transport.publish(
                &sender(),
                envelope(
                    MAX_MESSAGES_PER_SENDER_PER_WINDOW as u64 + 1,
                    "msg_rate_limited"
                )
            ),
            Err(TransportError::RateLimited)
        ));

        clock.advance(SENDER_RATE_WINDOW_SECONDS as u64);
        transport
            .publish(
                &sender(),
                envelope(
                    MAX_MESSAGES_PER_SENDER_PER_WINDOW as u64 + 1,
                    "msg_rate_recovered",
                ),
            )
            .unwrap();
    }
}
