//! Opt-in dynamic n-gram generations. Native b11769 writes only a private copy;
//! its silent close failures mean a valid candidate is NOT proof of a full flush.
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Seek},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

const MAX_BYTES: u64 = 256 * 1024 * 1024;
static RUNS: OnceLock<Mutex<HashMap<u32, Run>>> = OnceLock::new();
fn runs() -> &'static Mutex<HashMap<u32, Run>> {
    RUNS.get_or_init(Default::default)
}

fn copy_candidate(source: &Path, destination: &Path) -> Result<(), String> {
    let mut created = false;
    let result = (|| {
        let input = File::open(source).map_err(|e| e.to_string())?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)
            .map_err(|e| e.to_string())?;
        created = true;
        crate::persistence::enforce_private_file(destination)?;
        let bytes = std::io::copy(&mut input.take(MAX_BYTES + 1), &mut output)
            .map_err(|e| e.to_string())?;
        if bytes > MAX_BYTES {
            return Err("dynamic cache grew beyond allocation bound".into());
        }
        output.sync_all().map_err(|e| e.to_string())
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(destination);
    }
    result
}

struct Candidate {
    path: PathBuf,
    owned: bool,
}
impl Candidate {
    fn keep(mut self) {
        self.owned = false;
    }
}
impl Drop for Candidate {
    fn drop(&mut self) {
        if self.owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Generation {
    id: String,
    sha256: String,
    bytes: u64,
}

pub(crate) struct Run {
    root: PathBuf,
    work: PathBuf,
    before: Option<String>,
    _lock: File,
}

/// Bounded validation of the legacy four-token n-gram record format.
fn validate(path: &Path) -> Result<(u64, String), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    if len == 0 || len > MAX_BYTES {
        return Err("empty or oversized dynamic cache".into());
    }
    let mut file = BufReader::with_capacity(64 * 1024, file);
    let mut consumed = 0u64;
    while consumed < len {
        let mut header = [0u8; 20];
        file.read_exact(&mut header)
            .map_err(|_| "truncated n-gram header")?;
        let count = i32::from_le_bytes(header[16..20].try_into().unwrap());
        if count <= 0 || (count as u64) > (len.saturating_sub(consumed + 20)) / 8 {
            return Err("invalid n-gram count".into());
        }
        for token in header[..16].chunks_exact(4) {
            if i32::from_le_bytes(token.try_into().unwrap()) < -1 {
                return Err("invalid prefix token".into());
            }
        }
        for _ in 0..count {
            let mut pair = [0u8; 8];
            file.read_exact(&mut pair)
                .map_err(|_| "truncated token pair")?;
            if i32::from_le_bytes(pair[..4].try_into().unwrap()) < 0
                || i32::from_le_bytes(pair[4..].try_into().unwrap()) <= 0
            {
                return Err("invalid token/count".into());
            }
        }
        consumed += 20 + count as u64 * 8;
    }
    // Hash the same open file, then copy/verify again before publication.
    file.rewind().map_err(|e| e.to_string())?;
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let hashed =
        std::io::copy(&mut file.take(MAX_BYTES + 1), &mut hash).map_err(|e| e.to_string())?;
    if hashed != len {
        return Err("dynamic cache changed during validation".into());
    }
    Ok((len, format!("{:x}", hash.finalize())))
}

fn generation_path(root: &Path, generation: &Generation) -> Option<PathBuf> {
    uuid::Uuid::parse_str(&generation.id).ok()?;
    Some(root.join(format!("{}.bin", generation.id)))
}

fn valid_generation(root: &Path, name: &str) -> Option<PathBuf> {
    let mut bytes = Vec::new();
    File::open(root.join(name))
        .ok()?
        .take(4097)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    let generation: Generation = serde_json::from_slice(&bytes).ok()?;
    let path = generation_path(root, &generation)?;
    let (size, hash) = validate(&path).ok()?;
    (size == generation.bytes && hash == generation.sha256).then_some(path)
}

impl Run {
    fn prepare(root: PathBuf, seed: &Path) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        crate::persistence::enforce_private_file(&root.join("owner.lock"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("owner.lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock_exclusive()
            .map_err(|_| "dynamic cache is owned by another running instance")?;
        let work = root.join(format!("work-{}.bin", uuid::Uuid::new_v4()));
        let cleanup = Candidate {
            path: work.clone(),
            owned: true,
        };
        let source = valid_generation(&root, "latest.json")
            .or_else(|| valid_generation(&root, "previous.json"));
        // Reject structurally malformed input before llama.cpp's assertion-based parser.
        let source = source.or_else(|| seed.is_file().then(|| seed.to_owned()));
        let before = if let Some(source) = source {
            let (_, hash) = validate(&source)?;
            copy_candidate(&source, &work)?;
            if validate(&work)?.1 != hash {
                let _ = fs::remove_file(&work);
                return Err("dynamic seed changed during copy".into());
            }
            Some(hash)
        } else {
            File::create(&work).map_err(|e| e.to_string())?;
            None
        };
        cleanup.keep();
        Ok(Self {
            root,
            work,
            before,
            _lock: lock,
        })
    }

    fn commit(&self) -> Result<&'static str, String> {
        let (bytes, hash) = validate(&self.work)?;
        if self.before.as_deref() == Some(&hash) {
            return Ok("unchanged_flush_unverified");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let path = self.root.join(format!("{id}.bin"));
        copy_candidate(&self.work, &path)?;
        let cleanup = Candidate {
            path: path.clone(),
            owned: true,
        };
        if validate(&path)? != (bytes, hash.clone()) {
            return Err("candidate changed during commit".into());
        }
        let manifest = serde_json::to_vec(&Generation {
            id,
            sha256: hash,
            bytes,
        })
        .map_err(|e| e.to_string())?;
        // Do not rotate a corrupt newest generation over a still-valid fallback.
        if valid_generation(&self.root, "latest.json").is_some() {
            let old = fs::read(self.root.join("latest.json")).map_err(|e| e.to_string())?;
            crate::persistence::atomic_write(&self.root.join("previous.json"), &old, None)?;
        }
        crate::persistence::atomic_write(&self.root.join("latest.json"), &manifest, None)?;
        // From this point the pointer owns the payload, even if subsequent GC fails.
        cleanup.keep();
        // Keep both committed generations. Garbage collection cannot delete the rollback.
        let retained = [
            valid_generation(&self.root, "latest.json"),
            valid_generation(&self.root, "previous.json"),
        ];
        for entry in fs::read_dir(&self.root)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "bin")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
                && !retained.iter().flatten().any(|keep| keep == &path)
            {
                let _ = fs::remove_file(path);
            }
        }
        Ok("candidate_committed_flush_unverified")
    }

    fn finish(self, clean_exit: bool) {
        let result = if clean_exit {
            self.commit().map(str::to_owned)
        } else {
            Err("forced or abnormal exit; previous generation retained".into())
        };
        let record = serde_json::json!({"clean_exit": clean_exit, "outcome": result.as_ref().ok(), "error": result.as_ref().err(), "full_flush_verified": false});
        let _ = crate::persistence::atomic_write(
            &self.root.join("last-result.json"),
            &serde_json::to_vec_pretty(&record).unwrap(),
            None,
        );
        eprintln!("dynamic cache: {record}");
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Ok(meta) = fs::metadata(&self.work) {
            if meta.permissions().readonly() {
                let mut permissions = meta.permissions();
                // Windows-only: clear FILE_ATTRIBUTE_READONLY on our disposable
                // working file. This code is not compiled on Unix.
                #[allow(clippy::permissions_set_readonly_false)]
                permissions.set_readonly(false);
                let _ = fs::set_permissions(&self.work, permissions);
            }
        }
        let _ = fs::remove_file(&self.work);
    }
}

pub(crate) fn prepare(
    instance: &str,
    config: &crate::models::InstanceConfig,
    args: &mut [String],
) -> Result<Option<Run>, String> {
    if !config.lookup_cache_managed
        || !crate::speculative::parse_speculative_types(&config.spec_type)
            .iter()
            .any(|s| s == "ngram-cache")
    {
        return Ok(None);
    }
    if !config.launch_mode.eq_ignore_ascii_case("managed")
        || config
            .custom_args
            .iter()
            .any(|argument| !argument.trim().is_empty())
    {
        return Err(
            "managed dynamic cache requires structured launch arguments without custom overrides"
                .into(),
        );
    }
    if !config.slots_enabled {
        return Err("managed dynamic cache requires slots for exit draining".into());
    }
    if config.lookup_cache_dynamic.trim().is_empty() {
        return Err("managed dynamic cache requires ngram-cache and an external seed path (it may not exist yet)".into());
    }
    crate::commands::server::validate_instance_id(instance).map_err(|e| e.to_string())?;
    let indices = args
        .iter()
        .enumerate()
        .filter(|(_, s)| s.as_str() == "-lcd" || s.as_str() == "--lookup-cache-dynamic")
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if indices.len() != 1
        || indices[0] + 1 >= args.len()
        || args
            .iter()
            .any(|s| s.starts_with("--lookup-cache-dynamic=") || s.starts_with("-lcd="))
    {
        return Err("ambiguous managed dynamic cache arguments".into());
    }
    let store = crate::checkpoint::CheckpointStore::new(crate::utils::get_data_dir());
    let model = store
        .model_artifact_sha256(Path::new(&config.model_path))
        .map_err(|e| e.to_string())?;
    let engine = store
        .engine_artifact_sha256(Path::new(&args[0]))
        .map_err(|e| e.to_string())?;
    if !cfg!(windows) || engine != crate::commands::engine_capabilities::B11769_ARTIFACTS {
        return Err("managed dynamic cache format is not qualified for this engine bundle".into());
    }
    // Full model identity includes tokenizer data. Legacy format is explicitly namespaced.
    use sha2::{Digest, Sha256};
    let namespace = format!(
        "{:x}",
        Sha256::digest(format!(
            "ngram-legacy4-v1:{model}:{engine}:{}",
            config.lookup_cache_dynamic
        ))
    );
    let root = crate::utils::get_data_dir()
        .join("lookup-cache")
        .join(instance)
        .join(namespace);
    let run = Run::prepare(root, Path::new(&config.lookup_cache_dynamic))?;
    args[indices[0] + 1] = run.work.to_string_lossy().into_owned();
    Ok(Some(run))
}

pub(crate) struct Registration {
    pid: u32,
    active: bool,
}
impl Registration {
    pub fn handoff(mut self) {
        self.active = false;
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        if self.active {
            finish(self.pid, false);
        }
    }
}
pub(crate) fn register(pid: u32, run: Option<Run>) -> Registration {
    if let Some(run) = run {
        runs().lock().unwrap().insert(pid, run);
    }
    Registration { pid, active: true }
}
pub(crate) fn finish(pid: u32, clean_exit: bool) {
    // Hold the registry lock through publication so a concurrent stop waits for the
    // monitor's commit, rather than restarting while the generation lock is held.
    if let Some(run) = runs().lock().unwrap().remove(&pid) {
        run.finish(clean_exit);
    }
}

#[cfg(windows)]
pub(crate) fn await_finish(pid: u32) {
    // The monitor finalizes BEFORE taking any supervisor lifecycle lock.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while runs().lock().unwrap().contains_key(&pid) && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    // Monitor setup failure must not leave a stale writer lock or promote data.
    finish(pid, false);
}

pub(crate) fn status(instance: &str) -> serde_json::Value {
    if crate::commands::server::validate_instance_id(instance).is_err() {
        return serde_json::Value::Null;
    }
    let root = crate::utils::get_data_dir()
        .join("lookup-cache")
        .join(instance);
    let results = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let mut data = Vec::new();
            File::open(entry.path().join("last-result.json"))
                .ok()?
                .take(4096)
                .read_to_end(&mut data)
                .ok()?;
            serde_json::from_slice::<serde_json::Value>(&data).ok()
        })
        .collect::<Vec<_>>();
    serde_json::json!({"generations": results, "full_flush_verified": false})
}

pub(crate) fn drain(running: &crate::models::RunningInstance, active: impl Fn() -> u32) {
    use crate::checkpoint::SlotBackend;
    let Some(config) = running
        .launch_config
        .as_ref()
        .filter(|c| c.lookup_cache_managed)
    else {
        return;
    };
    let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let client = crate::checkpoint::LlamaSlotClient::new(config, std::time::Duration::from_secs(2));
    while std::time::Instant::now() < until {
        if active() == 0
            && client.as_ref().is_ok_and(|c| {
                c.slots()
                    .is_ok_and(|slots| slots.iter().all(|s| !s.is_processing))
            })
        {
            return;
        }
        if !crate::commands::server::running_instance_matches_live_process(running) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    eprintln!("dynamic cache drain timed out; incomplete requests may be cancelled on exit");
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(count: i32) -> Vec<u8> {
        [1i32, 2, -1, -1, 1, 3, count]
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect()
    }
    #[test]
    fn inactive_policy_is_dormant_and_manual_identity_overrides_are_rejected() {
        let mut config = crate::models::InstanceConfig {
            lookup_cache_managed: true,
            ..Default::default()
        };
        assert!(prepare("instance", &config, &mut []).unwrap().is_none());
        config.spec_type = "ngram-cache".into();
        config.launch_mode = "manual".into();
        assert!(prepare("instance", &config, &mut [])
            .err()
            .unwrap()
            .contains("structured"));
        config.launch_mode = "managed".into();
        config.custom_args = vec!["--model another.gguf".into()];
        assert!(prepare("instance", &config, &mut [])
            .err()
            .unwrap()
            .contains("structured"));
    }
    #[test]
    fn generations_survive_empty_corrupt_forced_and_concurrent_writers() {
        let root = std::env::temp_dir().join(format!("lookup-test-{}", uuid::Uuid::new_v4()));
        let seed = root.with_extension("seed");
        fs::write(&seed, record(1)).unwrap();
        let first = Run::prepare(root.clone(), &seed).unwrap();
        assert!(Run::prepare(root.clone(), &seed).is_err());
        fs::write(&first.work, record(2)).unwrap();
        first.finish(true);
        let old = fs::read(root.join("latest.json")).unwrap();
        for (data, clean) in [(vec![], true), (vec![0; 20], true), (record(3), false)] {
            let run = Run::prepare(root.clone(), &seed).unwrap();
            fs::write(&run.work, data).unwrap();
            run.finish(clean);
            assert_eq!(fs::read(root.join("latest.json")).unwrap(), old);
        }
        assert_eq!(fs::read(&seed).unwrap(), record(1));
        let run = Run::prepare(root.clone(), &seed).unwrap();
        fs::write(&run.work, record(4)).unwrap();
        run.finish(true);
        assert_eq!(fs::read(root.join("previous.json")).unwrap(), old);
        let newest = valid_generation(&root, "latest.json").unwrap();
        fs::write(newest, b"broken").unwrap();
        let run = Run::prepare(root.clone(), &seed).unwrap();
        assert_eq!(fs::read(&run.work).unwrap(), record(2));
        drop(run);
        fs::remove_dir_all(root).unwrap();
        fs::remove_file(seed).unwrap();
    }
    #[test]
    fn rejects_truncation_and_huge_counts_before_allocation() {
        let path = std::env::temp_dir().join(format!("lookup-{}", uuid::Uuid::new_v4()));
        let valid = record(1);
        for end in 0..valid.len() {
            fs::write(&path, &valid[..end]).unwrap();
            assert!(validate(&path).is_err());
        }
        let mut invalid = valid;
        invalid[16..20].copy_from_slice(&i32::MAX.to_le_bytes());
        fs::write(&path, invalid).unwrap();
        assert!(validate(&path).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn failed_publication_preserves_the_old_pointer_and_payload() {
        let root = std::env::temp_dir().join(format!("lookup-test-{}", uuid::Uuid::new_v4()));
        let run = Run::prepare(root.clone(), &root.join("absent-seed")).unwrap();
        fs::write(&run.work, record(1)).unwrap();
        run.finish(true);
        let pointer = fs::read(root.join("latest.json")).unwrap();
        let old = valid_generation(&root, "latest.json").unwrap();
        fs::create_dir(root.join("previous.json")).unwrap(); // deterministic replacement failure
        let run = Run::prepare(root.clone(), &root.join("absent-seed")).unwrap();
        fs::write(&run.work, record(2)).unwrap();
        assert!(run.commit().is_err());
        drop(run);
        assert_eq!(fs::read(root.join("latest.json")).unwrap(), pointer);
        assert_eq!(fs::read(old).unwrap(), record(1));
        assert_eq!(
            fs::read_dir(&root)
                .unwrap()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|s| s == "bin"))
                .count(),
            1
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copy_failure_never_deletes_an_existing_destination() {
        let root = std::env::temp_dir().join(format!("lookup-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("source");
        let destination = root.join("existing");
        fs::write(&source, record(1)).unwrap();
        fs::write(&destination, b"preserve").unwrap();
        assert!(copy_candidate(&source, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"preserve");
        fs::remove_dir_all(root).unwrap();
    }
}
