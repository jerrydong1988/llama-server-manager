//! Private ownership comes from native file creation, never renderer queue status.
use super::*;
type Ownership = HashMap<String, HashMap<String, Vec<String>>>;
static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
fn ledger_path(state: &AppState) -> PathBuf {
    state
        .config_dir
        .lock()
        .unwrap()
        .join("download-ownership-v2.json")
}
fn read(path: &Path) -> Result<Ownership, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("invalid download ownership: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(e.to_string()),
    }
}
pub(super) fn owns(state: &AppState, task: &str, path: &Path) -> Result<bool, String> {
    let _lock = LOCK.lock().map_err(|e| e.to_string())?;
    let records = read(&ledger_path(state))?;
    owns_record(&records, task, path)
}
fn owns_record(records: &Ownership, task: &str, path: &Path) -> Result<bool, String> {
    let expected = records
        .get(task)
        .and_then(|paths| paths.get(&path_identity_key(path)));
    let Some(expected) = expected else {
        return Ok(false);
    };
    let Some(parent) = path.parent() else {
        return Ok(false);
    };
    let actual = DownloadDirectory::open(parent, false).and_then(|dir| dir.identity(path));
    Ok(actual.is_ok_and(|identity| expected.contains(&identity)))
}

pub(super) fn record(
    state: &AppState,
    task: &str,
    paths: &[(&Path, String)],
) -> Result<(), String> {
    let _lock = LOCK.lock().map_err(|e| e.to_string())?;
    let path = ledger_path(state);
    let mut records = read(&path)?;
    for (file, identity) in paths {
        records
            .entry(task.to_string())
            .or_default()
            .insert(path_identity_key(file), vec![identity.clone()]);
    }
    crate::persistence::atomic_write(
        &path,
        &serde_json::to_vec(&records).map_err(|e| e.to_string())?,
        None,
    )
}
pub(super) fn check_references(state: &AppState, path: &Path) -> Result<(), String> {
    let references = super::super::scanner::instances_referencing_models(
        &state.instances.lock().unwrap(),
        &state.running.lock().unwrap(),
        &[path.to_path_buf()],
    )?;
    if references.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "模型仍被实例引用，不能覆盖或删除: {}",
            references.join(", ")
        ))
    }
}
pub(super) fn claim(
    state: &AppState,
    task: &str,
    dir: &DownloadDirectory,
    final_path: &Path,
    temp: &Path,
    metadata: &Path,
) -> Result<(), String> {
    check_references(state, final_path)?;
    if dir.metadata(final_path).is_ok() && !owns_in_directory(state, task, dir, final_path)? {
        return Err("下载目标已存在且不属于此任务，拒绝覆盖".into());
    }
    let mut created = Vec::new();
    let result = (|| {
        for path in [temp, metadata] {
            match dir.open_file(path, false, true) {
                Ok(file) => {
                    let identity =
                        DownloadDirectory::file_identity(&file).map_err(|e| e.to_string())?;
                    created.push((path, identity));
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::AlreadyExists
                        && owns_in_directory(state, task, dir, path)? => {}
                Err(e) => return Err(format!("下载临时文件不属于此任务或无法创建: {e}")),
            }
        }
        record(state, task, &created)
    })();
    if result.is_err() {
        for (path, identity) in created {
            let _ = dir.remove_owned(path, &[identity]);
        }
    }
    result
}
pub(super) fn owns_in_directory(
    state: &AppState,
    task: &str,
    dir: &DownloadDirectory,
    path: &Path,
) -> Result<bool, String> {
    let _lock = LOCK.lock().map_err(|e| e.to_string())?;
    let records = read(&ledger_path(state))?;
    Ok(matches_identity(&records, task, dir, path))
}
fn matches_identity(records: &Ownership, task: &str, dir: &DownloadDirectory, path: &Path) -> bool {
    records
        .get(task)
        .and_then(|paths| paths.get(&path_identity_key(path)))
        .is_some_and(|expected| dir.identity(path).is_ok_and(|id| expected.contains(&id)))
}
// Persist the incoming inode before publication. Either side of a crash is owned;
// the next successful update compacts the record back to the live identity.
fn prepare_replace(
    records: &mut Ownership,
    task: &str,
    dir: &DownloadDirectory,
    source: &Path,
    destination: &Path,
) -> Result<(), String> {
    let mut identities = Vec::new();
    match dir.metadata(destination) {
        Ok(_) => {
            if !matches_identity(records, task, dir, destination) {
                return Err("下载目标已存在且不属于此任务，拒绝覆盖".into());
            }
            identities.push(dir.identity(destination).map_err(|e| e.to_string())?);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    identities.push(dir.identity(source).map_err(|e| e.to_string())?);
    records
        .entry(task.into())
        .or_default()
        .insert(path_identity_key(destination), identities);
    Ok(())
}
pub(super) fn replace(
    state: &AppState,
    task: &str,
    dir: &DownloadDirectory,
    source: &Path,
    destination: &Path,
) -> Result<(), String> {
    check_references(state, destination)?;
    let _lock = LOCK.lock().map_err(|e| e.to_string())?;
    let ledger = ledger_path(state);
    let mut records = read(&ledger)?;
    prepare_replace(&mut records, task, dir, source, destination)?;
    crate::persistence::atomic_write(
        &ledger,
        &serde_json::to_vec(&records).map_err(|e| e.to_string())?,
        None,
    )?;
    dir.rename(source, destination).map_err(|e| e.to_string())
}
pub(super) fn remove(
    state: &AppState,
    task: &str,
    root: &Path,
    path: &Path,
) -> std::io::Result<()> {
    let root = DownloadDirectory::open(root, false)?;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("artifact has no parent"))?;
    let dir = root.descend(parent, false)?;
    remove_in_directory(state, task, &dir, path)
}
pub(super) fn remove_in_directory(
    state: &AppState,
    task: &str,
    dir: &DownloadDirectory,
    path: &Path,
) -> std::io::Result<()> {
    let _lock = LOCK
        .lock()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let records = read(&ledger_path(state)).map_err(std::io::Error::other)?;
    if !matches_identity(&records, task, dir, path) {
        return Ok(());
    }
    check_references(state, path).map_err(std::io::Error::other)?;
    let expected = &records[task][&path_identity_key(path)];
    dir.remove_owned(path, expected)
}

pub(super) fn open_partial(
    state: &AppState,
    task: &str,
    dir: &DownloadDirectory,
    path: &Path,
    append: bool,
) -> Result<std::fs::File, String> {
    let _lock = LOCK.lock().map_err(|e| e.to_string())?;
    let ledger = ledger_path(state);
    let mut records = read(&ledger)?;
    match dir.metadata(path) {
        Ok(_) => {
            let expected = records
                .get(task)
                .and_then(|files| files.get(&path_identity_key(path)))
                .ok_or("下载临时文件没有原生所有权记录")?;
            dir.open_owned_file(path, append, expected)
                .map_err(|e| e.to_string())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let file = dir
                .open_file(path, false, true)
                .map_err(|e| e.to_string())?;
            records.entry(task.into()).or_default().insert(
                path_identity_key(path),
                vec![DownloadDirectory::file_identity(&file).map_err(|e| e.to_string())?],
            );
            crate::persistence::atomic_write(
                &ledger,
                &serde_json::to_vec(&records).map_err(|e| e.to_string())?,
                None,
            )?;
            Ok(file)
        }
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_publication_recovers_before_and_after_rename_and_rejects_leaf_swap() {
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("download-transaction-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let dir = DownloadDirectory::open(&base, false).unwrap();
        let target = base.join("model.part.json");
        dir.atomic_write(&target, b"old metadata").unwrap();
        let mut records = Ownership::from([(
            "task".into(),
            HashMap::from([(
                path_identity_key(&target),
                vec![dir.identity(&target).unwrap()],
            )]),
        )]);
        let staged = dir.stage_write(b"new metadata").unwrap();
        prepare_replace(&mut records, "task", &dir, &staged, &target).unwrap();
        let records: Ownership =
            serde_json::from_slice(&serde_json::to_vec(&records).unwrap()).unwrap();
        assert!(matches_identity(&records, "task", &dir, &target));
        dir.rename(&staged, &target).unwrap();
        assert!(matches_identity(&records, "task", &dir, &target));
        let expected = records["task"][&path_identity_key(&target)].clone();
        dir.atomic_write(&target, b"unowned replacement").unwrap();
        assert!(dir.remove_owned(&target, &expected).is_err());
        assert_eq!(dir.read(&target).unwrap(), "unowned replacement");
        drop(dir);
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn queue_identity_cannot_grant_ownership_or_follow_replaced_files() {
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("download-owner-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let dir = DownloadDirectory::open(&base, false).unwrap();
        let file = base.join("model.gguf");
        dir.atomic_write(&file, b"owned-model").unwrap();
        let mut ledger = Ownership::new();
        assert!(!owns_record(&ledger, "forged-task", &file).unwrap());
        ledger.insert(
            "native-task".into(),
            HashMap::from([(path_identity_key(&file), vec![dir.identity(&file).unwrap()])]),
        );
        assert!(owns_record(&ledger, "native-task", &file).unwrap());
        assert!(!owns_record(&ledger, "forged-task", &file).unwrap());
        dir.atomic_write(&file, b"replacement-model").unwrap();
        assert!(!owns_record(&ledger, "native-task", &file).unwrap());
        drop(dir);
        std::fs::remove_dir_all(base).unwrap();
    }
}
