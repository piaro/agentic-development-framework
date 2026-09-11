//! Cooperative maintenance locking and atomic Record publication.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct StorageGuard {
    _file: Arc<File>,
}

impl StorageGuard {
    pub fn shared(root: &Path) -> Result<Self, String> {
        Self::acquire(root, false)
    }

    pub fn exclusive(root: &Path) -> Result<Self, String> {
        Self::acquire(root, true)
    }

    fn acquire(root: &Path, exclusive: bool) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let relative = ".adf/cache/locks/record-storage.lock";
        let path = root.join(relative);
        reject_symlinks(&root, &path)?;
        fs::create_dir_all(path.parent().ok_or("missing lock parent")?)
            .map_err(|e| e.to_string())?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let locked = if exclusive {
            file.try_lock()
        } else {
            file.try_lock_shared()
        };
        locked.map_err(|e| {
            format!("Record storage is busy; finish active ADF operations and retry: {e}")
        })?;
        Ok(Self {
            _file: Arc::new(file),
        })
    }
}

pub fn reject_symlinks(root: &Path, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "path escapes project")?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return Err("invalid project path".to_owned());
        }
        current.push(part);
        if fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!(
                "symlinked project path is not allowed for Record storage: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

pub fn read_record(path: &Path) -> Result<Vec<u8>, String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .is_file()
    {
        return Err(format!("Record must be a regular file: {}", path.display()));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(crate::record_storage::MAX_EXPANDED_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > crate::record_storage::MAX_EXPANDED_BYTES {
        return Err("Record exceeds storage limit".to_owned());
    }
    Ok(bytes)
}

/// Publish a fully synced sibling, preserving the previous file on failure.
/// A cooperative caller holds the relevant lock for compare-and-replace.
pub fn atomic_write(path: &Path, bytes: &[u8], create_new: bool) -> Result<(), String> {
    let parent = path.parent().ok_or("missing file parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(
        ".adf-storage-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        if let Ok(metadata) = fs::metadata(path) {
            file.set_permissions(metadata.permissions())
                .map_err(|e| e.to_string())?;
        }
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        if read_record(&temporary)? != bytes {
            return Err("temporary Record verification failed".to_owned());
        }
        if create_new {
            fs::hard_link(&temporary, path).map_err(|e| {
                format!(
                    "record already exists or cannot be published: {}: {e}",
                    path.display()
                )
            })?;
            fs::remove_file(&temporary).map_err(|e| e.to_string())?;
        } else {
            crate::project_setup::replace_file(&temporary, path).map_err(|e| e.to_string())?;
        }
        crate::project_setup::sync_directory(parent).map_err(|e| e.to_string())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

pub fn correction_lock(root: &Path) -> Result<File, String> {
    let path = root.join(".adf/cache/locks/result-corrections.lock");
    reject_symlinks(root, &path)?;
    fs::create_dir_all(path.parent().ok_or("missing lock parent")?).map_err(|e| e.to_string())?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.lock().map_err(|e| e.to_string())?;
    Ok(file)
}
