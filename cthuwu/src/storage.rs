use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{fs, fs::File, path::Path};

const SHA256_BLOCK_BYTES: usize = 64;

pub fn ensure_private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))?;
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("inspecting {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("{} must be a real directory, not a symlink", path.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("restricting {}", path.display()))?;
    }
    Ok(())
}

pub fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("syncing directory {}", path.display()))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn restrict_file(file: &File, description: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .with_context(|| format!("restricting {description}"))?;
    }
    #[cfg(not(unix))]
    let _ = (file, description);
    Ok(())
}

/// Computes RFC 2104 HMAC-SHA256 without introducing a second cryptography dependency.
///
/// Runtime state uses this only for local integrity/authenticity tags. Network protocols must
/// still bind messages to their transport-authenticated sender and must not treat this symmetric
/// primitive as a public signature.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut key_block = [0_u8; SHA256_BLOCK_BYTES];
    if key.len() > SHA256_BLOCK_BYTES {
        key_block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut inner_pad = [0x36_u8; SHA256_BLOCK_BYTES];
    let mut outer_pad = [0x5c_u8; SHA256_BLOCK_BYTES];
    for index in 0..SHA256_BLOCK_BYTES {
        inner_pad[index] ^= key_block[index];
        outer_pad[index] ^= key_block[index];
    }

    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().into()
}

/// Compares authentication tags without data-dependent early exit.
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn restricts_runtime_directories_and_files() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("private");
        ensure_private_directory(&directory).unwrap();
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );

        let path = directory.join("secret");
        let file = File::create(&path).unwrap();
        restrict_file(&file, "test secret").unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn rejects_a_symlink_as_a_private_directory() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        fs::create_dir(&real).unwrap();
        let linked = root.path().join("linked");
        symlink(&real, &linked).unwrap();
        assert!(ensure_private_directory(&linked).is_err());
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231_and_compares_in_constant_time() {
        let tag = hmac_sha256(&[0x0b; 20], b"Hi There");
        let rendered = tag
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            rendered,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert!(constant_time_eq(&tag, &tag));
        let mut changed = tag;
        changed[31] ^= 1;
        assert!(!constant_time_eq(&tag, &changed));
        assert!(!constant_time_eq(&tag, &tag[..31]));
    }
}
