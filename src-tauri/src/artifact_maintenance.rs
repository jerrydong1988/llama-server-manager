use serde::Serialize;
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub(crate) const KNOWN_INSTANCE_LOG_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
pub(crate) const ORPHAN_INSTANCE_LOG_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct InstanceLogMaintenanceReport {
    pub examined_files: u64,
    pub removed_files: u64,
    pub removed_bytes: u64,
    pub skipped_active: u64,
}

pub(crate) fn metadata_is_link_like(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

fn log_root(config_dir: &Path) -> PathBuf {
    config_dir.join("logs")
}

fn validate_log_root(root: &Path) -> Result<Option<PathBuf>, String> {
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Unable to inspect log directory {}: {error}",
                root.display()
            ))
        }
    };
    if metadata_is_link_like(&metadata) || !metadata.is_dir() {
        return Err(format!(
            "Refusing unsafe managed log directory: {}",
            root.display()
        ));
    }
    std::fs::canonicalize(root).map(Some).map_err(|error| {
        format!(
            "Unable to resolve log directory {}: {error}",
            root.display()
        )
    })
}

fn validated_log_file(config_dir: &Path, instance_id: &str) -> Result<Option<PathBuf>, String> {
    crate::commands::server::validate_instance_id(instance_id)
        .map_err(|error| error.to_string())?;
    let root = log_root(config_dir);
    let Some(canonical_root) = validate_log_root(&root)? else {
        return Ok(None);
    };
    let path = root.join(format!("{instance_id}.log"));
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Unable to inspect instance log {}: {error}",
                path.display()
            ))
        }
    };
    if metadata_is_link_like(&metadata) || !metadata.is_file() {
        return Err(format!(
            "Refusing unsafe managed instance log: {}",
            path.display()
        ));
    }
    let canonical = std::fs::canonicalize(&path)
        .map_err(|error| format!("Unable to resolve instance log {}: {error}", path.display()))?;
    if canonical.parent() != Some(canonical_root.as_path()) {
        return Err(format!(
            "Instance log resolves outside its managed directory: {}",
            path.display()
        ));
    }
    Ok(Some(canonical))
}

pub(crate) fn remove_instance_log(config_dir: &Path, instance_id: &str) -> Result<u64, String> {
    let Some(path) = validated_log_file(config_dir, instance_id)? else {
        return Ok(0);
    };
    let bytes = std::fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    std::fs::remove_file(&path)
        .map_err(|error| format!("Unable to remove instance log {}: {error}", path.display()))?;
    Ok(bytes)
}

pub(crate) fn prune_instance_logs(
    config_dir: &Path,
    known_instance_ids: &HashSet<String>,
    active_instance_ids: &HashSet<String>,
) -> Result<InstanceLogMaintenanceReport, String> {
    prune_instance_logs_with_policy(
        config_dir,
        known_instance_ids,
        active_instance_ids,
        SystemTime::now(),
        KNOWN_INSTANCE_LOG_RETENTION,
        ORPHAN_INSTANCE_LOG_RETENTION,
    )
}

fn prune_instance_logs_with_policy(
    config_dir: &Path,
    known_instance_ids: &HashSet<String>,
    active_instance_ids: &HashSet<String>,
    now: SystemTime,
    known_retention: Duration,
    orphan_retention: Duration,
) -> Result<InstanceLogMaintenanceReport, String> {
    let root = log_root(config_dir);
    if validate_log_root(&root)?.is_none() {
        return Ok(InstanceLogMaintenanceReport::default());
    }
    let mut report = InstanceLogMaintenanceReport::default();
    for entry in std::fs::read_dir(&root)
        .map_err(|error| format!("Unable to read log directory {}: {error}", root.display()))?
    {
        let entry =
            entry.map_err(|error| format!("Unable to inspect managed log entry: {error}"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(instance_id) = name.strip_suffix(".log") else {
            continue;
        };
        if crate::commands::server::validate_instance_id(instance_id).is_err() {
            continue;
        }
        report.examined_files = report.examined_files.saturating_add(1);
        if active_instance_ids.contains(instance_id) {
            report.skipped_active = report.skipped_active.saturating_add(1);
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
            format!(
                "Unable to inspect managed log {}: {error}",
                entry.path().display()
            )
        })?;
        if metadata_is_link_like(&metadata) || !metadata.is_file() {
            continue;
        }
        let retention = if known_instance_ids.contains(instance_id) {
            known_retention
        } else {
            orphan_retention
        };
        let old_enough = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= retention);
        if !old_enough {
            continue;
        }
        let bytes = remove_instance_log(config_dir, instance_id)?;
        report.removed_files = report.removed_files.saturating_add(1);
        report.removed_bytes = report.removed_bytes.saturating_add(bytes);
    }
    Ok(report)
}

/// Keeps the recent tail of a fixed manager-owned diagnostic log. Callers must
/// provide an application-private path; links and reparse points are rejected.
pub(crate) fn compact_regular_file_tail(
    path: &Path,
    max_bytes: u64,
    retained_bytes: u64,
) -> Result<u64, String> {
    if retained_bytes >= max_bytes {
        return Err("retained log size must be smaller than maximum log size".into());
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(format!(
                "Unable to inspect diagnostic log {}: {error}",
                path.display()
            ))
        }
    };
    if metadata_is_link_like(&metadata) || !metadata.is_file() {
        return Err(format!(
            "Refusing unsafe diagnostic log path: {}",
            path.display()
        ));
    }
    let original_bytes = metadata.len();
    if original_bytes <= max_bytes {
        return Ok(0);
    }
    let marker = b"[runtime-service log compacted; recent tail retained]\n";
    let keep = retained_bytes
        .saturating_sub(marker.len() as u64)
        .min(original_bytes);
    let mut reader = std::fs::File::open(path)
        .map_err(|error| format!("Unable to open diagnostic log {}: {error}", path.display()))?;
    reader
        .seek(SeekFrom::Start(original_bytes.saturating_sub(keep)))
        .map_err(|error| format!("Unable to seek diagnostic log {}: {error}", path.display()))?;
    let mut tail = vec![0_u8; keep as usize];
    reader.read_exact(&mut tail).map_err(|error| {
        format!(
            "Unable to read diagnostic log tail {}: {error}",
            path.display()
        )
    })?;
    drop(reader);
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|error| {
            format!(
                "Unable to truncate diagnostic log {}: {error}",
                path.display()
            )
        })?;
    let mut writer = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|error| {
            format!(
                "Unable to reopen diagnostic log {}: {error}",
                path.display()
            )
        })?;
    writer
        .write_all(marker)
        .and_then(|_| writer.write_all(&tail))
        .and_then(|_| writer.flush())
        .map_err(|error| {
            format!(
                "Unable to compact diagnostic log {}: {error}",
                path.display()
            )
        })?;
    Ok(original_bytes.saturating_sub(marker.len() as u64 + keep))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "lsm-artifact-maintenance-{name}-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn instance_log_removal_is_exact_and_idempotent() {
        let dir = TestDirectory::new("exact-log");
        let logs = dir.0.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("instance-a.log"), b"remove").unwrap();
        std::fs::write(logs.join("instance-ab.log"), b"keep").unwrap();

        assert_eq!(remove_instance_log(&dir.0, "instance-a").unwrap(), 6);
        assert_eq!(remove_instance_log(&dir.0, "instance-a").unwrap(), 0);
        assert_eq!(
            std::fs::read(logs.join("instance-ab.log")).unwrap(),
            b"keep"
        );
        assert!(remove_instance_log(&dir.0, "../outside").is_err());
    }

    #[test]
    fn pruning_protects_active_logs_and_uses_shorter_orphan_retention() {
        let dir = TestDirectory::new("log-retention");
        let logs = dir.0.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        for id in ["known", "orphan", "active"] {
            std::fs::write(logs.join(format!("{id}.log")), id.as_bytes()).unwrap();
        }
        let known = HashSet::from(["known".to_string(), "active".to_string()]);
        let active = HashSet::from(["active".to_string()]);
        let report = prune_instance_logs_with_policy(
            &dir.0,
            &known,
            &active,
            SystemTime::now() + Duration::from_secs(1),
            Duration::ZERO,
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(report.examined_files, 3);
        assert_eq!(report.removed_files, 2);
        assert_eq!(report.skipped_active, 1);
        assert!(logs.join("active.log").exists());
        assert!(!logs.join("known.log").exists());
        assert!(!logs.join("orphan.log").exists());
    }

    #[test]
    fn diagnostic_log_compaction_retains_the_recent_tail() {
        let dir = TestDirectory::new("runtime-log");
        let path = dir.0.join("runtime-service.log");
        std::fs::write(&path, [b'a'; 192]).unwrap();
        let mut active_append_handle = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();

        let removed = compact_regular_file_tail(&path, 96, 80).unwrap();
        active_append_handle
            .write_all(b"live-after-compact\n")
            .unwrap();
        active_append_handle.flush().unwrap();
        let content = std::fs::read(&path).unwrap();
        assert!(removed > 0);
        assert!(content.len() <= 99);
        assert!(content.ends_with(b"live-after-compact\n"));
        assert_eq!(compact_regular_file_tail(&path, 128, 80).unwrap(), 0);
        assert!(compact_regular_file_tail(&path, 32, 32).is_err());
    }
}
