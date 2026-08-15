//! Durable, public-safe names for independently operated Tentacles.
//!
//! The two-part phonetic seed is inspired by the earlier Apache-2.0
//! `pierce403/cthulhu-launcher` utility, expanded with a third epithet so a growing
//! collective does not immediately fill the available name space.

use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};

pub const ELDRITCH_NAME_SCHEME: &str = "eldritch-v1";

const PREFIXES: &[&str] = &[
    "Cth", "Azath", "Nyarl", "Yog", "Shub", "Dagon", "Hastur", "Ithaqua", "Tsath",
    "Vhul", "Xoth", "Zhar", "Rhan", "Ghatan", "Thog", "Yibb", "Nyth", "Kthan", "Oth",
    "Ubb", "Zoth", "Mnar", "Quach", "Yugg",
];

const SUFFIXES: &[&str] = &[
    "ulhu", "oth", "athotep", "soth", "ogtha", "oggua", "ogga", "aroth", "ith", "uun",
    "aqua", "eph", "orr", "ygg", "azhul", "omoth", "ath", "orath", "uth", "ezzar",
    "othra", "agoth", "yoth", "uloth", "azath", "uul", "egha", "othoth", "ir", "uunath",
    "ael", "yrrh",
];

const EPITHETS: &[&str] = &[
    "the Star-Entombed",
    "of the Gloaming Rift",
    "the Moonless Whisper",
    "of Sunken Y'ha-nthlei",
    "the Unblinking Deep",
    "of the Violet Gulf",
    "the Dreaming Maw",
    "of the Ninth Tide",
    "the Ashen Oracle",
    "of the Hollow Moon",
    "the Door Beneath",
    "of Nameless Carcosa",
    "the Salt-Crowned",
    "of the Last Constellation",
    "the Patient Hunger",
    "of the Black Nebula",
    "the Lantern Below",
    "of the Drowned Archive",
    "the Velvet Eclipse",
    "of the Outer Dark",
    "the Spiral Witness",
    "of the Sleeping Trench",
    "the Quiet Cataclysm",
    "of the Crooked Stars",
];

/// Maps the already-random, durable Tentacle ID to a stable public name.
///
/// Callers still persist the resulting string so future naming schemes cannot silently rename an
/// existing node. Domain-separated derivation also reproduces the default after registry-state
/// recovery without coupling the name to mutable Nature traits.
pub fn generate_eldritch_name(tentacle_id: &str) -> Result<String> {
    ensure!(!tentacle_id.is_empty(), "Tentacle ID must not be empty");
    let mut hasher = Sha256::new();
    hasher.update(b"cthuwu:");
    hasher.update(ELDRITCH_NAME_SCHEME.as_bytes());
    hasher.update(b"\0");
    hasher.update(tentacle_id.as_bytes());
    let digest = hasher.finalize();
    let mut entropy = [0_u8; 6];
    entropy.copy_from_slice(&digest[..6]);
    Ok(name_from_entropy(entropy))
}

fn name_from_entropy(entropy: [u8; 6]) -> String {
    let prefix = PREFIXES[index(entropy[0], entropy[1], PREFIXES.len())];
    let suffix = SUFFIXES[index(entropy[2], entropy[3], SUFFIXES.len())];
    let epithet = EPITHETS[index(entropy[4], entropy[5], EPITHETS.len())];
    format!("{prefix}{suffix} {epithet}")
}

fn index(high: u8, low: u8, length: usize) -> usize {
    usize::from(u16::from_be_bytes([high, low])) % length
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_vectors_remain_eldritch_and_public_safe() {
        assert_eq!(
            name_from_entropy([0, 0, 0, 0, 0, 0]),
            "Cthulhu the Star-Entombed"
        );
        assert_eq!(
            name_from_entropy([0, 8, 0, 7, 0, 23]),
            "Tsatharoth of the Crooked Stars"
        );
        for name in [
            name_from_entropy([255; 6]),
            name_from_entropy([17, 91, 203, 7, 44, 218]),
        ] {
            assert!(name.len() <= 128);
            assert!(!name.contains('\n'));
            assert!(name.split_whitespace().count() >= 2);
        }
    }

    #[test]
    fn expanded_space_is_not_the_launchers_original_seventy_two_names() {
        assert!(PREFIXES.len() * SUFFIXES.len() * EPITHETS.len() >= 10_000);
    }

    #[test]
    fn stable_tentacle_ids_recover_the_same_default_name() {
        let first = generate_eldritch_name("tentacle-0123456789abcdef").unwrap();
        assert_eq!(ELDRITCH_NAME_SCHEME, "eldritch-v1");
        assert_eq!(first, "Quachath the Quiet Cataclysm");
        assert_eq!(first, generate_eldritch_name("tentacle-0123456789abcdef").unwrap());
        assert_eq!(
            generate_eldritch_name("tentacle-fedcba9876543210").unwrap(),
            "Zothogtha of Sunken Y'ha-nthlei"
        );
    }
}
