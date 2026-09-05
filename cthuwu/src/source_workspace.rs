//! Download the source independently of model availability. Updates remain operator tasks.
use crate::{operator::run_process, workspace_runtime::WorkspaceRuntime};
use anyhow::{Context, Result};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub async fn bootstrap(root: PathBuf, workspace: Arc<WorkspaceRuntime>) {
    if let Err(error) = initialize(&root).await {
        tracing::warn!(%error, "source checkout unavailable; /update can retry after environment repair");
    }
    if let Err(error) =
        workspace.checkpoint("Initialize the Tentacle source checkout and prime-Tentacle record")
    {
        tracing::warn!(%error, "source bootstrap workspace checkpoint failed");
    }
}

async fn initialize(root: &Path) -> Result<()> {
    let receipt = source_command(root, "init").await?;
    anyhow::ensure!(receipt.ok, "source bootstrap failed: {}", receipt.summary);
    receipt
        .exit_code
        .context("source bootstrap returned no process receipt")?;
    Ok(())
}

pub async fn status(root: &Path) -> Result<crate::operator::ToolReceipt> {
    source_command(root, "status").await
}

async fn source_command(root: &Path, operation: &str) -> Result<crate::operator::ToolReceipt> {
    // Execute the compiled helper at startup, not an arbitrary existing workspace script.
    // NamedTempFile cleans up on task cancellation and never uses the OS temp directory.
    let mut helper = tempfile::NamedTempFile::new_in(root.join("tmp"))?;
    crate::storage::restrict_file(helper.as_file(), "source bootstrap helper")?;
    helper.write_all(include_bytes!("../../scripts/code.py"))?;
    helper.as_file().sync_all()?;
    let arguments = vec![
        helper.path().to_string_lossy().into_owned(),
        "--root".into(),
        root.to_string_lossy().into_owned(),
        operation.into(),
    ];
    run_process(
        "source_workspace",
        Path::new("python3"),
        &arguments,
        root,
        Duration::from_secs(180),
    )
    .await
}
