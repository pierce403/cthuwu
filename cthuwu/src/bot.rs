use crate::{
    contact::{Contact, ContactField, ContactStore, OnboardingStage},
    dedupe::ProcessedMessages,
    matching::suggest_matches,
    model::{Model, ModelRequest},
};
use anyhow::{Context, Result};
use std::sync::Arc;

const MAX_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_BYTES: usize = 16 * 1024;

pub struct UwUBot {
    contacts: ContactStore,
    processed: ProcessedMessages,
    model: Arc<dyn Model>,
}

impl UwUBot {
    pub fn new(
        contacts: ContactStore,
        processed: ProcessedMessages,
        model: Arc<dyn Model>,
    ) -> Self {
        Self {
            contacts,
            processed,
            model,
        }
    }

    pub async fn receive_text(
        &self,
        message_id: &str,
        inbox_id: &str,
        text: &str,
    ) -> Result<Option<String>> {
        self.receive_text_inner(message_id, inbox_id, text)
            .await
            .map(|response| response.map(limit_response))
    }

    async fn receive_text_inner(
        &self,
        message_id: &str,
        inbox_id: &str,
        text: &str,
    ) -> Result<Option<String>> {
        if !self.processed.claim(message_id, inbox_id)? {
            return Ok(None);
        }
        if text.len() > MAX_MESSAGE_BYTES {
            return Ok(Some(
                "that's a little too much unknowable truth at once. could you send a shorter message?"
                    .into(),
            ));
        }

        let (mut contact, created) = self.contacts.load_or_create(inbox_id)?;
        if created {
            return Ok(Some(
                "hewwo, new friend. i'm cthuwu. i keep a private local contact note so i can remember what you choose to share. use /profile, /set, /skip, or /forget anytime. what would you like me to call you?"
                    .into(),
            ));
        }
        contact.mark_seen();
        self.contacts.save(&contact)?;

        if let Some(command) = text.trim().strip_prefix('/') {
            return self.handle_command(&mut contact, command).await.map(Some);
        }

        if contact.stage != OnboardingStage::Complete {
            let advanced = contact.record_answer(text);
            let response = if advanced {
                prompt_for_stage(&contact)
            } else {
                "please answer yes or no. /skip also means no, and you can change this later with /share on or /share off.".to_owned()
            };
            self.contacts.save(&contact)?;
            return Ok(Some(response));
        }

        let profile = contact.profile_markdown();
        let response = self
            .model
            .respond(ModelRequest {
                profile: &profile,
                message: text,
            })
            .await
            .context("generating Cthuwu response")
            .unwrap_or_else(|_| {
                "the dream-current got a little tangled. i heard you, but couldn't form a proper reply yet."
                    .to_owned()
            });
        Ok(Some(response))
    }

    async fn handle_command(&self, contact: &mut Contact, command: &str) -> Result<String> {
        let (name, arguments) = command
            .trim()
            .split_once(char::is_whitespace)
            .map(|(name, arguments)| (name, arguments.trim()))
            .unwrap_or((command.trim(), ""));

        match name.to_ascii_lowercase().as_str() {
            "help" => Ok(help()),
            "profile" | "export" => Ok(format!(
                "here is what i remember locally for this XMTP inbox:\n\n{}",
                contact.profile_markdown()
            )),
            "skip" => {
                if contact.stage == OnboardingStage::Complete {
                    return Ok("onboarding is already complete. use /set <field> <value> to change something.".into());
                }
                contact.skip();
                let response = prompt_for_stage(contact);
                self.contacts.save(contact)?;
                Ok(response)
            }
            "set" => self.set_field(contact, arguments),
            "share" => self.set_sharing(contact, arguments),
            "pause" => {
                contact.introductions_paused = true;
                self.contacts.save(contact)?;
                Ok("paused. i won't include you in match suggestions until you use /resume.".into())
            }
            "resume" => {
                contact.introductions_paused = false;
                self.contacts.save(contact)?;
                Ok("resumed. your sharing preference is otherwise unchanged.".into())
            }
            "matches" => self.matches(contact),
            "forget" if arguments.eq_ignore_ascii_case("confirm") => {
                self.contacts.delete(&contact.inbox_id)?;
                Ok("forgotten. your local contact note is gone. network copies of messages are outside this local deletion.".into())
            }
            "forget" => Ok(
                "this deletes your local contact note. send /forget confirm if that is what you want."
                    .into(),
            ),
            "" => Ok(help()),
            other => Ok(format!(
                "i don't know /{other}. the small forbidden command list is:\n{}",
                help()
            )),
        }
    }

    fn set_field(&self, contact: &mut Contact, arguments: &str) -> Result<String> {
        let Some((field, value)) = arguments.split_once(char::is_whitespace) else {
            return Ok("usage: /set name|hopes|resources|needs <value>".into());
        };
        let Some(field) = ContactField::parse(&field.to_ascii_lowercase()) else {
            return Ok("field must be name, hopes, resources, or needs.".into());
        };
        if value.trim().is_empty() {
            return Ok("please include a value after the field.".into());
        }
        contact.set_field(field, value);
        self.contacts.save(contact)?;
        Ok("updated. use /profile to inspect what i remember.".into())
    }

    fn set_sharing(&self, contact: &mut Contact, arguments: &str) -> Result<String> {
        match arguments.to_ascii_lowercase().as_str() {
            "on" | "yes" => {
                contact.sharing_enabled = true;
                contact.sharing_consent_version = crate::contact::CURRENT_SHARING_CONSENT_VERSION;
                self.contacts.save(contact)?;
                Ok("sharing is on. opted-in people may see your chosen name and matching terms in private suggestions; i will not disclose your inbox or introduce anyone automatically.".into())
            }
            "off" | "no" => {
                contact.sharing_enabled = false;
                contact.sharing_consent_version = 0;
                self.contacts.save(contact)?;
                Ok("sharing is off. you will not appear in new match suggestions.".into())
            }
            _ => Ok("usage: /share on|off".into()),
        }
    }

    fn matches(&self, contact: &Contact) -> Result<String> {
        if !contact.is_matching_enabled() {
            return Ok("matching is opt-in. use /share on if you want private suggestions.".into());
        }
        if contact.introductions_paused {
            return Ok(
                "your suggestions are paused. use /resume when you want to look again.".into(),
            );
        }
        let contacts = self.contacts.list()?;
        let suggestions = suggest_matches(contact, contacts.iter());
        if suggestions.is_empty() {
            return Ok(
                "i don't see a clear opted-in match yet. the constellation is still small.".into(),
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

fn limit_response(mut response: String) -> String {
    if response.len() <= MAX_RESPONSE_BYTES {
        return response;
    }
    const SUFFIX: &str = "\n\n…response shortened to fit safely through XMTP.";
    let maximum_content = MAX_RESPONSE_BYTES - SUFFIX.len();
    let boundary = response
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= maximum_content)
        .last()
        .unwrap_or(0);
    response.truncate(boundary);
    response.push_str(SUFFIX);
    response
}

fn prompt_for_stage(contact: &Contact) -> String {
    match contact.stage {
        OnboardingStage::Name => "what would you like me to call you?".into(),
        OnboardingStage::Hopes => {
            "lovely to meet you. what are you hoping or dreaming about these days?".into()
        }
        OnboardingStage::Resources => "i'll remember that. what resources, skills, time, knowledge, introductions, objects, or other help might you enjoy sharing? /skip is always okay.".into(),
        OnboardingStage::Needs => "thank you. what resources, introductions, knowledge, or support could the network help you find?".into(),
        OnboardingStage::SharingConsent => "may i show your chosen name and matching terms from your needs and offers in private suggestions to other opted-in people? answer yes or no. i will not disclose your inbox or make introductions automatically.".into(),
        OnboardingStage::Complete if contact.is_matching_enabled() => "the tiny stars have taken note. private matching is on; use /matches to look, /share off to opt out, or /profile to review your note.".into(),
        OnboardingStage::Complete => "the tiny stars have taken note. matching is off. use /share on only if you decide you want private suggestions.".into(),
    }
}

fn help() -> String {
    [
        "/profile — inspect your local contact note",
        "/set name|hopes|resources|needs <value> — correct a field",
        "/skip — skip the current onboarding question",
        "/share on|off — control private match suggestions",
        "/matches — view explainable private suggestions",
        "/pause and /resume — pause or resume suggestions",
        "/forget confirm — delete your local contact note",
        "/help — show these commands",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dedupe::ProcessedMessages, model::DeterministicModel};

    struct FailingModel;

    #[async_trait::async_trait]
    impl Model for FailingModel {
        async fn respond(&self, _request: ModelRequest<'_>) -> Result<String> {
            anyhow::bail!("test model unavailable")
        }
    }

    fn bot(root: &std::path::Path) -> UwUBot {
        UwUBot::new(
            ContactStore::new(root).unwrap(),
            ProcessedMessages::new(root).unwrap(),
            Arc::new(DeterministicModel),
        )
    }

    async fn send(bot: &UwUBot, sequence: usize, id: &str, text: &str) -> String {
        bot.receive_text(&format!("message-{sequence}"), id, text)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn guides_a_new_contact_through_opt_in_onboarding() {
        let root = tempfile::tempdir().unwrap();
        let bot = bot(root.path());
        let id = "012345abcdef";

        assert!(send(&bot, 0, id, "hello").await.contains("private local"));
        assert!(send(&bot, 1, id, "Ada").await.contains("dreaming"));
        assert!(
            send(&bot, 2, id, "A neighborhood workshop")
                .await
                .contains("resources")
        );
        assert!(
            send(&bot, 3, id, "Rust and security reviews")
                .await
                .contains("help you find")
        );
        assert!(
            send(&bot, 4, id, "Introductions to organizers")
                .await
                .contains("private suggestions")
        );
        assert!(send(&bot, 5, id, "yes").await.contains("matching is on"));

        let note = std::fs::read_to_string(root.path().join("contacts/012345abcdef.md")).unwrap();
        assert!(note.contains("> Ada"));
        assert!(note.contains("> A neighborhood workshop"));
        assert!(note.contains("> Rust and security reviews"));
        assert!(note.contains("> Introductions to organizers"));
        assert!(note.contains("onboarding_stage: complete"));
        assert!(note.contains("sharing_enabled: true"));
        assert!(note.contains("sharing_consent_version: 1"));
    }

    #[tokio::test]
    async fn duplicate_message_produces_no_second_reply() {
        let root = tempfile::tempdir().unwrap();
        let bot = bot(root.path());
        assert!(
            bot.receive_text("same-id", "aabbcc", "hello")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            bot.receive_text("same-id", "aabbcc", "hello")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn response_limit_preserves_utf8_and_protocol_bound() {
        let response = limit_response("🦑".repeat(MAX_RESPONSE_BYTES));
        assert!(response.len() <= MAX_RESPONSE_BYTES);
        assert!(response.ends_with("through XMTP."));
    }

    #[tokio::test]
    async fn supports_skip_correction_profile_and_deletion() {
        let root = tempfile::tempdir().unwrap();
        let bot = bot(root.path());
        let id = "aabbcc";
        send(&bot, 0, id, "hello").await;
        send(&bot, 1, id, "/skip").await;
        send(&bot, 2, id, "/set name Nyx").await;
        let profile = send(&bot, 3, id, "/profile").await;
        assert!(profile.contains("> Nyx"));

        assert!(send(&bot, 4, id, "/forget").await.contains("confirm"));
        assert!(send(&bot, 5, id, "/forget confirm").await.contains("gone"));
        assert!(!root.path().join("contacts/aabbcc.md").exists());
    }

    #[tokio::test]
    async fn completed_chat_updates_last_seen_before_model_work() {
        let root = tempfile::tempdir().unwrap();
        let bot = bot(root.path());
        let id = "aabbcc";
        for (sequence, text) in ["hello", "Nyx", "Build a garden", "/skip", "/skip", "no"]
            .into_iter()
            .enumerate()
        {
            send(&bot, sequence, id, text).await;
        }

        let store = ContactStore::new(root.path()).unwrap();
        let mut contact = store.load(id).unwrap().unwrap();
        assert_eq!(contact.stage, OnboardingStage::Complete);
        contact.last_seen = 0;
        store.save(&contact).unwrap();

        send(&bot, 7, id, "hello again").await;
        assert!(store.load(id).unwrap().unwrap().last_seen > 0);
    }

    #[tokio::test]
    async fn ambiguous_sharing_answer_repeats_the_question() {
        let root = tempfile::tempdir().unwrap();
        let bot = bot(root.path());
        let id = "aabbcc";
        for (sequence, text) in ["hello", "Nyx", "/skip", "/skip", "/skip"]
            .into_iter()
            .enumerate()
        {
            send(&bot, sequence, id, text).await;
        }
        let response = send(&bot, 6, id, "maybe").await;
        assert!(response.contains("yes or no"));
        let store = ContactStore::new(root.path()).unwrap();
        assert_eq!(
            store.load(id).unwrap().unwrap().stage,
            OnboardingStage::SharingConsent
        );
    }

    #[tokio::test]
    async fn model_failure_returns_a_safe_reply_without_losing_contact_state() {
        let root = tempfile::tempdir().unwrap();
        let bot = UwUBot::new(
            ContactStore::new(root.path()).unwrap(),
            ProcessedMessages::new(root.path()).unwrap(),
            Arc::new(FailingModel),
        );
        let id = "aabbcc";
        for (sequence, text) in ["hello", "Nyx", "/skip", "/skip", "/skip", "no"]
            .into_iter()
            .enumerate()
        {
            send(&bot, sequence, id, text).await;
        }

        let response = send(&bot, 7, id, "are you there?").await;
        assert!(response.contains("dream-current"));
        assert!(
            ContactStore::new(root.path())
                .unwrap()
                .load(id)
                .unwrap()
                .is_some()
        );
    }
}
