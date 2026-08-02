use serde::{Serialize, de::DeserializeOwned};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

const MAX_STATE_BYTES: u64 = 8 * 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("Council state path is unsafe: {0}")]
    Unsafe(String),
    #[error("Council state exceeds the {MAX_STATE_BYTES}-byte limit")]
    Oversized,
    #[error("Council state is invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("Council state I/O failed: {0}")]
    Io(#[from] io::Error),
}

/// Atomic, permission-restricted JSON persistence below `<data>/state/council/`.
#[derive(Clone, Debug)]
pub struct CouncilStateStore {
    directory: PathBuf,
}

impl CouncilStateStore {
    pub fn new(data_dir: &Path) -> Result<Self, PersistenceError> {
        let state = checked_directory(data_dir, "data directory")?;
        let state = ensure_directory(&state.join("state"))?;
        let directory = ensure_directory(&state.join("council"))?;
        Ok(Self { directory })
    }

    pub fn save<T: Serialize>(&self, name: &str, value: &T) -> Result<(), PersistenceError> {
        let name = state_filename(name)?;
        let path = self.directory.join(&name);
        reject_symlink_if_present(&path)?;
        let encoded = serde_json::to_vec_pretty(value)?;
        if encoded.len() as u64 > MAX_STATE_BYTES {
            return Err(PersistenceError::Oversized);
        }

        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary =
            self.directory
                .join(format!(".{name}.{}.{}.tmp", std::process::id(), sequence));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        restrict_file(&file)?;
        let result = (|| -> Result<(), PersistenceError> {
            file.write_all(&encoded)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)?;
            restrict_path(&path)?;
            sync_directory(&self.directory)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn load<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, PersistenceError> {
        let path = self.directory.join(state_filename(name)?);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(PersistenceError::Unsafe(format!(
                "{} must be a regular file",
                path.display()
            )));
        }
        if metadata.len() > MAX_STATE_BYTES {
            return Err(PersistenceError::Oversized);
        }
        restrict_path(&path)?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        File::open(path)?
            .take(MAX_STATE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(PersistenceError::Oversized);
        }
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

fn state_filename(name: &str) -> Result<String, PersistenceError> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(PersistenceError::Unsafe("invalid state name".to_owned()));
    }
    Ok(format!("{name}.json"))
}

fn checked_directory(path: &Path, description: &str) -> Result<PathBuf, PersistenceError> {
    let metadata = fs::symlink_metadata(path).map_err(PersistenceError::Io)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PersistenceError::Unsafe(format!(
            "{description} must be a real directory"
        )));
    }
    restrict_directory(path)?;
    Ok(path.to_path_buf())
}

fn ensure_directory(path: &Path) -> Result<PathBuf, PersistenceError> {
    fs::create_dir_all(path)?;
    checked_directory(path, "Council state directory")
}

fn reject_symlink_if_present(path: &Path) -> Result<(), PersistenceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            PersistenceError::Unsafe(format!("{} must be a regular file", path.display())),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn restrict_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn restrict_file(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

fn restrict_path(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Example {
        version: u8,
        values: Vec<String>,
    }

    #[test]
    fn state_round_trips_and_reloads() {
        let root = tempfile::tempdir().unwrap();
        let store = CouncilStateStore::new(root.path()).unwrap();
        let state = Example {
            version: 1,
            values: vec!["lease".to_owned(), "agenda".to_owned()],
        };
        store.save("council-state", &state).unwrap();
        let reloaded = CouncilStateStore::new(root.path())
            .unwrap()
            .load::<Example>("council-state")
            .unwrap()
            .unwrap();
        assert_eq!(reloaded, state);
    }

    #[test]
    fn rejects_path_traversal_names() {
        let root = tempfile::tempdir().unwrap();
        let store = CouncilStateStore::new(root.path()).unwrap();
        assert!(store.save("../../escape", &1).is_err());
    }

    #[test]
    fn rejects_corrupt_and_oversized_state() {
        let root = tempfile::tempdir().unwrap();
        let store = CouncilStateStore::new(root.path()).unwrap();
        fs::write(store.directory().join("corrupt.json"), b"{not-json").unwrap();
        assert!(matches!(
            store.load::<Example>("corrupt"),
            Err(PersistenceError::InvalidJson(_))
        ));

        let oversized = File::create(store.directory().join("oversized.json")).unwrap();
        oversized.set_len(MAX_STATE_BYTES + 1).unwrap();
        assert!(matches!(
            store.load::<Example>("oversized"),
            Err(PersistenceError::Oversized)
        ));

        let exact = "a".repeat(MAX_STATE_BYTES as usize - 2);
        store.save("exact-limit", &exact).unwrap();
        assert_eq!(store.load::<String>("exact-limit").unwrap().unwrap(), exact);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_data_and_state_files() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        fs::create_dir(&real).unwrap();
        let linked = root.path().join("linked");
        symlink(&real, &linked).unwrap();
        assert!(matches!(
            CouncilStateStore::new(&linked),
            Err(PersistenceError::Unsafe(_))
        ));

        let store = CouncilStateStore::new(&real).unwrap();
        let target = real.join("target");
        fs::write(&target, b"1").unwrap();
        symlink(&target, store.directory().join("state.json")).unwrap();
        assert!(matches!(
            store.save("state", &1),
            Err(PersistenceError::Unsafe(_))
        ));
        assert!(matches!(
            store.load::<u8>("state"),
            Err(PersistenceError::Unsafe(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn files_and_directories_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let store = CouncilStateStore::new(root.path()).unwrap();
        store.save("state", &1).unwrap();
        assert_eq!(
            fs::metadata(store.directory())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.directory().join("state.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
