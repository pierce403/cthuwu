use crate::contact::{ContactStore, OnboardingStage};
use anyhow::Result;

pub struct UwUBot {
    contacts: ContactStore,
}

impl UwUBot {
    pub fn new(contacts: ContactStore) -> Self {
        Self { contacts }
    }

    pub fn receive_text(&self, inbox_id: &str, text: &str) -> Result<String> {
        const MAX_MESSAGE_BYTES: usize = 16 * 1024;
        if text.len() > MAX_MESSAGE_BYTES {
            return Ok("that's a little too much unknowable truth at once. could you send a shorter message?".into());
        }

        let (mut contact, created) = self.contacts.load_or_create(inbox_id)?;
        if created {
            return Ok("hewwo, new friend. i'm cthuwu. what would you like me to call you?".into());
        }

        let response = match contact.stage {
            OnboardingStage::Name => {
                contact.record_answer(text);
                "lovely to meet you. what are you hoping or dreaming about these days?"
            }
            OnboardingStage::Hopes => {
                contact.record_answer(text);
                "i'll remember that. what resources, skills, time, knowledge, or other help might you enjoy sharing with a mutual-aid network?"
            }
            OnboardingStage::Resources => {
                contact.record_answer(text);
                "thank you. and what resources, introductions, knowledge, or support could the network help you find?"
            }
            OnboardingStage::Needs => {
                contact.record_answer(text);
                "the tiny stars have taken note. i'll remember what you hope for, what you can share, and what might help you."
            }
            OnboardingStage::Complete => {
                "i'm listening. the wider resource-sharing conversation is still waking up."
            }
        };

        self.contacts.save(&contact)?;
        Ok(response.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guides_a_new_contact_through_onboarding() {
        let root = tempfile::tempdir().unwrap();
        let bot = UwUBot::new(ContactStore::new(root.path()).unwrap());
        let id = "012345abcdef";

        assert!(bot.receive_text(id, "hello").unwrap().contains("call you"));
        assert!(bot.receive_text(id, "Ada").unwrap().contains("dreaming"));
        assert!(bot.receive_text(id, "A neighborhood workshop").unwrap().contains("sharing"));
        assert!(bot.receive_text(id, "Rust and security reviews").unwrap().contains("help you find"));
        assert!(bot.receive_text(id, "Introductions to organizers").unwrap().contains("remember"));

        let note = std::fs::read_to_string(root.path().join("contacts/012345abcdef.md")).unwrap();
        assert!(note.contains("> Ada"));
        assert!(note.contains("> A neighborhood workshop"));
        assert!(note.contains("> Rust and security reviews"));
        assert!(note.contains("> Introductions to organizers"));
        assert!(note.contains("onboarding_stage: complete"));
    }
}
