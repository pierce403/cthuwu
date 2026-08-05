use crate::{
    contact::{Contact, ContactField, ContactStore, OnboardingStage},
    dedupe::ProcessedMessages,
    matching::suggest_matches,
    model::{Model, ModelRequest},
    operator::OperatorHarness,
    principal::{OperatorStore, PrincipalRole},
};
use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};

const MAX_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_BYTES: usize = 16 * 1024;
const ONBOARDING_PROMPT_CADENCE: u32 = 3;

pub struct UwUBot {
    contacts: ContactStore,
    processed: ProcessedMessages,
    model: Arc<dyn Model>,
    operators: Arc<Mutex<OperatorStore>>,
    operator_harness: Arc<OperatorHarness>,
}

impl UwUBot {
    pub fn new(
        contacts: ContactStore,
        processed: ProcessedMessages,
        model: Arc<dyn Model>,
        operators: Arc<Mutex<OperatorStore>>,
        operator_harness: Arc<OperatorHarness>,
    ) -> Self {
        Self {
            contacts,
            processed,
            model,
            operators,
            operator_harness,
        }
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
        authenticated_sent_at_ns: &str,
        text: &str,
        role: PrincipalRole,
    ) -> Result<Option<String>> {
        self.receive_classified(
            message_id,
            inbox_id,
            authenticated_sent_at_ns,
            text,
            role,
            true,
        )
        .await
    }

    /// Dispatch a message whose durable replay claim completed before authority-lane admission.
    pub(crate) async fn receive_authenticated_claimed(
        &self,
        message_id: &str,
        inbox_id: &str,
        authenticated_sent_at_ns: &str,
        text: &str,
        role: PrincipalRole,
    ) -> Result<Option<String>> {
        self.receive_classified(
            message_id,
            inbox_id,
            authenticated_sent_at_ns,
            text,
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
        self.receive_classified(message_id, inbox_id, "0", text, PrincipalRole::User, true)
            .await
    }

    async fn receive_classified(
        &self,
        message_id: &str,
        inbox_id: &str,
        _authenticated_sent_at_ns: &str,
        text: &str,
        role: PrincipalRole,
        claim_message: bool,
    ) -> Result<Option<String>> {
        if claim_message && !self.processed.claim(message_id, inbox_id)? {
            return Ok(None);
        }
        if text.len() > MAX_MESSAGE_BYTES {
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
            PrincipalRole::Operator => self
                .operator_harness
                .respond(text)
                .await
                .unwrap_or_else(|_| {
                    "THE PRIVILEGED DREAM-CURRENT FAILED. I DID NOT COMPLETE YOUR REQUEST, OPERATOR."
                        .to_owned()
                }),
            PrincipalRole::StaleOperator => {
                "THIS MESSAGE PREDATES THE LOCAL OPERATOR AUTHORIZATION BOUNDARY. I WILL EXECUTE NOTHING FROM IT; SEND A NEW MESSAGE, OPERATOR."
                    .to_owned()
            }
            PrincipalRole::RevokedOperator => {
                "THIS OPERATOR ROLE IS REVOKED. I WILL EXECUTE NOTHING FOR THIS INBOX.".to_owned()
            }
            PrincipalRole::User => self.receive_user(inbox_id, text).await?,
        };
        Ok(Some(limit_response(response, role)))
    }

    async fn receive_user(&self, inbox_id: &str, text: &str) -> Result<String> {
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

        let profile = contact.profile_markdown();
        let mut response = self.model_reply(&profile, text).await;

        if created && !response.contains('?') {
            contact.mark_onboarding_prompted();
            response.push_str("\n\nheh, one tiny optional thing: i can keep a private note on this node about only what u choose to tell me. if u feel like it, what should i call u? saying “just chat” is totally fine too :3");
        } else if !answered_onboarding && contact.stage != OnboardingStage::Complete {
            contact.note_conversation_turn();
            if !contact.awaiting_onboarding_answer
                && contact.onboarding_turns_since_prompt >= ONBOARDING_PROMPT_CADENCE
            {
                contact.mark_onboarding_prompted();
                response.push_str("\n\n");
                response.push_str(&prompt_for_stage(&contact));
            }
        }

        self.contacts.save(&contact)?;
        Ok(response)
    }

    async fn model_reply(&self, profile: &str, text: &str) -> String {
        self.model
            .respond(ModelRequest {
                profile,
                message: text,
            })
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
    matches!(
        name.as_str(),
        "exec" | "read" | "write" | "edit" | "search" | "qmd" | "operator"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::DeterministicModel,
        operator::{DeterministicOperatorModel, OperatorToolRuntime, ToolReceipt},
    };
    use std::{path::Path, sync::Mutex as StdMutex};

    const OPERATOR_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    struct FailingModel;

    #[async_trait::async_trait]
    impl Model for FailingModel {
        async fn respond(&self, _request: ModelRequest<'_>) -> Result<String> {
            anyhow::bail!("test model unavailable")
        }
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
        let harness =
            OperatorHarness::new(Arc::new(DeterministicOperatorModel), tools, root.to_owned());
        UwUBot::new(
            ContactStore::new(root).unwrap(),
            ProcessedMessages::new(root).unwrap(),
            model,
            Arc::new(Mutex::new(operators)),
            Arc::new(harness),
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
        let help = send(&bot, 2, id, "/help").await;
        for response in [welcome, denied, help] {
            assert!(!response.contains("/profile"));
            assert!(!response.contains("/exec"));
            assert!(!response.contains("/help"));
        }
        assert!(tools.calls.lock().unwrap().is_empty());
        assert!(!root.path().join("owned").exists());
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
        operators
            .add_at(OPERATOR_ID, "Dean", "100")
            .unwrap();
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
        operators
            .add_at(OPERATOR_ID, "Dean", "100")
            .unwrap();
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
        operators
            .add_at(OPERATOR_ID, "Dean", "100")
            .unwrap();
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
        operators
            .add_at(OPERATOR_ID, "Dean", "100")
            .unwrap();
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
            .receive_authenticated_claimed(
                "preclaimed-message",
                OPERATOR_ID,
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
}
