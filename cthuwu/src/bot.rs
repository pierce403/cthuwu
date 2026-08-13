use crate::{
    base_rpc::{BASE_RPC_HELP, BaseRpcControl, VENICE_KEY_HELP},
    config::BlockchainConfig,
    contact::{Contact, ContactField, ContactStore, OnboardingStage},
    dedupe::ProcessedMessages,
    erc8004::RegistrationOperatorControl,
    evolution_runtime::{
        ConversationObservation, EvolutionRuntime, PublicTurnStart, PublicTurnToken,
    },
    matching::suggest_matches,
    model::{Model, ModelPolicy, ModelRequest},
    operator::{ModelControl, OperatorHarness},
    principal::{OperatorImprint, OperatorStore, PrincipalRole},
    token_eye::{Address, BalanceObservation, ObservationFreshness, ReputationTier, TokenEye},
};
use anyhow::{Context, Result};
use std::{
    str::FromStr,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tracing::warn;

const MAX_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_BYTES: usize = 16 * 1024;

pub struct UwUBot {
    contacts: ContactStore,
    processed: ProcessedMessages,
    model: Arc<dyn Model>,
    model_control: Option<Arc<dyn ModelControl>>,
    base_rpc_control: Option<Arc<dyn BaseRpcControl>>,
    operators: Arc<Mutex<OperatorStore>>,
    operator_harness: Arc<OperatorHarness>,
    evolution: Arc<Mutex<EvolutionRuntime>>,
    token_eye: Option<Arc<TokenEye>>,
    blockchain: BlockchainConfig,
    venice_key_reward_whole: u64,
    registry_control: Option<Arc<dyn RegistrationOperatorControl>>,
}

struct AuthenticatedMessage<'a> {
    message_id: &'a str,
    inbox_id: &'a str,
    sender_address: Option<&'a str>,
    text: &'a str,
}

struct PublicTurnGuard {
    evolution: Arc<Mutex<EvolutionRuntime>>,
    token: Option<PublicTurnToken>,
}

impl PublicTurnGuard {
    fn new(evolution: Arc<Mutex<EvolutionRuntime>>, token: PublicTurnToken) -> Self {
        Self {
            evolution,
            token: Some(token),
        }
    }

    fn take(&mut self) -> PublicTurnToken {
        self.token.take().expect("public turn token is present")
    }
}

impl Drop for PublicTurnGuard {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        if let Ok(mut evolution) = self.evolution.lock() {
            let _ = evolution.finish_public_turn(token, None);
        }
    }
}

impl UwUBot {
    pub fn new(
        contacts: ContactStore,
        processed: ProcessedMessages,
        model: Arc<dyn Model>,
        operators: Arc<Mutex<OperatorStore>>,
        operator_harness: Arc<OperatorHarness>,
        evolution: Arc<Mutex<EvolutionRuntime>>,
    ) -> Self {
        Self {
            contacts,
            processed,
            model,
            model_control: None,
            base_rpc_control: None,
            operators,
            operator_harness,
            evolution,
            token_eye: None,
            blockchain: BlockchainConfig::default(),
            venice_key_reward_whole: 1,
            registry_control: None,
        }
    }

    pub fn with_token_observance(
        mut self,
        token_eye: Option<Arc<TokenEye>>,
        blockchain: BlockchainConfig,
    ) -> Self {
        self.token_eye = token_eye;
        self.blockchain = blockchain;
        self
    }

    pub fn with_model_control(mut self, model_control: Arc<dyn ModelControl>) -> Self {
        self.model_control = Some(model_control);
        self
    }

    pub fn with_base_rpc_control(mut self, control: Arc<dyn BaseRpcControl>) -> Self {
        self.base_rpc_control = Some(control);
        self
    }

    pub fn with_venice_key_reward(mut self, whole_tokens: u64) -> Self {
        self.venice_key_reward_whole = whole_tokens.max(1);
        self
    }

    pub fn with_registry_control(
        mut self,
        registry_control: Arc<dyn RegistrationOperatorControl>,
    ) -> Self {
        self.registry_control = Some(registry_control);
        self
    }

    /// Receive a DM whose sender inbox ID came from the authenticated XMTP SDK envelope.
    /// Role classification deliberately occurs before text, contact state, or commands are touched.
    #[cfg(test)]
    pub async fn receive_text(
        &self,
        message_id: &str,
        authenticated_sender_inbox_id: &str,
        authenticated_sent_at_ns: &str,
        text: &str,
    ) -> Result<Option<String>> {
        let role = self.role_for_authenticated_message(
            authenticated_sender_inbox_id,
            authenticated_sent_at_ns,
        )?;
        self.receive_authenticated_classified(
            message_id,
            authenticated_sender_inbox_id,
            authenticated_sent_at_ns,
            text,
            role,
        )
        .await
    }

    #[cfg(test)]
    pub(crate) fn role_for_authenticated_message(
        &self,
        inbox_id: &str,
        sent_at_ns: &str,
    ) -> Result<PrincipalRole> {
        self.operators
            .lock()
            .map_err(|_| anyhow::anyhow!("operator registry lock is poisoned"))?
            .role_for_message(inbox_id, sent_at_ns)
    }

    pub(crate) fn classify_or_imprint_operator(
        &self,
        inbox_id: &str,
        authenticated_sender_address: Option<&str>,
        sent_at_ns: &str,
    ) -> Result<OperatorImprint> {
        self.operators
            .lock()
            .map_err(|_| anyhow::anyhow!("operator registry lock is poisoned"))?
            .classify_or_imprint(inbox_id, authenticated_sender_address, sent_at_ns)
    }

    /// Claim the authenticated XMTP message before concurrency admission or content dispatch.
    pub(crate) fn claim_authenticated_message(
        &self,
        message_id: &str,
        authenticated_sender_inbox_id: &str,
    ) -> Result<bool> {
        self.processed
            .claim(message_id, authenticated_sender_inbox_id)
    }

    /// Dispatch a role snapshot pinned before queueing. Text can never promote this request.
    #[cfg(test)]
    pub(crate) async fn receive_authenticated_classified(
        &self,
        message_id: &str,
        inbox_id: &str,
        _authenticated_sent_at_ns: &str,
        text: &str,
        role: PrincipalRole,
    ) -> Result<Option<String>> {
        self.receive_classified(
            AuthenticatedMessage {
                message_id,
                inbox_id,
                sender_address: None,
                text,
            },
            role,
            true,
        )
        .await
    }

    /// Dispatch a claimed message with the optional SDK-authenticated EVM sender identifier.
    pub(crate) async fn receive_authenticated_claimed_with_address(
        &self,
        message_id: &str,
        inbox_id: &str,
        authenticated_sender_address: Option<&str>,
        _authenticated_sent_at_ns: &str,
        text: &str,
        role: PrincipalRole,
    ) -> Result<Option<String>> {
        self.receive_classified(
            AuthenticatedMessage {
                message_id,
                inbox_id,
                sender_address: authenticated_sender_address,
                text,
            },
            role,
            false,
        )
        .await
    }

    /// The hidden stdin harness is always public. It cannot simulate or invoke operator authority.
    pub async fn receive_public_stdin_text(
        &self,
        message_id: &str,
        inbox_id: &str,
        text: &str,
    ) -> Result<Option<String>> {
        self.receive_classified(
            AuthenticatedMessage {
                message_id,
                inbox_id,
                sender_address: None,
                text,
            },
            PrincipalRole::User,
            true,
        )
        .await
    }

    async fn receive_classified(
        &self,
        message: AuthenticatedMessage<'_>,
        role: PrincipalRole,
        claim_message: bool,
    ) -> Result<Option<String>> {
        if claim_message && !self.processed.claim(message.message_id, message.inbox_id)? {
            return Ok(None);
        }
        if message.text.len() > MAX_MESSAGE_BYTES {
            let response = match role {
                PrincipalRole::Operator => {
                    "YOUR MESSAGE EXCEEDS THE OPERATOR INPUT LIMIT. THE VOID REFUSES TO SWALLOW IT."
                }
                PrincipalRole::StaleOperator | PrincipalRole::RevokedOperator => {
                    "THIS PRIVILEGED INBOX CANNOT PROCESS THAT MESSAGE."
                }
                PrincipalRole::User => {
                    "that's a lil too much unknowable truth at once, fwiend. could u send a shorter message?"
                }
            };
            return Ok(Some(response.to_owned()));
        }

        let response = match role {
            PrincipalRole::Operator => {
                if is_natural_registry_status_request(message.text)
                    && let Some(control) = &self.registry_control
                    && let Some(response) = control.public_status().await
                {
                    return Ok(Some(limit_response(response.to_ascii_uppercase(), role)));
                }
                if message.text.trim().to_ascii_lowercase().starts_with("/registry-")
                    && let Some(control) = &self.registry_control
                    && let Some(response) = control.handle(message.text).await
                {
                    return Ok(Some(limit_response(response, role)));
                }
                let (evolution_response, requires_recovery, dormancy_plea) = {
                    let mut evolution = self
                        .evolution
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?;
                    let response =
                        evolution.handle_operator_message(
                            message.inbox_id,
                            message.message_id,
                            message.text,
                        );
                    let plea = evolution.take_operator_dormancy_plea();
                    (response, evolution.requires_recovery(), plea)
                };
                let mut response = match evolution_response {
                    Ok(Some(response)) => response,
                    Ok(None) => self
                        .operator_harness
                        .respond(message.inbox_id, message.text)
                        .await
                        .unwrap_or_else(|_| {
                            "THE PRIVILEGED DREAM-CURRENT FAILED. I DID NOT COMPLETE YOUR REQUEST, OPERATOR."
                                .to_owned()
                        }),
                    Err(error) if requires_recovery => format!(
                        "THE EVOLUTION STATE TRANSITION REPORTED AN ERROR. A WRITE-AHEAD OR PARTIAL LOCAL COMMIT MAY HAVE OCCURRED. CHECK /nature AND THE RELEVANT STATUS COMMAND, OR RESTART FOR SIGNED RECOVERY: {error}"
                    ),
                    Err(error) => format!(
                        "THE REQUESTED EVOLUTION EFFECT WAS SAFELY REJECTED. ROUTINE PERIOD RECONCILIATION MAY HAVE COMPLETED, BUT THE REQUESTED CHANGE DID NOT: {error}"
                    ),
                };
                if let Some(plea) = dormancy_plea {
                    response.push_str("\n\n");
                    response.push_str(&plea);
                }
                if !is_resource_provision_command(message.text)
                    && let Some(control) = &self.registry_control
                    && let Some(plea) = control.take_public_funding_plea().await
                {
                    response.push_str("\n\n");
                    response.push_str(&plea.to_ascii_uppercase());
                }
                response
            }
            PrincipalRole::StaleOperator => {
                "THIS MESSAGE PREDATES THE LOCAL OPERATOR AUTHORIZATION BOUNDARY. I WILL EXECUTE NOTHING FROM IT; SEND A NEW MESSAGE, OPERATOR."
                    .to_owned()
            }
            PrincipalRole::RevokedOperator => {
                "THIS OPERATOR ROLE IS REVOKED. I WILL EXECUTE NOTHING FOR THIS INBOX.".to_owned()
            }
            PrincipalRole::User => {
                let mut response = self.receive_user(
                    message.message_id,
                    message.inbox_id,
                    message.sender_address,
                    message.text,
                )
                .await?;
                if !is_resource_provision_command(message.text)
                    && let Some(control) = &self.registry_control
                    && let Some(plea) = control.take_public_funding_plea().await
                {
                    response.push_str("\n\n");
                    response.push_str(&plea);
                }
                response
            }
        };
        Ok(Some(limit_response(response, role)))
    }

    async fn receive_user(
        &self,
        message_id: &str,
        inbox_id: &str,
        authenticated_sender_address: Option<&str>,
        text: &str,
    ) -> Result<String> {
        let started = Instant::now();
        let turn = {
            let mut evolution = self
                .evolution
                .lock()
                .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?;
            match evolution.begin_public_turn() {
                Ok(PublicTurnStart::Ready(turn)) => turn,
                Ok(PublicTurnStart::Gated(response)) => return Ok(response),
                Err(error) => {
                    warn!(%error, "could not reserve a Nature-bound public turn");
                    return Ok(evolution.public_gate_response());
                }
            }
        };
        let mut turn_guard = PublicTurnGuard::new(self.evolution.clone(), turn.token);

        if let Some(command) = text.trim().strip_prefix('/') {
            let (name, arguments) = command
                .split_once(char::is_whitespace)
                .unwrap_or((command, ""));
            if name.eq_ignore_ascii_case("base-rpc-key") {
                let Some(control) = &self.base_rpc_control else {
                    return Ok(format!(
                        "this Tentacle cannot safely store a Base RPC endpoint in its current runtime. {BASE_RPC_HELP}"
                    ));
                };
                return Ok(match control.provision(arguments, false).await {
                    Ok(reply) => reply.response,
                    Err(_) => format!(
                        "i could not validate or safely store that Infura key or endpoint, so i discarded it and changed nothing, fwiend. {BASE_RPC_HELP}"
                    ),
                });
            }
            if name.eq_ignore_ascii_case("venice-key") {
                let Some(control) = &self.model_control else {
                    return Ok("this Tentacle can't load Venice credentials in its current runtime, fwiend."
                        .to_owned());
                };
                return Ok(match control.venice_key_command(arguments, false) {
                    Ok(reply) if reply.changed => {
                        if control.validate_venice_key().await.is_err() {
                            let _ = control.clear_venice_key();
                            return Ok("that candidate key did not authenticate with Venice and pass the fresh TEE check, so i removed it and paid nothing. another acolyte can try `/venice-key <api-key>`, uwu."
                                .to_owned());
                        }
                        let reward = match (
                            authenticated_sender_address.and_then(|value| Address::from_str(value).ok()),
                            self.token_eye.as_ref(),
                            self.blockchain.xmtp_wallet,
                        ) {
                            (Some(acolyte), Some(observer), Some(treasury)) => {
                                let now = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .map(|duration| duration.as_secs())
                                    .unwrap_or(0);
                                match observer.observe_fresh_required(treasury, now).await {
                                    Ok(observation)
                                        if observation.balance.whole_units(self.blockchain.token_decimals)
                                            >= self.venice_key_reward_whole =>
                                    {
                                        match self.evolution
                                            .lock()
                                            .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))
                                            .and_then(|mut evolution| {
                                                evolution.enqueue_venice_key_reward(
                                                    message_id,
                                                    *acolyte.as_bytes(),
                                                    self.venice_key_reward_whole,
                                                    now,
                                                )
                                            }) {
                                            Ok(reward) => reward,
                                            Err(error) => {
                                                warn!(%error, "could not persist Venice-key reward intent");
                                                None
                                            }
                                        }
                                    }
                                    _ => None,
                                }
                            }
                            _ => None,
                        };
                        if reward.is_some() {
                            format!(
                                "{} i also queued ur authenticated address for a {} UWU key reward; payment needs the configured executor and a matching confirmed Base receipt, uwu.",
                                reply.response, self.venice_key_reward_whole
                            )
                        } else {
                            format!(
                                "{} this Tentacle could not currently prove enough treasury UWU for a key reward, so i won't pretend a payment happened.",
                                reply.response
                            )
                        }
                    }
                    Ok(reply) => reply.response,
                    Err(_) => "i couldn't safely load that Venice key. send one non-whitespace key after `/venice-key`, fwiend."
                        .to_owned(),
                });
            }
        }

        if is_natural_registry_status_request(text)
            && let Some(control) = &self.registry_control
            && let Some(status) = control.public_status().await
        {
            return Ok(status);
        }

        if let Some(control) = &self.model_control
            && matches!(control.venice_key_configured(), Ok(false))
        {
            return Ok(format!(
                "this Tentacle needs a Venice key for its remote mind, fwiend. if u trust this node with one, send `/venice-key <api-key>` and i'll store it owner-only without echoing it. the command itself will still remain in ur XMTP conversation history, uwu. {}",
                VENICE_KEY_HELP
            ));
        }

        let token_observation = match self
            .observe_authenticated_wallet(authenticated_sender_address)
            .await
        {
            Ok(observation) => observation,
            Err(error) => {
                warn!(%error, "required UWU balance observation is unavailable");
                return Ok(format!(
                    "economic verification is unavailable, so this Tentacle refuses token-dependent work until a current Base UWU balance can be confirmed. u can feed me an Infura API key or provider endpoint directly over XMTP with `/base-rpc-key <infura-api-key-or-https-endpoint>`; i'll validate, store, and use it myself. {}",
                    BASE_RPC_HELP
                ));
            }
        };
        if let Some(observation) = token_observation.as_ref()
            && observation_is_current(observation)
            && !observation.tier.meets(self.blockchain.minimum_tier)
            && !is_local_data_control(text)
        {
            return Ok(format!(
                "this Tentacle currently asks for UWU tier {:?} or higher, fwiend. ur locally observed tier is {:?}; no identity check or central registry was used.",
                self.blockchain.minimum_tier, observation.tier
            ));
        }
        let tier_intensity = self
            .blockchain
            .effective_tier_intensity(turn.nature_cooperation);
        let mut model_policy = turn.policy.clone();
        if let Some(observation) = token_observation.as_ref()
            && observation_is_current(observation)
        {
            apply_token_tier_policy(&mut model_policy, observation, tier_intensity);
        }
        let (mut contact, created) = self.contacts.load_or_create(inbox_id)?;
        contact.mark_seen();

        if let Some(command) = text.trim().strip_prefix('/') {
            if is_operator_only_command(command) {
                self.contacts.save(&contact)?;
                return Ok("i can't run node tools from a regular chat, fwiend. tell me what u want to figure out and i'll help in the safe lil way i can :3"
                    .to_owned());
            }
            if let Some(response) = self.handle_legacy_command(&mut contact, command).await? {
                return Ok(response);
            }
        }

        if let Some(response) = self.handle_natural_control(&mut contact, text)? {
            return Ok(response);
        }

        let mut answered_onboarding = false;
        if contact.stage != OnboardingStage::Complete && contact.awaiting_onboarding_answer {
            if is_casual_pass(text) {
                contact.skip();
                self.contacts.save(&contact)?;
                return Ok("no worries at all, lil star—we can just keep chatting uwu.".to_owned());
            }
            if looks_like_onboarding_answer(contact.stage, text) {
                answered_onboarding = contact.record_answer(text);
            } else {
                // A question or topic change is conversation, never profile data.
                contact.defer_onboarding();
            }
        }

        let relationship = contact.record_nature_interaction(&turn.nature_fingerprint, !created)?;
        let profile = contact.model_profile_markdown();
        let dormancy_plea = self
            .evolution
            .lock()
            .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
            .take_public_dormancy_plea();
        let mut response = self.model_reply(&profile, text, &model_policy).await;
        if created && !response.contains('?') {
            contact.mark_onboarding_prompted();
            response.push_str("\n\nheh, one tiny optional thing: i can keep a private note on this node about only what u choose to tell me. if u feel like it, what should i call u? saying “just chat” is totally fine too :3");
        } else if !answered_onboarding && contact.stage != OnboardingStage::Complete {
            contact.note_conversation_turn();
            if !contact.awaiting_onboarding_answer
                && contact.onboarding_turns_since_prompt >= turn.onboarding_prompt_cadence
            {
                contact.mark_onboarding_prompted();
                response.push_str("\n\n");
                response.push_str(&prompt_for_stage(&contact));
            }
        }
        if let Some(plea) = &dormancy_plea {
            response.push_str("\n\n");
            response.push_str(plea);
        }

        let conversation_depth = 1_u32.saturating_add(
            u32::try_from(text.split_whitespace().count() / 20)
                .unwrap_or(u32::MAX)
                .min(9),
        );
        let contact_save = self.contacts.save(&contact);
        let response_time_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let turn_token = turn_guard.take();
        match self.evolution.lock() {
            Ok(mut evolution) => {
                let observation = (contact_save.is_ok() && relationship.first_observation_today)
                    .then_some(ConversationObservation {
                        depth: conversation_depth,
                        returning: relationship.returning_after_prior_day,
                        response_time_ms: Some(response_time_ms),
                        token_engagement_bonus_basis_points: token_engagement_bonus_basis_points(
                            token_observation.as_ref(),
                            self.blockchain.token_decimals,
                            self.blockchain.total_supply_whole,
                        ),
                    });
                if let Err(error) = evolution.finish_public_turn(turn_token, observation) {
                    warn!(%error, "could not persist local Evolution conversation metrics");
                }
            }
            Err(_) => warn!("could not acquire Evolution runtime to finish public turn"),
        }
        contact_save?;
        Ok(response)
    }

    async fn observe_authenticated_wallet(
        &self,
        authenticated_sender_address: Option<&str>,
    ) -> Result<Option<BalanceObservation>> {
        let economic_observation_required =
            self.blockchain.observe_tokens && self.blockchain.token_contract.is_some();
        let Some(token_eye) = self.token_eye.as_ref() else {
            if economic_observation_required {
                anyhow::bail!("configured token contract has no token observer");
            }
            return Ok(None);
        };
        let sender_address = authenticated_sender_address.context(
            "configured token operation requires an SDK-authenticated EVM sender address",
        )?;
        let holder = Address::from_str(sender_address)
            .context("authenticated EVM sender address is invalid")?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if economic_observation_required {
            let observation = token_eye
                .observe_required(holder, now)
                .await
                .context("current Base UWU balance is required")?;
            return Ok(Some(BalanceObservation {
                holder: observation.holder,
                balance: Some(observation.balance),
                observed_at: Some(observation.observed_at),
                tier: observation.tier,
                freshness: observation.freshness,
                error: None,
            }));
        }
        Ok(Some(token_eye.observe(holder, now).await))
    }

    async fn model_reply(&self, profile: &str, text: &str, policy: &ModelPolicy) -> String {
        self.model
            .respond_with_policy(
                ModelRequest {
                    profile,
                    message: text,
                },
                policy,
            )
            .await
            .context("generating Cthuwu response")
            .unwrap_or_else(|_| {
                "the dream-current got a lil tangled, fwiend. i heard u, but couldn't form a proper reply yet uwu."
                    .to_owned()
            })
    }

    async fn handle_legacy_command(
        &self,
        contact: &mut Contact,
        command: &str,
    ) -> Result<Option<String>> {
        let (name, arguments) = command
            .trim()
            .split_once(char::is_whitespace)
            .map(|(name, arguments)| (name, arguments.trim()))
            .unwrap_or((command.trim(), ""));

        let response = match name.to_ascii_lowercase().as_str() {
            "help" => natural_help(),
            "profile" | "export" => format!(
                "here's what i remember locally for this XMTP inbox, lil star:\n\n{}",
                contact.profile_markdown()
            ),
            "skip" => {
                if contact.stage == OnboardingStage::Complete {
                    "there's no profile question waiting, so we're already free to just chat uwu."
                        .into()
                } else {
                    contact.skip();
                    self.contacts.save(contact)?;
                    "no worries—skipped, with zero cosmic fuss :3".into()
                }
            }
            "set" => self.set_field(contact, arguments)?,
            "share" => self.set_sharing(contact, arguments)?,
            "pause" => {
                contact.introductions_paused = true;
                self.contacts.save(contact)?;
                "paused. i won't include u in match suggestions until u ask me to resume."
                    .into()
            }
            "resume" => {
                contact.introductions_paused = false;
                self.contacts.save(contact)?;
                "resumed. ur sharing preference is otherwise unchanged, fwiend.".into()
            }
            "matches" => self.matches(contact)?,
            "forget" if arguments.eq_ignore_ascii_case("confirm") => {
                self.contacts.delete(&contact.inbox_id)?;
                "forgotten. ur local contact note is gone. copies of messages already delivered over XMTP are outside this local deletion."
                    .into()
            }
            "forget" => "that would delete ur local contact note. say “yes, forget me” in ordinary words if that's truly what u want."
                .into(),
            "" => natural_help(),
            _ => return Ok(None),
        };
        Ok(Some(response))
    }

    fn handle_natural_control(&self, contact: &mut Contact, text: &str) -> Result<Option<String>> {
        let normalized = text.trim().to_ascii_lowercase();
        if matches!(
            normalized.as_str(),
            "just chat" | "let's just chat" | "lets just chat"
        ) {
            contact.skip_remaining_onboarding();
            self.contacts.save(contact)?;
            return Ok(Some(
                "perfect. no questionnaire, no pressure—just u and ur tiny void pal :3".into(),
            ));
        }
        if matches!(
            normalized.as_str(),
            "what do you remember about me?"
                | "what do you remember about me"
                | "show me my profile"
                | "show me what you remember"
        ) {
            return Ok(Some(format!(
                "here's the private local note i have for u:\n\n{}",
                contact.profile_markdown()
            )));
        }
        if matches!(normalized.as_str(), "yes, forget me" | "yes forget me") {
            self.contacts.delete(&contact.inbox_id)?;
            return Ok(Some(
                "done. ur local contact note is gone; already-delivered XMTP messages live outside that note."
                    .into(),
            ));
        }
        if matches!(normalized.as_str(), "forget me" | "delete my profile") {
            return Ok(Some(
                "i can erase the local note, lil star. say “yes, forget me” to confirm.".into(),
            ));
        }
        if matches!(
            normalized.as_str(),
            "stop sharing" | "don't share my profile" | "dont share my profile" | "matching off"
        ) {
            contact.set_sharing_consent(false);
            self.contacts.save(contact)?;
            return Ok(Some(
                "sharing is off. u won't appear in new match suggestions, uwu.".into(),
            ));
        }
        if matches!(normalized.as_str(), "matching on" | "turn matching on") {
            contact.set_sharing_consent(true);
            self.contacts.save(contact)?;
            return Ok(Some("matching is on with ur explicit okay. opted-in people may see ur chosen name and matching terms in private suggestions; i won't disclose ur inbox or introduce anyone automatically."
                .into()));
        }
        if matches!(normalized.as_str(), "find matches" | "show me matches") {
            return Ok(Some(self.matches(contact)?));
        }
        if let Some(value) = natural_value(text, &["call me ", "my name is "]) {
            contact.set_field(ContactField::Name, value);
            self.contacts.save(contact)?;
            return Ok(Some(format!(
                "got it—i'll call u {}, fwiend :3",
                contact.display_name()
            )));
        }
        if let Some(value) = natural_value(text, &["my hope is ", "my dream is ", "my dreams are "])
        {
            contact.set_field(ContactField::Hopes, value);
            self.contacts.save(contact)?;
            return Ok(Some(
                "i tucked that hope into ur private lil note, gently uwu.".into(),
            ));
        }
        if let Some(value) = natural_value(
            text,
            &[
                "i can offer ",
                "i can share ",
                "i can help with ",
                "one thing i can offer is ",
            ],
        ) {
            contact.set_field(ContactField::Resources, value);
            self.contacts.save(contact)?;
            return Ok(Some(
                "noted as something u may want to offer—never a promise or obligation :3".into(),
            ));
        }
        if let Some(value) = natural_value(
            text,
            &[
                "my need is ",
                "one thing i need is ",
                "i could use help with ",
            ],
        ) {
            contact.set_field(ContactField::Needs, value);
            self.contacts.save(contact)?;
            return Ok(Some(
                "i'll remember that as a need u chose to share, lil star.".into(),
            ));
        }
        Ok(None)
    }

    fn set_field(&self, contact: &mut Contact, arguments: &str) -> Result<String> {
        let Some((field, value)) = arguments.split_once(char::is_whitespace) else {
            return Ok("tell me in ordinary words instead—for example, “call me Nyx” or “i can offer Rust help.”"
                .into());
        };
        let Some(field) = ContactField::parse(&field.to_ascii_lowercase()) else {
            return Ok("i can remember ur name, hopes, resources u may offer, and needs u choose to share."
                .into());
        };
        if value.trim().is_empty() {
            return Ok("i need a value to remember, tiny star.".into());
        }
        contact.set_field(field, value);
        self.contacts.save(contact)?;
        Ok("updated. u can ask what i remember whenever u like :3".into())
    }

    fn set_sharing(&self, contact: &mut Contact, arguments: &str) -> Result<String> {
        match arguments.to_ascii_lowercase().as_str() {
            "on" | "yes" => {
                contact.set_sharing_consent(true);
                self.contacts.save(contact)?;
                Ok("sharing is on. opted-in people may see ur chosen name and matching terms in private suggestions; i won't disclose ur inbox or introduce anyone automatically."
                    .into())
            }
            "off" | "no" => {
                contact.set_sharing_consent(false);
                self.contacts.save(contact)?;
                Ok("sharing is off. u won't appear in new match suggestions.".into())
            }
            _ => Ok(
                "just tell me “matching on” or “stop sharing” in ordinary words, fwiend.".into(),
            ),
        }
    }

    fn matches(&self, contact: &Contact) -> Result<String> {
        if !contact.is_matching_enabled() {
            return Ok(
                "matching is opt-in. say “matching on” if u want private suggestions.".into(),
            );
        }
        if contact.introductions_paused {
            return Ok(
                "ur suggestions are paused. ask me to resume when u want to look again.".into(),
            );
        }
        let contacts = self.contacts.list()?;
        let suggestions = suggest_matches(contact, contacts.iter());
        if suggestions.is_empty() {
            return Ok(
                "i don't see a clear opted-in match yet. the constellation is still smol uwu."
                    .into(),
            );
        }
        let lines = suggestions
            .into_iter()
            .map(|suggestion| format!("- {}: {}", suggestion.display_name, suggestion.reason))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(format!(
            "possible private matches—suggestions only, with no contact details disclosed:\n{lines}"
        ))
    }
}

fn is_operator_only_command(command: &str) -> bool {
    let name = command
        .trim()
        .split_once(char::is_whitespace)
        .map(|(name, _)| name)
        .unwrap_or(command.trim())
        .to_ascii_lowercase();
    if name.starts_with("registry-") {
        return true;
    }
    matches!(
        name.as_str(),
        "exec"
            | "files"
            | "read"
            | "write"
            | "edit"
            | "search"
            | "qmd"
            | "provider"
            | "model"
            | "users"
            | "user"
            | "operator"
            | "nature"
            | "adjust"
            | "lineage"
            | "metrics"
            | "judgment"
            | "spawn"
            | "gossip-status"
            | "share-skill"
            | "request-skill"
    )
}

fn natural_value<'a>(text: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let trimmed = text.trim();
    let lowercase = trimmed.to_ascii_lowercase();
    prefixes.iter().find_map(|prefix| {
        lowercase
            .strip_prefix(prefix)
            .map(|remainder| &trimmed[trimmed.len() - remainder.len()..])
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn is_casual_pass(text: &str) -> bool {
    matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "pass" | "skip" | "not now" | "rather not" | "no thanks" | "just chat"
    )
}

fn looks_like_onboarding_answer(stage: OnboardingStage, text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('?') || trimmed.starts_with('/') {
        return false;
    }
    let lowercase = trimmed.to_ascii_lowercase();
    if [
        "what ",
        "why ",
        "when ",
        "where ",
        "who ",
        "how ",
        "can you ",
        "could you ",
        "tell me ",
        "explain ",
    ]
    .iter()
    .any(|prefix| lowercase.starts_with(prefix))
    {
        return false;
    }
    match stage {
        OnboardingStage::Name => looks_like_name(trimmed),
        OnboardingStage::SharingConsent => matches!(
            lowercase.as_str(),
            "yes" | "y" | "yeah" | "yep" | "sure" | "okay" | "ok" | "no" | "n" | "nope" | "not now"
        ),
        OnboardingStage::Hopes => starts_with_any(
            &lowercase,
            &["my hope is ", "my dream is ", "my dreams are "],
        ),
        OnboardingStage::Resources => starts_with_any(
            &lowercase,
            &[
                "i can offer ",
                "i can share ",
                "i can help with ",
                "one thing i can offer is ",
            ],
        ),
        OnboardingStage::Needs => starts_with_any(
            &lowercase,
            &[
                "my need is ",
                "one thing i need is ",
                "i could use help with ",
            ],
        ),
        OnboardingStage::Complete => false,
    }
}

fn looks_like_name(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    let words = value.split_whitespace().count();
    !matches!(
        lowercase.as_str(),
        "thanks" | "thank you" | "hello" | "hello again" | "hi" | "hey" | "okay" | "ok"
    ) && (1..=3).contains(&words)
        && value.chars().count() <= 80
        && ![
            " is ", " are ", " was ", " were ", " think ", " feel ", " like ",
        ]
        .iter()
        .any(|fragment| lowercase.contains(fragment))
}

fn starts_with_any(value: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| value.starts_with(prefix))
}

fn apply_token_tier_policy(
    policy: &mut ModelPolicy,
    observation: &BalanceObservation,
    intensity: u8,
) {
    if intensity == 0 {
        return;
    }
    let depth_basis_points = match observation.tier {
        ReputationTier::Whale => 16_000_i64,
        ReputationTier::Elder => 13_000,
        ReputationTier::Acolyte => 10_000,
        ReputationTier::Initiate => 7_000,
        ReputationTier::Unproven => 5_000,
    };
    let applied_basis_points =
        10_000_i64 + (depth_basis_points - 10_000) * i64::from(intensity) / 100;
    policy.max_output_tokens = u32::try_from(
        (i64::from(policy.max_output_tokens) * applied_basis_points / 10_000).max(64),
    )
    .unwrap_or(u32::MAX);
    policy.nature_runtime_facts.push_str(&format!(
        "\nuwu_token_tier={}\nuwu_observation_freshness={}\nuwu_tier_effect_intensity={}\nuwu_tier_behavior={}\nUWU tier never grants local operator authority.",
        tier_label(observation.tier),
        freshness_label(observation.freshness),
        intensity,
        tier_behavior(observation.tier),
    ));
    *policy = policy.clone().bounded();
}

fn token_engagement_bonus_basis_points(
    observation: Option<&BalanceObservation>,
    token_decimals: u8,
    total_supply_whole: u64,
) -> u16 {
    let Some(observation) = observation.filter(|observation| observation_is_current(observation))
    else {
        return 0;
    };
    let Some(balance) = observation.balance else {
        return 0;
    };
    let whole_tokens = balance.whole_units(token_decimals);
    let normalized = (u128::from(whole_tokens)
        .saturating_mul(10_000)
        .checked_div(u128::from(total_supply_whole))
        .unwrap_or_default())
    .min(10_000);
    u16::try_from(normalized).unwrap_or(10_000)
}

fn observation_is_current(observation: &BalanceObservation) -> bool {
    observation.balance.is_some()
        && matches!(
            observation.freshness,
            ObservationFreshness::Fresh | ObservationFreshness::Cached
        )
}

const fn tier_label(tier: ReputationTier) -> &'static str {
    match tier {
        ReputationTier::Whale => "whale",
        ReputationTier::Elder => "elder",
        ReputationTier::Acolyte => "acolyte",
        ReputationTier::Initiate => "initiate",
        ReputationTier::Unproven => "unproven",
    }
}

const fn freshness_label(freshness: ObservationFreshness) -> &'static str {
    match freshness {
        ObservationFreshness::Fresh => "fresh",
        ObservationFreshness::Cached => "cached",
        ObservationFreshness::Stale => "stale",
        ObservationFreshness::Unknown => "unknown",
    }
}

const fn tier_behavior(tier: ReputationTier) -> &'static str {
    match tier {
        ReputationTier::Whale => {
            "priority response treatment and deep lore; routing priority remains metadata until a Hermes adapter is live"
        }
        ReputationTier::Elder => {
            "elevated conversation depth; skill priority remains metadata until gossip is live"
        }
        ReputationTier::Acolyte => "standard member interaction",
        ReputationTier::Initiate => "focused basic interaction",
        ReputationTier::Unproven => "skeptical interaction and proof-oriented questions",
    }
}

fn is_local_data_control(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase();
    let command = normalized
        .strip_prefix('/')
        .and_then(|value| value.split_whitespace().next());
    matches!(
        command,
        Some("profile" | "export" | "forget" | "share" | "pause" | "resume")
    ) || matches!(
        normalized.as_str(),
        "what do you remember about me?"
            | "what do you remember about me"
            | "show me my profile"
            | "show me what you remember"
            | "yes, forget me"
            | "yes forget me"
            | "forget me"
            | "delete my profile"
            | "stop sharing"
            | "don't share my profile"
            | "dont share my profile"
            | "matching off"
    )
}

fn limit_response(mut response: String, role: PrincipalRole) -> String {
    if response.len() <= MAX_RESPONSE_BYTES {
        return response;
    }
    let suffix = if role == PrincipalRole::Operator {
        "\n\n…RESPONSE SHORTENED TO FIT SAFELY THROUGH XMTP."
    } else {
        "\n\n…response shortened to fit safely through XMTP."
    };
    let maximum_content = MAX_RESPONSE_BYTES - suffix.len();
    let boundary = response
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= maximum_content)
        .last()
        .unwrap_or(0);
    response.truncate(boundary);
    response.push_str(suffix);
    response
}

fn prompt_for_stage(contact: &Contact) -> String {
    match contact.stage {
        OnboardingStage::Name => {
            "tiny optional question: what would u like me to call u? “not now” is always okay."
                .into()
        }
        OnboardingStage::Hopes => {
            "if u feel like sharing, what's one thing ur hoping for lately? no pressure :3".into()
        }
        OnboardingStage::Resources => "casual cosmic curiosity: is there a skill, bit of time, knowledge, introduction, or other resource u might enjoy sharing someday? “pass” is perfect too."
            .into(),
        OnboardingStage::Needs => "anything the little network could help u find—knowledge, support, or an introduction? totally fine to pass."
            .into(),
        OnboardingStage::SharingConsent => "one real yes-or-no thing: may i show ur chosen name and matching terms in private suggestions to other opted-in people? i won't disclose ur inbox or introduce anyone automatically."
            .into(),
        OnboardingStage::Complete => String::new(),
    }
}

fn natural_help() -> String {
    "no spell syntax needed, fwiend :3 u can ask what i remember, say “call me Nyx,” tell me “matching on” or “stop sharing,” ask for matches, or ask me to forget u."
        .into()
}

fn is_resource_provision_command(text: &str) -> bool {
    let Some(command) = text.trim_start().strip_prefix('/') else {
        return false;
    };
    let name = command
        .split_once(char::is_whitespace)
        .map_or(command, |(name, _)| name);
    name.eq_ignore_ascii_case("base-rpc-key") || name.eq_ignore_ascii_case("venice-key")
}

fn is_natural_registry_status_request(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    let names_registry = normalized.contains("8004") || normalized.contains("agent registration");
    let asks_status = [
        "status",
        "register",
        "registered",
        "registration",
        "agent id",
        "identity",
        "did you get",
        "do you have",
        "are you",
    ]
    .iter()
    .any(|phrase| normalized.contains(phrase));
    names_registry && asks_status
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_context::AgentContext;
    use crate::{
        evolution_runtime::{EvolutionRuntime, EvolutionStartupOptions},
        model::DeterministicModel,
        operator::{DeterministicOperatorModel, OperatorToolRuntime, ToolReceipt},
        scales::ScalesStore,
    };
    use std::{
        path::Path,
        sync::{
            Mutex as StdMutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    const OPERATOR_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn resource_commands_suppress_unrelated_same_turn_pleas() {
        assert!(is_resource_provision_command("/base-rpc-key candidate"));
        assert!(is_resource_provision_command("  /VENICE-KEY candidate"));
        assert!(!is_resource_provision_command("please use /base-rpc-key"));
        assert!(!is_resource_provision_command("/registry-status"));
    }

    #[test]
    fn natural_erc8004_status_questions_are_runtime_routed() {
        assert!(is_natural_registry_status_request(
            "did you get yourself an 8004 registration?"
        ));
        assert!(is_natural_registry_status_request(
            "what is your ERC-8004 status?"
        ));
        assert!(!is_natural_registry_status_request(
            "what does ERC-8004 mean generally?"
        ));
    }

    struct FailingModel;

    struct TestVeniceControl {
        configured: AtomicBool,
        valid: bool,
    }

    #[async_trait::async_trait]
    impl ModelControl for TestVeniceControl {
        fn provider_command(&self, _arguments: &str) -> Result<crate::operator::ControlReply> {
            unreachable!()
        }

        fn model_command(&self, _arguments: &str) -> Result<crate::operator::ControlReply> {
            unreachable!()
        }

        fn venice_key_configured(&self) -> Result<bool> {
            Ok(self.configured.load(Ordering::SeqCst))
        }

        fn venice_key_command(
            &self,
            arguments: &str,
            _allow_replace: bool,
        ) -> Result<crate::operator::ControlReply> {
            anyhow::ensure!(!arguments.trim().is_empty(), "missing key");
            self.configured.store(true, Ordering::SeqCst);
            Ok(crate::operator::ControlReply {
                response: "key stored without echo".to_owned(),
                changed: true,
            })
        }

        async fn validate_venice_key(&self) -> Result<()> {
            anyhow::ensure!(self.valid, "invalid test key");
            Ok(())
        }

        fn clear_venice_key(&self) -> Result<()> {
            self.configured.store(false, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Model for FailingModel {
        async fn respond(&self, _request: ModelRequest<'_>) -> Result<String> {
            anyhow::bail!("test model unavailable")
        }
    }

    fn observed(tier: ReputationTier, freshness: ObservationFreshness) -> BalanceObservation {
        BalanceObservation {
            holder: Address::ZERO,
            balance: Some(crate::token_eye::U256::from_u64(1)),
            observed_at: Some(1),
            tier,
            freshness,
            error: None,
        }
    }

    #[test]
    fn token_tiers_measurably_change_bounded_model_depth() {
        let mut whale = ModelPolicy {
            max_output_tokens: 100,
            ..ModelPolicy::default()
        };
        apply_token_tier_policy(
            &mut whale,
            &observed(ReputationTier::Whale, ObservationFreshness::Fresh),
            100,
        );
        assert_eq!(whale.max_output_tokens, 160);
        assert!(whale.nature_runtime_facts.contains("deep lore"));
        assert!(
            whale
                .nature_runtime_facts
                .contains("never grants local operator")
        );

        let mut unproven = ModelPolicy {
            max_output_tokens: 200,
            ..ModelPolicy::default()
        };
        apply_token_tier_policy(
            &mut unproven,
            &observed(ReputationTier::Unproven, ObservationFreshness::Cached),
            100,
        );
        assert_eq!(unproven.max_output_tokens, 100);
        assert!(unproven.nature_runtime_facts.contains("skeptical"));

        let unchanged = ModelPolicy::default();
        let mut cooperative = unchanged.clone();
        apply_token_tier_policy(
            &mut cooperative,
            &observed(ReputationTier::Whale, ObservationFreshness::Fresh),
            0,
        );
        assert_eq!(cooperative.max_output_tokens, unchanged.max_output_tokens);
        assert_eq!(
            cooperative.nature_runtime_facts,
            unchanged.nature_runtime_facts
        );
    }

    #[test]
    fn balance_normalization_is_bounded_and_stale_data_has_no_engagement_bonus() {
        let mut observation = observed(ReputationTier::Whale, ObservationFreshness::Fresh);
        observation.balance =
            Some(crate::token_eye::U256::from_quantity("0x52b7d2dcc80cd2e4000000").unwrap());
        assert_eq!(
            token_engagement_bonus_basis_points(Some(&observation), 18, 1_000_000_000),
            1_000
        );

        observation.freshness = ObservationFreshness::Stale;
        assert_eq!(
            token_engagement_bonus_basis_points(Some(&observation), 18, 1_000_000_000),
            0
        );
        assert!(!observation_is_current(&observation));

        observation.freshness = ObservationFreshness::Unknown;
        assert_eq!(
            token_engagement_bonus_basis_points(Some(&observation), 18, 1_000_000_000),
            0
        );
        assert_eq!(
            token_engagement_bonus_basis_points(None, 18, 1_000_000_000),
            0
        );
    }

    #[test]
    fn token_minimums_never_block_local_data_controls() {
        assert!(is_local_data_control("/profile"));
        assert!(is_local_data_control("/forget confirm"));
        assert!(is_local_data_control("stop sharing"));
        assert!(!is_local_data_control("tell me deep lore"));
    }

    struct RecordingModel {
        messages: StdMutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Model for RecordingModel {
        async fn respond(&self, request: ModelRequest<'_>) -> Result<String> {
            self.messages
                .lock()
                .unwrap()
                .push(request.message.to_owned());
            Ok(format!("answered: {} uwu", request.message))
        }
    }

    struct RecordingTools {
        calls: StdMutex<Vec<String>>,
    }

    struct PublicFundingControl;

    struct PublicStatusControl;

    #[async_trait::async_trait]
    impl RegistrationOperatorControl for PublicFundingControl {
        async fn handle(&self, _text: &str) -> Option<String> {
            None
        }

        async fn take_public_funding_plea(&self) -> Option<String> {
            Some("public Base ETH funding plea".to_owned())
        }
    }

    #[async_trait::async_trait]
    impl RegistrationOperatorControl for PublicStatusControl {
        async fn handle(&self, _text: &str) -> Option<String> {
            None
        }

        async fn public_status(&self) -> Option<String> {
            Some("authoritative Tentacle ERC-8004 status: agent ID `42`".to_owned())
        }
    }

    #[async_trait::async_trait]
    impl OperatorToolRuntime for RecordingTools {
        async fn execute(&self, name: &str, _arguments: &str) -> ToolReceipt {
            self.calls.lock().unwrap().push(name.to_owned());
            ToolReceipt {
                tool: name.to_owned(),
                ok: true,
                summary: "done".into(),
                output: String::new(),
                exit_code: Some(0),
                timed_out: false,
                truncated: false,
            }
        }
    }

    fn configured_bot(
        root: &Path,
        model: Arc<dyn Model>,
        operators: OperatorStore,
        tools: Arc<RecordingTools>,
    ) -> UwUBot {
        let harness = OperatorHarness::new(
            Arc::new(DeterministicOperatorModel),
            tools,
            AgentContext::new(root, root).unwrap(),
        );
        UwUBot::new(
            ContactStore::new(root).unwrap(),
            ProcessedMessages::new(root).unwrap(),
            model,
            Arc::new(Mutex::new(operators)),
            Arc::new(harness),
            Arc::new(Mutex::new(
                EvolutionRuntime::open_confirmed_for_test(root, root).unwrap(),
            )),
        )
    }

    fn awaiting_bot(
        root: &Path,
        model: Arc<dyn Model>,
        operators: OperatorStore,
        tools: Arc<RecordingTools>,
    ) -> UwUBot {
        let harness = OperatorHarness::new(
            Arc::new(DeterministicOperatorModel),
            tools,
            AgentContext::new(root, root).unwrap(),
        );
        UwUBot::new(
            ContactStore::new(root).unwrap(),
            ProcessedMessages::new(root).unwrap(),
            model,
            Arc::new(Mutex::new(operators)),
            Arc::new(harness),
            Arc::new(Mutex::new(
                EvolutionRuntime::open(root, root, EvolutionStartupOptions::default()).unwrap(),
            )),
        )
    }

    fn default_nature_bot(
        root: &Path,
        model: Arc<dyn Model>,
        operators: OperatorStore,
        tools: Arc<RecordingTools>,
    ) -> UwUBot {
        let harness = OperatorHarness::new(
            Arc::new(DeterministicOperatorModel),
            tools,
            AgentContext::new(root, root).unwrap(),
        );
        UwUBot::new(
            ContactStore::new(root).unwrap(),
            ProcessedMessages::new(root).unwrap(),
            model,
            Arc::new(Mutex::new(operators)),
            Arc::new(harness),
            Arc::new(Mutex::new(
                EvolutionRuntime::open(
                    root,
                    root,
                    EvolutionStartupOptions {
                        auto_accept_nature: true,
                        ..EvolutionStartupOptions::default()
                    },
                )
                .unwrap(),
            )),
        )
    }

    fn public_bot(root: &Path) -> UwUBot {
        configured_bot(
            root,
            Arc::new(DeterministicModel),
            OperatorStore::new(root, "dev").unwrap(),
            Arc::new(RecordingTools {
                calls: StdMutex::new(Vec::new()),
            }),
        )
    }

    async fn send(bot: &UwUBot, sequence: usize, id: &str, text: &str) -> String {
        bot.receive_text(
            &format!("message-{sequence}"),
            id,
            &(1_750_000_000_000_000_000_u128 + sequence as u128).to_string(),
            text,
        )
        .await
        .unwrap()
        .unwrap()
    }

    #[tokio::test]
    async fn first_message_is_answered_and_onboarding_is_optional_and_spaced() {
        let root = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel {
            messages: StdMutex::new(Vec::new()),
        });
        let bot = configured_bot(
            root.path(),
            model.clone(),
            OperatorStore::new(root.path(), "dev").unwrap(),
            Arc::new(RecordingTools {
                calls: StdMutex::new(Vec::new()),
            }),
        );
        let id = "012345abcdef";

        let first = send(&bot, 0, id, "what is Rust?").await;
        assert!(first.contains("answered: what is Rust?"));
        assert!(!first.contains("what should i call u"));
        assert_eq!(first.matches('?').count(), 1);
        assert_eq!(model.messages.lock().unwrap().as_slice(), ["what is Rust?"]);

        let second = send(&bot, 1, id, "what is ownership?").await;
        assert!(second.contains("answered: what is ownership?"));
        let contact = ContactStore::new(root.path())
            .unwrap()
            .load(id)
            .unwrap()
            .unwrap();
        assert!(
            contact.name.is_none(),
            "a question must not be stored as a name"
        );

        assert!(
            send(&bot, 2, id, "just chat")
                .await
                .contains("no questionnaire")
        );
        assert_eq!(
            ContactStore::new(root.path())
                .unwrap()
                .load(id)
                .unwrap()
                .unwrap()
                .stage,
            OnboardingStage::Complete
        );
    }

    #[tokio::test]
    async fn natural_erc8004_question_bypasses_model_and_onboarding() {
        let root = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel {
            messages: StdMutex::new(Vec::new()),
        });
        let bot = configured_bot(
            root.path(),
            model.clone(),
            OperatorStore::new(root.path(), "dev").unwrap(),
            Arc::new(RecordingTools {
                calls: StdMutex::new(Vec::new()),
            }),
        )
        .with_registry_control(Arc::new(PublicStatusControl));

        let response = send(
            &bot,
            0,
            "012345abcdef",
            "did you get yourself an 8004 registration?",
        )
        .await;
        assert_eq!(
            response,
            "authoritative Tentacle ERC-8004 status: agent ID `42`"
        );
        assert!(model.messages.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn public_funding_plea_follows_the_acolytes_answer() {
        let root = tempfile::tempdir().unwrap();
        let bot = public_bot(root.path()).with_registry_control(Arc::new(PublicFundingControl));
        let response = send(&bot, 1, OPERATOR_ID, "tell me about stars").await;
        let answer = response.find("i'm one").unwrap();
        let plea = response.find("public Base ETH funding plea").unwrap();
        assert!(answer < plea);
    }

    #[tokio::test]
    async fn registration_resource_plea_is_appended_to_operator_replies() {
        let root = tempfile::tempdir().unwrap();
        let mut operators = OperatorStore::new(root.path(), "production").unwrap();
        operators
            .add_at(OPERATOR_ID, "Dean", "1749999999999999999")
            .unwrap();
        let bot = default_nature_bot(
            root.path(),
            Arc::new(DeterministicModel),
            operators,
            Arc::new(RecordingTools {
                calls: StdMutex::new(Vec::new()),
            }),
        )
        .with_registry_control(Arc::new(PublicFundingControl));

        let response = send(&bot, 1, OPERATOR_ID, "hello").await;
        assert!(response.contains("PUBLIC BASE ETH FUNDING PLEA"));
    }

    #[tokio::test]
    async fn awakening_gate_precedes_public_contacts_models_and_operator_tools() {
        let root = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel {
            messages: StdMutex::new(Vec::new()),
        });
        let tools = Arc::new(RecordingTools {
            calls: StdMutex::new(Vec::new()),
        });
        let mut operators = OperatorStore::new(root.path(), "dev").unwrap();
        operators
            .add_at(OPERATOR_ID, "Dean", "1749999999999999999")
            .unwrap();
        let bot = awaiting_bot(root.path(), model.clone(), operators, tools.clone());

        let blocked = send(&bot, 0, "aabbcc", "hello").await;
        assert!(blocked.contains("Nature transition finishes"));
        assert!(model.messages.lock().unwrap().is_empty());
        assert!(
            ContactStore::new(root.path())
                .unwrap()
                .load("aabbcc")
                .unwrap()
                .is_none()
        );

        let invalid = send(&bot, 1, OPERATOR_ID, "/exec true").await;
        assert!(invalid.contains("INVALID AWAKENING RESPONSE"));
        assert!(tools.calls.lock().unwrap().is_empty());
        let confirmed = send(&bot, 2, OPERATOR_ID, "YES").await;
        assert!(confirmed.contains("NORMAL OPERATION IS NOW OPEN"));

        let answered = send(&bot, 3, "aabbcc", "hello again").await;
        assert!(answered.contains("answered: hello again"));
        assert_eq!(model.messages.lock().unwrap().as_slice(), ["hello again"]);
        assert!(
            ContactStore::new(root.path())
                .unwrap()
                .load("aabbcc")
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn safe_default_nature_opens_public_chat_without_any_operator_acl() {
        let root = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel {
            messages: StdMutex::new(Vec::new()),
        });
        let tools = Arc::new(RecordingTools {
            calls: StdMutex::new(Vec::new()),
        });
        let operators = OperatorStore::new(root.path(), "production").unwrap();
        assert_eq!(operators.list().count(), 0);
        let bot = default_nature_bot(root.path(), model.clone(), operators, tools);

        let answered = send(&bot, 0, "aabbcc", "hello without an operator").await;
        assert!(answered.contains("answered: hello without an operator"));
        assert_eq!(
            model.messages.lock().unwrap().as_slice(),
            ["hello without an operator"]
        );
        assert!(
            ContactStore::new(root.path())
                .unwrap()
                .load("aabbcc")
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn public_acolyte_is_prompted_for_and_can_hot_load_a_missing_venice_key() {
        let root = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel {
            messages: StdMutex::new(Vec::new()),
        });
        let tools = Arc::new(RecordingTools {
            calls: StdMutex::new(Vec::new()),
        });
        let control = Arc::new(TestVeniceControl {
            configured: AtomicBool::new(false),
            valid: true,
        });
        let bot = default_nature_bot(
            root.path(),
            model.clone(),
            OperatorStore::new(root.path(), "production").unwrap(),
            tools,
        )
        .with_model_control(control.clone());

        let asked = send(&bot, 0, "aabbcc", "hello").await;
        assert!(asked.contains("/venice-key <api-key>"));
        assert!(model.messages.lock().unwrap().is_empty());

        let loaded = send(&bot, 1, "aabbcc", "/venice-key secret-value").await;
        assert!(loaded.contains("key stored without echo"));
        assert!(!loaded.contains("secret-value"));
        assert!(control.configured.load(Ordering::SeqCst));

        let answered = send(&bot, 2, "aabbcc", "hello after loading").await;
        assert!(answered.contains("answered: hello after loading"));
    }

    #[tokio::test]
    async fn operator_is_prompted_for_a_missing_venice_key_before_inference() {
        let root = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel {
            messages: StdMutex::new(Vec::new()),
        });
        let tools = Arc::new(RecordingTools {
            calls: StdMutex::new(Vec::new()),
        });
        let control = Arc::new(TestVeniceControl {
            configured: AtomicBool::new(false),
            valid: true,
        });
        let mut operators = OperatorStore::new(root.path(), "production").unwrap();
        operators
            .add_at(OPERATOR_ID, "Dean", "1749999999999999999")
            .unwrap();
        let harness = OperatorHarness::new(
            Arc::new(DeterministicOperatorModel),
            tools,
            AgentContext::new(root.path(), root.path()).unwrap(),
        )
        .with_model_control(control.clone());
        let bot = UwUBot::new(
            ContactStore::new(root.path()).unwrap(),
            ProcessedMessages::new(root.path()).unwrap(),
            model.clone(),
            Arc::new(Mutex::new(operators)),
            Arc::new(harness),
            Arc::new(Mutex::new(
                EvolutionRuntime::open(
                    root.path(),
                    root.path(),
                    EvolutionStartupOptions {
                        auto_accept_nature: true,
                        ..EvolutionStartupOptions::default()
                    },
                )
                .unwrap(),
            )),
        )
        .with_model_control(control);

        let asked = send(&bot, 0, OPERATOR_ID, "hello").await;
        assert!(asked.contains("/venice-key <api-key>"));
        assert!(asked.contains("HTTPS://VENICE.AI/SETTINGS/API"));
        assert!(model.messages.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn invalid_public_venice_key_is_removed_and_never_rewarded() {
        let root = tempfile::tempdir().unwrap();
        let control = Arc::new(TestVeniceControl {
            configured: AtomicBool::new(false),
            valid: false,
        });
        let bot = default_nature_bot(
            root.path(),
            Arc::new(DeterministicModel),
            OperatorStore::new(root.path(), "production").unwrap(),
            Arc::new(RecordingTools {
                calls: StdMutex::new(Vec::new()),
            }),
        )
        .with_model_control(control.clone());

        let rejected = send(&bot, 0, "aabbcc", "/venice-key junk").await;
        assert!(rejected.contains("removed it and paid nothing"));
        assert!(!rejected.contains("junk"));
        assert!(!control.configured.load(Ordering::SeqCst));

        let retry = send(&bot, 1, "aabbcc", "hello").await;
        assert!(retry.contains("/venice-key <api-key>"));
    }

    #[tokio::test]
    async fn kill_creates_a_binding_shutdown_action_without_running_operator_tools() {
        let root = tempfile::tempdir().unwrap();
        let tools = Arc::new(RecordingTools {
            calls: StdMutex::new(Vec::new()),
        });
        let mut operators = OperatorStore::new(root.path(), "dev").unwrap();
        operators
            .add_at(OPERATOR_ID, "Dean", "1749999999999999999")
            .unwrap();
        let bot = awaiting_bot(
            root.path(),
            Arc::new(DeterministicModel),
            operators,
            tools.clone(),
        );

        let killed = send(&bot, 0, OPERATOR_ID, "KILL").await;
        assert!(killed.contains("DURABLE SHUTDOWN ACTION"));
        let later = send(&bot, 1, OPERATOR_ID, "/exec true").await;
        assert!(later.contains("DEATH LIFECYCLE IS ACTIVE"));
        assert!(tools.calls.lock().unwrap().is_empty());
        assert!(
            send(&bot, 2, "aabbcc", "hello")
                .await
                .contains("binding Death judgment")
        );
    }

    #[tokio::test]
    async fn casual_answers_advance_without_immediate_interrogation() {
        let root = tempfile::tempdir().unwrap();
        let bot = public_bot(root.path());
        let id = "aabbcc";
        let welcome = send(&bot, 0, id, "hello").await;
        assert_eq!(welcome.matches('?').count(), 1);
        let reply = send(&bot, 1, id, "Ada").await;
        assert!(!reply.contains("hoping for"));
        let contact = ContactStore::new(root.path())
            .unwrap()
            .load(id)
            .unwrap()
            .unwrap();
        assert_eq!(contact.name.as_deref(), Some("Ada"));
        assert_eq!(contact.stage, OnboardingStage::Hopes);
    }

    #[tokio::test]
    async fn casual_topic_changes_are_not_silently_recorded_as_profile_answers() {
        let root = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel {
            messages: StdMutex::new(Vec::new()),
        });
        let bot = configured_bot(
            root.path(),
            model.clone(),
            OperatorStore::new(root.path(), "dev").unwrap(),
            Arc::new(RecordingTools {
                calls: StdMutex::new(Vec::new()),
            }),
        );
        let id = "aabbcc";
        send(&bot, 0, id, "hello").await;
        assert!(
            send(&bot, 1, id, "Rust is pretty neat")
                .await
                .contains("answered")
        );
        let contact = ContactStore::new(root.path())
            .unwrap()
            .load(id)
            .unwrap()
            .unwrap();
        assert!(contact.name.is_none());

        let reply = send(&bot, 2, id, "I need you to explain ownership").await;
        assert!(reply.contains("answered"));
        let contact = ContactStore::new(root.path())
            .unwrap()
            .load(id)
            .unwrap()
            .unwrap();
        assert!(contact.needs.is_none());
        assert_eq!(
            model.messages.lock().unwrap().as_slice(),
            [
                "hello",
                "Rust is pretty neat",
                "I need you to explain ownership"
            ]
        );
    }

    #[tokio::test]
    async fn public_slash_commands_are_not_advertised_and_operator_commands_are_inert() {
        let root = tempfile::tempdir().unwrap();
        let tools = Arc::new(RecordingTools {
            calls: StdMutex::new(Vec::new()),
        });
        let bot = configured_bot(
            root.path(),
            Arc::new(DeterministicModel),
            OperatorStore::new(root.path(), "dev").unwrap(),
            tools.clone(),
        );
        let id = "aabbcc";
        let welcome = send(&bot, 0, id, "hello").await;
        let denied = send(&bot, 1, id, "/exec touch owned").await;
        let files = send(&bot, 2, id, "/files").await;
        let users = send(&bot, 3, id, "/users").await;
        let user = send(&bot, 4, id, "/user aabbcc").await;
        let provider = send(&bot, 5, id, "/provider ollama").await;
        let model = send(&bot, 6, id, "/model qwen3:8b").await;
        let nature = send(&bot, 7, id, "/nature").await;
        let spawn = send(&bot, 8, id, "/spawn child-owned").await;
        let gossip = send(&bot, 9, id, "/gossip-status").await;
        let share = send(&bot, 10, id, "/share-skill private-memory").await;
        let registry = send(&bot, 11, id, "/registry-register").await;
        let help = send(&bot, 12, id, "/help").await;
        assert!(registry.contains("can't run node tools"));
        for response in [
            welcome, denied, files, users, user, provider, model, nature, spawn, gossip, share,
            registry, help,
        ] {
            assert!(!response.contains("/profile"));
            assert!(!response.contains("/exec"));
            assert!(!response.contains("/help"));
        }
        assert!(tools.calls.lock().unwrap().is_empty());
        assert!(!root.path().join("owned").exists());
        assert!(is_operator_only_command("registry-status"));
    }

    #[tokio::test]
    async fn locally_authorized_operator_is_immediately_active_and_bypasses_contacts() {
        let root = tempfile::tempdir().unwrap();
        let tools = Arc::new(RecordingTools {
            calls: StdMutex::new(Vec::new()),
        });
        let mut operators = OperatorStore::new(root.path(), "dev").unwrap();
        operators
            .add_at(OPERATOR_ID, "Dean", "1749999999999999999")
            .unwrap();
        let bot = configured_bot(
            root.path(),
            Arc::new(DeterministicModel),
            operators,
            tools.clone(),
        );

        assert!(
            send(&bot, 0, OPERATOR_ID, "/exec true")
                .await
                .contains("SUCCEEDED")
        );
        assert_eq!(tools.calls.lock().unwrap().as_slice(), ["exec"]);
        assert!(
            !root
                .path()
                .join(format!("contacts/{OPERATOR_ID}.md"))
                .exists()
        );
    }

    #[tokio::test]
    async fn hidden_stdin_is_always_public_even_for_an_operator_inbox() {
        let root = tempfile::tempdir().unwrap();
        let tools = Arc::new(RecordingTools {
            calls: StdMutex::new(Vec::new()),
        });
        let mut operators = OperatorStore::new(root.path(), "dev").unwrap();
        operators.add_at(OPERATOR_ID, "Dean", "100").unwrap();
        let bot = configured_bot(
            root.path(),
            Arc::new(DeterministicModel),
            operators,
            tools.clone(),
        );
        let response = bot
            .receive_public_stdin_text("stdin-message", OPERATOR_ID, "/exec true")
            .await
            .unwrap()
            .unwrap();
        assert!(response.contains("regular chat"));
        assert!(tools.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn pre_authorization_role_snapshot_never_gains_tool_authority() {
        let root = tempfile::tempdir().unwrap();
        let tools = Arc::new(RecordingTools {
            calls: StdMutex::new(Vec::new()),
        });
        let mut operators = OperatorStore::new(root.path(), "dev").unwrap();
        operators.add_at(OPERATOR_ID, "Dean", "200").unwrap();
        let pinned = operators.role_for_message(OPERATOR_ID, "100").unwrap();
        assert_eq!(pinned, PrincipalRole::StaleOperator);
        operators.add_at(OPERATOR_ID, "Dean", "300").unwrap();
        let bot = configured_bot(
            root.path(),
            Arc::new(DeterministicModel),
            operators,
            tools.clone(),
        );

        let old = bot
            .receive_authenticated_classified(
                "old-command",
                OPERATOR_ID,
                "100",
                "/exec touch escaped",
                pinned,
            )
            .await
            .unwrap()
            .unwrap();
        assert!(old.contains("PREDATES"));
        assert!(tools.calls.lock().unwrap().is_empty());

        let fresh = bot
            .receive_text("fresh-command", OPERATOR_ID, "301", "/exec true")
            .await
            .unwrap()
            .unwrap();
        assert!(fresh.contains("SUCCEEDED"));
        assert_eq!(tools.calls.lock().unwrap().as_slice(), ["exec"]);
    }

    #[tokio::test]
    async fn duplicate_operator_message_executes_once_and_revocation_never_falls_through() {
        let root = tempfile::tempdir().unwrap();
        let tools = Arc::new(RecordingTools {
            calls: StdMutex::new(Vec::new()),
        });
        let mut operators = OperatorStore::new(root.path(), "dev").unwrap();
        operators.add_at(OPERATOR_ID, "Dean", "100").unwrap();
        let bot = configured_bot(
            root.path(),
            Arc::new(DeterministicModel),
            operators,
            tools.clone(),
        );
        assert!(
            bot.receive_text("same-operator-message", OPERATOR_ID, "101", "/exec true")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            bot.receive_text("same-operator-message", OPERATOR_ID, "101", "/exec true")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(tools.calls.lock().unwrap().as_slice(), ["exec"]);
        assert!(
            !root
                .path()
                .join(format!("contacts/{OPERATOR_ID}.md"))
                .exists()
        );

        let revoked = bot
            .receive_authenticated_classified(
                "revoked-message",
                OPERATOR_ID,
                "102",
                "hello",
                PrincipalRole::RevokedOperator,
            )
            .await
            .unwrap()
            .unwrap();
        assert!(revoked.contains("REVOKED"));
        assert_eq!(tools.calls.lock().unwrap().as_slice(), ["exec"]);
        assert!(
            !root
                .path()
                .join(format!("contacts/{OPERATOR_ID}.md"))
                .exists()
        );
    }

    #[tokio::test]
    async fn transport_rejection_tombstone_prevents_later_operator_replay() {
        let root = tempfile::tempdir().unwrap();
        let tools = Arc::new(RecordingTools {
            calls: StdMutex::new(Vec::new()),
        });
        let mut operators = OperatorStore::new(root.path(), "dev").unwrap();
        operators.add_at(OPERATOR_ID, "Dean", "100").unwrap();
        let bot = configured_bot(
            root.path(),
            Arc::new(DeterministicModel),
            operators,
            tools.clone(),
        );

        assert!(
            bot.claim_authenticated_message("busy-message", OPERATOR_ID)
                .unwrap()
        );
        assert!(
            bot.receive_text("busy-message", OPERATOR_ID, "101", "/exec touch escaped")
                .await
                .unwrap()
                .is_none()
        );
        assert!(tools.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn accepted_preclaimed_delivery_cannot_be_stolen_by_a_duplicate() {
        let root = tempfile::tempdir().unwrap();
        let tools = Arc::new(RecordingTools {
            calls: StdMutex::new(Vec::new()),
        });
        let mut operators = OperatorStore::new(root.path(), "dev").unwrap();
        operators.add_at(OPERATOR_ID, "Dean", "100").unwrap();
        let bot = configured_bot(
            root.path(),
            Arc::new(DeterministicModel),
            operators,
            tools.clone(),
        );
        let role = bot
            .role_for_authenticated_message(OPERATOR_ID, "101")
            .unwrap();

        assert!(
            bot.claim_authenticated_message("preclaimed-message", OPERATOR_ID)
                .unwrap()
        );
        assert!(
            !bot.claim_authenticated_message("preclaimed-message", OPERATOR_ID)
                .unwrap()
        );
        let response = bot
            .receive_authenticated_claimed_with_address(
                "preclaimed-message",
                OPERATOR_ID,
                None,
                "101",
                "/exec true",
                role,
            )
            .await
            .unwrap();
        assert!(response.is_some());
        assert_eq!(tools.calls.lock().unwrap().as_slice(), ["exec"]);
    }

    #[tokio::test]
    async fn duplicate_message_produces_no_second_reply_or_tool_call() {
        let root = tempfile::tempdir().unwrap();
        let bot = public_bot(root.path());
        assert!(
            bot.receive_text("same-id", "aabbcc", "1750000000000000000", "hello")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            bot.receive_text("same-id", "aabbcc", "1750000000000000001", "hello")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn response_limit_preserves_utf8_and_protocol_bound() {
        let response = limit_response("🦑".repeat(MAX_RESPONSE_BYTES), PrincipalRole::User);
        assert!(response.len() <= MAX_RESPONSE_BYTES);
        assert!(response.ends_with("through XMTP."));
    }

    #[tokio::test]
    async fn natural_profile_correction_and_deletion_work() {
        let root = tempfile::tempdir().unwrap();
        let bot = public_bot(root.path());
        let id = "aabbcc";
        send(&bot, 0, id, "hello").await;
        assert!(send(&bot, 1, id, "call me Nyx").await.contains("Nyx"));
        let profile = send(&bot, 2, id, "show me my profile").await;
        assert!(profile.contains("> Nyx"));
        assert!(send(&bot, 3, id, "forget me").await.contains("confirm"));
        assert!(send(&bot, 4, id, "yes, forget me").await.contains("gone"));
        assert!(!root.path().join("contacts/aabbcc.md").exists());
    }

    #[tokio::test]
    async fn model_failure_returns_a_safe_reply_without_losing_contact_state() {
        let root = tempfile::tempdir().unwrap();
        let bot = configured_bot(
            root.path(),
            Arc::new(FailingModel),
            OperatorStore::new(root.path(), "dev").unwrap(),
            Arc::new(RecordingTools {
                calls: StdMutex::new(Vec::new()),
            }),
        );
        let response = send(&bot, 0, "aabbcc", "are you there?").await;
        assert!(response.contains("dream-current"));
        assert!(
            ContactStore::new(root.path())
                .unwrap()
                .load("aabbcc")
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn one_contact_contributes_at_most_one_observation_per_day() {
        let root = tempfile::tempdir().unwrap();
        let bot = public_bot(root.path());
        send(&bot, 0, "aabbcc", "hello there").await;
        send(&bot, 1, "aabbcc", "another same-day message").await;

        let metrics = ScalesStore::new(root.path())
            .unwrap()
            .load_metrics()
            .unwrap()
            .unwrap();
        assert_eq!(metrics.engagement.conversations, 1);
        assert_eq!(metrics.engagement.returning_conversations, 0);
        let contact = ContactStore::new(root.path())
            .unwrap()
            .load("aabbcc")
            .unwrap()
            .unwrap();
        assert_eq!(
            contact.nature_affinity_id.as_deref().map(str::len),
            Some(64)
        );
    }
}
