use crate::contact::Contact;
use std::collections::BTreeSet;

#[derive(Debug, Eq, PartialEq)]
pub struct MatchSuggestion {
    pub display_name: String,
    pub reason: String,
    score: usize,
}

pub fn suggest_matches<'a>(
    caller: &Contact,
    contacts: impl IntoIterator<Item = &'a Contact>,
) -> Vec<MatchSuggestion> {
    if !caller.is_matching_enabled() || caller.introductions_paused {
        return Vec::new();
    }

    let caller_needs = tokens(caller.needs.as_deref());
    let caller_offers = tokens(caller.resources.as_deref());
    let mut suggestions = contacts
        .into_iter()
        .filter(|other| {
            other.inbox_id != caller.inbox_id
                && other.is_matching_enabled()
                && !other.introductions_paused
        })
        .filter_map(|other| {
            let needs_from_other = intersection(&caller_needs, &tokens(other.resources.as_deref()));
            let useful_to_other = intersection(&caller_offers, &tokens(other.needs.as_deref()));
            let score = needs_from_other.len() + useful_to_other.len();
            if score == 0 {
                return None;
            }

            let reason = match (needs_from_other.is_empty(), useful_to_other.is_empty()) {
                (false, false) => format!(
                    "you may be able to help each other around {}",
                    joined(&needs_from_other, &useful_to_other)
                ),
                (false, true) => format!(
                    "their opt-in offer overlaps your needs around {}",
                    needs_from_other.join(", ")
                ),
                (true, false) => format!(
                    "your opt-in offer overlaps their needs around {}",
                    useful_to_other.join(", ")
                ),
                (true, true) => unreachable!(),
            };

            Some(MatchSuggestion {
                display_name: other.display_name(),
                reason,
                score,
            })
        })
        .collect::<Vec<_>>();

    suggestions.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    suggestions.truncate(5);
    suggestions
}

fn tokens(value: Option<&str>) -> BTreeSet<String> {
    let mut output = BTreeSet::new();
    let Some(value) = value else {
        return output;
    };
    if value == "_Skipped._" {
        return output;
    }
    for word in value
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
    {
        if (3..=32).contains(&word.len()) && !STOP_WORDS.contains(&word) {
            output.insert(word.to_owned());
        }
    }
    output
}

fn intersection(left: &BTreeSet<String>, right: &BTreeSet<String>) -> Vec<String> {
    left.intersection(right).cloned().collect()
}

fn joined(left: &[String], right: &[String]) -> String {
    let mut words = left
        .iter()
        .chain(right)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    words.truncate(6);
    words.join(", ")
}

const STOP_WORDS: &[&str] = &[
    "about", "also", "and", "are", "can", "for", "from", "have", "help", "into", "might", "need",
    "offer", "other", "some", "that", "the", "their", "them", "they", "this", "time", "want",
    "with", "would",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact::{ContactField, ContactStore};

    fn contact(store: &ContactStore, id: &str, name: &str, offers: &str, needs: &str) -> Contact {
        let (mut contact, _) = store.load_or_create(id).unwrap();
        contact.set_field(ContactField::Name, name);
        contact.set_field(ContactField::Resources, offers);
        contact.set_field(ContactField::Needs, needs);
        contact.sharing_enabled = true;
        contact.sharing_consent_version = crate::contact::CURRENT_SHARING_CONSENT_VERSION;
        contact
    }

    #[test]
    fn produces_explainable_opt_in_matches_without_inbox_ids() {
        let root = tempfile::tempdir().unwrap();
        let store = ContactStore::new(root.path()).unwrap();
        let caller = contact(
            &store,
            "aaaa",
            "Ada",
            "Rust security mentoring",
            "community workshop space",
        );
        let other = contact(
            &store,
            "bbbb",
            "Bo",
            "community workshop space",
            "Rust mentoring",
        );

        let matches = suggest_matches(&caller, [&other]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].display_name, "Bo");
        assert!(matches[0].reason.contains("rust"));
        assert!(matches[0].reason.contains("workshop"));
        assert!(!matches[0].reason.contains("bbbb"));
    }

    #[test]
    fn excludes_contacts_without_bilateral_opt_in() {
        let root = tempfile::tempdir().unwrap();
        let store = ContactStore::new(root.path()).unwrap();
        let mut caller = contact(&store, "aaaa", "Ada", "Rust", "workshop");
        let other = contact(&store, "bbbb", "Bo", "workshop", "Rust");

        caller.sharing_enabled = false;
        assert!(suggest_matches(&caller, [&other]).is_empty());
    }

    #[test]
    fn skipped_fields_do_not_create_matches() {
        let root = tempfile::tempdir().unwrap();
        let store = ContactStore::new(root.path()).unwrap();
        let mut caller = contact(&store, "aaaa", "Ada", "Rust", "workshop");
        let mut other = contact(&store, "bbbb", "Bo", "workshop", "Rust");
        caller.resources = Some("_Skipped._".to_owned());
        caller.needs = Some("_Skipped._".to_owned());
        other.resources = Some("_Skipped._".to_owned());
        other.needs = Some("_Skipped._".to_owned());

        assert!(suggest_matches(&caller, [&other]).is_empty());
    }
}
