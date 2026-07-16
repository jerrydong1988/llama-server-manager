#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    parent_dir(path).join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()))
}

fn sync_file(path: &Path) -> Result<(), String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("failed to sync {}: {error}", path.display()))
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), String> {
    File::open(parent_dir(path))
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "failed to sync parent directory {}: {error}",
                parent_dir(path).display()
            )
        })
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn replace_path_raw(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_IGNORE_MERGE_ERRORS,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let source_wide = wide(source);
    let destination_wide = wide(destination);
    let succeeded = unsafe {
        if destination.exists() {
            ReplaceFileW(
                destination_wide.as_ptr(),
                source_wide.as_ptr(),
                std::ptr::null(),
                REPLACEFILE_IGNORE_MERGE_ERRORS,
                std::ptr::null(),
                std::ptr::null(),
            )
        } else {
            MoveFileExW(
                source_wide.as_ptr(),
                destination_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    if succeeded == 0 {
        return Err(format!(
            "failed to replace {} with {}: {}",
            destination.display(),
            source.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn replace_path_raw(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::rename(source, destination).map_err(|error| {
        format!(
            "failed to replace {} with {}: {error}",
            destination.display(),
            source.display()
        )
    })
}

pub fn replace_file(
    source: &Path,
    destination: &Path,
    backup: Option<&Path>,
) -> Result<(), String> {
    if !source.is_file() {
        return Err(format!(
            "replacement source is not a file: {}",
            source.display()
        ));
    }
    std::fs::create_dir_all(parent_dir(destination)).map_err(|error| {
        format!(
            "failed to create directory {}: {error}",
            parent_dir(destination).display()
        )
    })?;
    sync_file(source)?;

    if let Some(backup_path) = backup.filter(|_| destination.is_file()) {
        std::fs::create_dir_all(parent_dir(backup_path)).map_err(|error| {
            format!(
                "failed to create backup directory {}: {error}",
                parent_dir(backup_path).display()
            )
        })?;
        let backup_temp = temporary_path(backup_path);
        let backup_result = (|| {
            std::fs::copy(destination, &backup_temp).map_err(|error| {
                format!(
                    "failed to prepare backup {}: {error}",
                    backup_path.display()
                )
            })?;
            sync_file(&backup_temp)?;
            replace_path_raw(&backup_temp, backup_path)?;
            sync_parent(backup_path)
        })();
        if backup_result.is_err() {
            let _ = std::fs::remove_file(&backup_temp);
        }
        backup_result?;
    }

    replace_path_raw(source, destination)?;
    sync_parent(destination)
}

pub fn atomic_write(path: &Path, contents: &[u8], backup: Option<&Path>) -> Result<(), String> {
    std::fs::create_dir_all(parent_dir(path)).map_err(|error| {
        format!(
            "failed to create directory {}: {error}",
            parent_dir(path).display()
        )
    })?;
    let temporary = temporary_path(path);
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| {
                format!(
                    "failed to create temporary file {}: {error}",
                    temporary.display()
                )
            })?;
        file.write_all(contents).map_err(|error| {
            format!(
                "failed to write temporary file {}: {error}",
                temporary.display()
            )
        })?;
        file.flush().map_err(|error| {
            format!(
                "failed to flush temporary file {}: {error}",
                temporary.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "failed to sync temporary file {}: {error}",
                temporary.display()
            )
        })?;
        drop(file);
        replace_file(&temporary, path, backup)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("lsm-persistence-{name}-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn atomic_write_replaces_content_and_preserves_backup() {
        let directory = test_dir("backup");
        let path = directory.join("state.json");
        let backup = directory.join("state.json.bak");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, b"old").unwrap();

        atomic_write(&path, b"new", Some(&backup)).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read(&backup).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_replacement_keeps_existing_destination() {
        let directory = test_dir("failure");
        let path = directory.join("state.json");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, b"old").unwrap();
        let missing = directory.join("missing.tmp");

        assert!(replace_file(&missing, &path, None).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(directory);
    }
}
