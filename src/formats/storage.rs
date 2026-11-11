use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{anyhow, Context, Result};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct StagedFile {
    path: PathBuf,
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn stage(path: &Path, bytes: &[u8]) -> Result<StagedFile> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for _ in 0..100 {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(".texembed-{}-{id}.tmp", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(mut file) => {
                let staged = StagedFile { path: temporary };
                file.write_all(bytes)
                    .with_context(|| format!("Failed to stage {}", path.display()))?;
                file.sync_all()
                    .with_context(|| format!("Failed to flush {}", path.display()))?;
                return Ok(staged);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to stage {}", path.display()))
            }
        }
    }
    Err(anyhow!(
        "Could not create a unique staging file for {}",
        path.display()
    ))
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let staged = stage(path, bytes)?;
    fs::rename(&staged.path, path).with_context(|| {
        format!(
            "Failed to replace {}; the original file was not truncated",
            path.display()
        )
    })
}

pub(super) fn write_pair(first: (&Path, &[u8]), second: (&Path, &[u8])) -> Result<()> {
    if first.0 == second.0 || fs::canonicalize(first.0)? == fs::canonicalize(second.0)? {
        return Err(anyhow!("Cannot save two outputs to the same path"));
    }
    let original =
        fs::read(first.0).with_context(|| format!("Cannot back up {}", first.0.display()))?;
    let rollback = stage(first.0, &original)?;
    let staged_first = stage(first.0, first.1)?;
    let staged_second = stage(second.0, second.1)?;
    drop(
        File::open(second.0)
            .with_context(|| format!("Cannot open paired file {}", second.0.display()))?,
    );
    fs::rename(&staged_first.path, first.0)
        .with_context(|| format!("Cannot replace {}", first.0.display()))?;
    if let Err(error) = fs::rename(&staged_second.path, second.0) {
        if let Err(rollback_error) = fs::rename(&rollback.path, first.0) {
            let recovery = rollback.path.clone();
            std::mem::forget(rollback);
            return Err(anyhow!("Paired save failed: {error}; rollback failed: {rollback_error}. Original data is preserved at {}", recovery.display()));
        }
        return Err(anyhow!(
            "Paired save failed for {}: {error}. The stream was restored",
            second.0.display()
        ));
    }
    Ok(())
}
