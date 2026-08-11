use crate::validation::{bounded_text, validate_slug};
use crate::{CthulhuId, ValidationError, ValidationErrorKind};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Signature {
    pub algorithm: String,
    pub key_id: String,
    pub value: String,
}

impl Signature {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_slug("signature.algorithm", &self.algorithm, 48)?;
        bounded_text("signature.keyId", &self.key_id, 128)?;
        bounded_text("signature.value", &self.value, 1_024)?;
        if !self.key_id.is_ascii()
            || !self.value.is_ascii()
            || self.key_id.chars().any(char::is_whitespace)
            || self.value.chars().any(char::is_whitespace)
        {
            return Err(ValidationError::new(
                "signature",
                ValidationErrorKind::InvalidFormat,
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignatureError(String);

impl SignatureError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SignatureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SignatureError {}

/// Production runtimes provide a signer bound to a legacy v1 Council coordination namespace.
/// This control-plane signature is not an ERC-8004 transaction signer and carries no Cthuwu-wide
/// authority; Cthuwu has no central owner or signing key.
pub trait CouncilSigner {
    fn signer_cthulhu_id(&self) -> &CthulhuId;

    fn sign(&self, canonical_envelope: &[u8]) -> Result<Signature, SignatureError>;
}

/// Production runtimes provide signature verification from their chosen trust mechanism.
pub trait CouncilVerifier {
    fn verify(
        &self,
        sender_cthulhu_id: &CthulhuId,
        canonical_envelope: &[u8],
        signature: &Signature,
    ) -> Result<(), SignatureError>;
}

/// Deterministic and deliberately forgeable. Never use this module for a real Council.
#[cfg(any(test, feature = "test-signer"))]
pub mod test_signing {
    use super::*;

    #[derive(Clone, Debug)]
    pub struct DeterministicTestSigner {
        cthulhu_id: CthulhuId,
        key_id: String,
        test_key: Vec<u8>,
    }

    impl DeterministicTestSigner {
        pub fn new(
            cthulhu_id: CthulhuId,
            key_id: impl Into<String>,
            test_key: impl AsRef<[u8]>,
        ) -> Self {
            Self {
                cthulhu_id,
                key_id: key_id.into(),
                test_key: test_key.as_ref().to_vec(),
            }
        }

        fn digest(&self, message: &[u8]) -> String {
            // FNV-1a is used only to make fixture output reproducible. It provides no authenticity.
            let mut value = 0xcbf29ce484222325_u64;
            for byte in self.test_key.iter().chain(message) {
                value ^= u64::from(*byte);
                value = value.wrapping_mul(0x100000001b3);
            }
            format!("{value:016x}")
        }
    }

    impl CouncilSigner for DeterministicTestSigner {
        fn signer_cthulhu_id(&self) -> &CthulhuId {
            &self.cthulhu_id
        }

        fn sign(&self, canonical_envelope: &[u8]) -> Result<Signature, SignatureError> {
            Ok(Signature {
                algorithm: "test-fnv1a64".into(),
                key_id: self.key_id.clone(),
                value: self.digest(canonical_envelope),
            })
        }
    }

    impl CouncilVerifier for DeterministicTestSigner {
        fn verify(
            &self,
            sender_cthulhu_id: &CthulhuId,
            canonical_envelope: &[u8],
            signature: &Signature,
        ) -> Result<(), SignatureError> {
            if sender_cthulhu_id != &self.cthulhu_id
                || signature.algorithm != "test-fnv1a64"
                || signature.key_id != self.key_id
                || signature.value != self.digest(canonical_envelope)
            {
                return Err(SignatureError::new("deterministic test signature mismatch"));
            }
            Ok(())
        }
    }
}
