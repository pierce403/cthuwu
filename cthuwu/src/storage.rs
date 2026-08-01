use anyhow::{Context, Result, bail};
use std::{fs, fs::File, path::Path};

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
}
