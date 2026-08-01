use crate::{
    contact::normalize_inbox_id,
    storage::{ensure_private_directory, sync_directory},
};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct ProcessedMessages {
    directory: PathBuf,
}

impl ProcessedMessages {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let directory = data_dir.join("state").join("processed");
        ensure_private_directory(&directory)?;
        Ok(Self { directory })
    }

    /// Claims a message before processing it.
    ///
    /// This intentionally favors at-most-once replies: a crash after the claim can drop a
    /// response, but replaying the same network message cannot create duplicate replies.
    pub fn claim(&self, message_id: &str, sender_inbox_id: &str) -> Result<bool> {
        let message_id = message_key(message_id)?;
        normalize_inbox_id(sender_inbox_id)?;
        let path = self.directory.join(message_id);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                file.sync_all()?;
                sync_directory(&self.directory)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error).with_context(|| format!("claiming {}", path.display())),
        }
    }
}

fn message_key(value: &str) -> Result<String> {
    let normalized = value.trim();
    if normalized.is_empty()
        || normalized.len() > 1_024
        || !normalized
            .chars()
            .all(|character| character.is_ascii_graphic())
    {
        bail!("invalid message ID");
    }
    Ok(Sha256::digest(normalized.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_a_message_exactly_once_across_instances() {
        let root = tempfile::tempdir().unwrap();
        let first = ProcessedMessages::new(root.path()).unwrap();
        assert!(first.claim("message-01", "aabbcc").unwrap());
        let markers = std::fs::read_dir(root.path().join("state/processed"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].file_name().to_string_lossy().len(), 64);
        assert_eq!(std::fs::read(markers[0].path()).unwrap(), b"");

        let after_restart = ProcessedMessages::new(root.path()).unwrap();
        assert!(!after_restart.claim("message-01", "aabbcc").unwrap());
        assert!(after_restart.claim("message-02", "aabbcc").unwrap());
    }

    #[test]
    fn hashes_opaque_ids_instead_of_using_them_as_paths() {
        let root = tempfile::tempdir().unwrap();
        let processed = ProcessedMessages::new(root.path()).unwrap();
        assert!(processed.claim("../../oops", "aabbcc").unwrap());
        assert!(!root.path().join("oops").exists());
        assert_eq!(
            std::fs::read_dir(root.path().join("state/processed"))
                .unwrap()
                .count(),
            1
        );
    }
}
