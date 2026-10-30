//! Local admission evidence, separate from immutable saved-cache compatibility.
//! A successful bounded self-test is experimental evidence for this exact tuple;
//! it does not certify an architecture family, arbitrary contexts, or backends.
use crate::checkpoint::{CheckpointFingerprint, CheckpointStore};
use crate::models::{EngineCapabilities, InstanceConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const SCHEMA: u32 = 2;
static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn cancel() {
    CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

pub(crate) fn candidate(config: &InstanceConfig, caps: &EngineCapabilities) -> bool {
    let kinds = crate::speculative::parse_speculative_types(&config.spec_type);
    let kv = if config.cache_type_k.is_empty() {
        "f16"
    } else {
        &config.cache_type_k
    };
    let emits = |field: &str| {
        config.explicit_overrides.as_ref().map_or(true, |keys| {
            keys.iter().any(|key| {
                key == field
                    || matches!(
                        (field, key.as_str()),
                        ("ctx_size", "ctx_size_auto")
                            | ("gpu_layers", "gpu_layers_auto")
                            | ("fit_mode", "fit")
                            | ("kv_unified", "kv_unified_mode")
                    )
            })
        })
    };
    cfg!(windows)
        && caps.context_checkpoint_persistence
        && crate::commands::engine_capabilities::hardware_scope(config)
        && matches!(config.parallel, 1 | 2 | 4)
        && config.ctx_size % config.parallel as u32 == 0
        && (1024..=32768).contains(&(config.ctx_size / config.parallel as u32))
        && (1..=32).contains(&config.ctx_checkpoints)
        && (1..=8).contains(&config.draft_tokens)
        && config.kv_unified
        && matches!(config.kv_unified_mode.as_str(), "" | "on")
        && matches!(kv, "f16" | "q8_0")
        && [
            &config.cache_type_v,
            &config.cache_type_draft_k,
            &config.cache_type_draft_v,
        ]
        .iter()
        .all(|value| {
            (if value.is_empty() {
                "f16"
            } else {
                value.as_str()
            }) == kv
        })
        && kinds.iter().all(|kind| {
            matches!(
                kind.as_str(),
                "draft-mtp" | "draft-simple" | "draft-dflash" | "ngram-mod"
            ) && caps.speculative_types.contains(kind)
        })
        && kinds
            .iter()
            .filter(|kind| kind.starts_with("draft-"))
            .count()
            <= 1
        && [
            "ctx_size",
            "gpu_layers",
            "flash_attn",
            "fit_mode",
            "parallel",
            "ctx_checkpoints",
            "kv_unified",
        ]
        .iter()
        .all(|key| emits(key))
        && (config.parallel == 1 || emits("cache_idle_slots"))
        && (!kinds.iter().any(|kind| kind.starts_with("draft-"))
            || [
                "draft_model_path",
                "draft_gpu_layers",
                "draft_tokens",
                "spec_type",
            ]
            .iter()
            .all(|key| emits(key)))
        && [
            ("cache_type_k", &config.cache_type_k),
            ("cache_type_v", &config.cache_type_v),
            ("cache_type_draft_k", &config.cache_type_draft_k),
            ("cache_type_draft_v", &config.cache_type_draft_v),
        ]
        .iter()
        .all(|(key, value)| value.is_empty() || emits(key))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    schema: u32,
    fingerprint: CheckpointFingerprint,
    hardware: String,
    completed_at: String,
    old_process_exited: bool,
    processes: Vec<Value>,
    effective_slots: Value,
    slots: Vec<Value>,
    evidence_directory: String,
}

fn receipt_path(
    store: &CheckpointStore,
    fingerprint: &CheckpointFingerprint,
) -> Result<PathBuf, String> {
    fingerprint.validate().map_err(|error| error.to_string())?;
    Ok(store
        .qualification_root()
        .join(format!("{}.json", fingerprint.digest)))
}

fn matches(
    receipt: &Receipt,
    fingerprint: &CheckpointFingerprint,
    hardware: &str,
    slots: u32,
) -> bool {
    receipt.schema == SCHEMA
        && receipt.fingerprint == *fingerprint
        && receipt.hardware == hardware
        && receipt.old_process_exited
        && receipt.processes.len() >= 4
        && receipt
            .processes
            .iter()
            .all(|process| process["exited"] == true)
        && receipt.slots.len() == slots as usize
        && receipt
            .slots
            .iter()
            .enumerate()
            .all(|(id, slot)| slot["id"] == id && slot["passed"] == true)
}

pub(crate) fn qualified(
    store: &CheckpointStore,
    fingerprint: &CheckpointFingerprint,
    slots: u32,
) -> bool {
    let Ok(path) = receipt_path(store, fingerprint) else {
        return false;
    };
    if path.with_extension("blocked.json").exists() {
        return false;
    }
    if !fs::metadata(&path).is_ok_and(|meta| meta.len() <= 1024 * 1024) {
        return false;
    }
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let Ok(receipt) = serde_json::from_slice::<Receipt>(&bytes) else {
        return false;
    };
    hardware_identity().is_ok_and(|hardware| matches(&receipt, fingerprint, &hardware, slots))
}

fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

fn hardware_identity() -> Result<String, String> {
    if !cfg!(windows) {
        return Err("local checkpoint qualification currently requires Windows".into());
    }
    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; (Get-ItemProperty 'HKLM:\\SOFTWARE\\Microsoft\\Cryptography').MachineGuid; @(Get-CimInstance Win32_VideoController | Sort-Object PNPDeviceID | Select-Object PNPDeviceID,DriverVersion,Name) | ConvertTo-Json -Compress"]);
    hidden(&mut command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("GPU/driver identity probe timed out".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let result = child.wait_with_output().map_err(|e| e.to_string())?;
    if !result.status.success() || result.stdout.is_empty() || result.stdout.len() > 65536 {
        return Err("GPU/driver identity is unavailable".into());
    }
    let environment = std::env::vars()
        .filter(|(key, _)| {
            ["GGML_", "HIP_", "ROCR_", "CUDA_", "LLAMA_ARG_"]
                .iter()
                .any(|prefix| key.starts_with(prefix))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut digest = Sha256::new();
    digest.update(result.stdout);
    digest.update(serde_json::to_vec(&environment).map_err(|e| e.to_string())?);
    Ok(format!("{:x}", digest.finalize()))
}

pub(crate) struct Lease;
pub(crate) fn reserve() -> Result<Lease, String> {
    use std::sync::atomic::Ordering;
    RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| "another checkpoint validation is running")?;
    CANCEL.store(false, Ordering::SeqCst);
    Ok(Lease)
}
impl Drop for Lease {
    fn drop(&mut self) {
        RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

struct EngineProcess {
    child: Child,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}
impl EngineProcess {
    fn spawn(argv: &[String], directory: &Path, label: &str) -> Result<Self, String> {
        let mut command = crate::process_environment::external_command(&argv[0]);
        command.args(&argv[1..]).current_dir(
            Path::new(&argv[0])
                .parent()
                .ok_or("engine directory missing")?,
        );
        hidden(&mut command);
        let log =
            fs::File::create(directory.join(format!("{label}.log"))).map_err(|e| e.to_string())?;
        command
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log);
        let child = command.spawn().map_err(|e| e.to_string())?;
        #[allow(unused_mut)] // The Windows job is assigned after creating the owned process.
        let mut process = Self {
            child,
            #[cfg(windows)]
            job: std::ptr::null_mut(),
        };
        #[cfg(windows)]
        unsafe {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::JobObjects::*;
            process.job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if process.job.is_null() {
                return Err("cannot create isolated validation process job".into());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                process.job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as _,
            ) == 0
                || AssignProcessToJobObject(process.job, process.child.as_raw_handle()) == 0
            {
                return Err("cannot contain isolated validation process".into());
            }
        }
        Ok(process)
    }
    fn stop(&mut self) -> Result<(), String> {
        if self.child.try_wait().map_err(|e| e.to_string())?.is_none() {
            self.child.kill().map_err(|e| e.to_string())?;
        }
        self.child.wait().map_err(|e| e.to_string())?;
        Ok(())
    }
}
impl Drop for EngineProcess {
    fn drop(&mut self) {
        let _ = self.stop();
        #[cfg(windows)]
        unsafe {
            if !self.job.is_null() {
                windows_sys::Win32::Foundation::CloseHandle(self.job);
            }
        }
    }
}

struct Probe {
    client: reqwest::blocking::Client,
    base: String,
    key: String,
    deadline: Instant,
}
impl Probe {
    fn check(&self) -> Result<(), String> {
        if CANCEL.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("checkpoint validation cancelled".into());
        }
        if Instant::now() >= self.deadline {
            return Err("checkpoint validation exceeded 20 minutes".into());
        }
        Ok(())
    }
    fn request(&self, endpoint: &str, body: Option<Value>) -> Result<Value, String> {
        self.check()?;
        let url = format!("{}{endpoint}", self.base);
        let request = if let Some(body) = body {
            self.client.post(url).json(&body)
        } else {
            self.client.get(url)
        };
        let response = request
            .bearer_auth(&self.key)
            .header("Connection", "close")
            .timeout(
                self.deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(180)),
            )
            .send()
            .map_err(|e| e.to_string())?;
        let status = response.status();
        let result = response.json::<Value>().map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("validation {endpoint}: HTTP {status}: {result}"));
        }
        Ok(result)
    }
    fn ready(
        &self,
        process: &mut EngineProcess,
        slots: i32,
        context: u32,
    ) -> Result<Value, String> {
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            self.check()?;
            if process
                .child
                .try_wait()
                .map_err(|e| e.to_string())?
                .is_some()
            {
                return Err("validation engine exited; see the isolated engine log".into());
            }
            if self.request("/health", None).is_ok() {
                break;
            }
            if Instant::now() > deadline {
                return Err("validation engine did not become ready".into());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let result = self.request("/slots", None)?;
        if result.as_array().map(Vec::len) != Some(slots as usize)
            || !result.as_array().is_some_and(|rows| {
                rows.iter()
                    .all(|row| row["n_ctx"].as_u64() == Some(context as u64))
            })
        {
            return Err("effective slot count differs from the requested configuration".into());
        }
        Ok(Value::Array(
            result
                .as_array()
                .ok_or("slot list missing")?
                .iter()
                .map(|row| json!({"id":row["id"],"n_ctx":row["n_ctx"]}))
                .collect(),
        ))
    }
    fn completion(&self, id: i32, prompt: &str, count: u32, cache: bool) -> Result<Value, String> {
        self.generate(id, prompt, count, cache, None)
    }
    fn generate(
        &self,
        id: i32,
        prompt: &str,
        count: u32,
        cache: bool,
        grammar: Option<&str>,
    ) -> Result<Value, String> {
        let mut body = json!({"id_slot":id,"prompt":prompt,"n_predict":count,
            "temperature":0,"seed":42,"cache_prompt":cache,"return_tokens":true,"ignore_eos":true});
        if let Some(grammar) = grammar {
            body["grammar"] = json!(grammar);
        }
        let result = self.request("/completion", Some(body))?;
        if result["content"].as_str().map_or(true, |s| s.is_empty())
            || result["tokens"]
                .as_array()
                .map_or(true, |tokens| tokens.is_empty())
        {
            return Err("validation did not produce text and token evidence".into());
        }
        // Native completion responses echo long prompts/settings. Keep only
        // inference evidence so task journals stay bounded even for four slots.
        Ok(Value::Object(
            [
                "content",
                "tokens",
                "timings",
                "tokens_evaluated",
                "tokens_predicted",
                "stop_type",
                "truncated",
            ]
            .into_iter()
            .filter_map(|key| {
                result
                    .get(key)
                    .map(|value| (key.to_string(), value.clone()))
            })
            .collect(),
        ))
    }
}

fn same_tokens(expected: &Value, actual: &Value) -> Result<(), String> {
    if expected["tokens"] != actual["tokens"] || expected["content"] != actual["content"] {
        return Err("restored generation differs from the live reference".into());
    }
    Ok(())
}

fn same_output(expected: &Value, actual: &Value) -> Result<(), String> {
    same_tokens(expected, actual)?;
    for field in ["draft_n", "draft_n_accepted"] {
        if expected["timings"][field] != actual["timings"][field] {
            return Err(format!("restored speculative counter differs: {field}"));
        }
    }
    Ok(())
}

fn inspect_slot(path: &Path, draft_required: bool) -> Result<Value, String> {
    fn read<const N: usize>(file: &mut fs::File, at: u64) -> Result<[u8; N], String> {
        let mut bytes = [0; N];
        file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
        file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }
    fn u32_at(file: &mut fs::File, at: u64) -> Result<u32, String> {
        Ok(u32::from_le_bytes(read(file, at)?))
    }
    fn u64_at(file: &mut fs::File, at: u64) -> Result<u64, String> {
        Ok(u64::from_le_bytes(read(file, at)?))
    }
    fn blob(file: &mut fs::File, at: &mut u64, size: u64) -> Result<u64, String> {
        let n = u64_at(file, *at)?;
        *at = at
            .checked_add(8)
            .and_then(|at| at.checked_add(n))
            .filter(|end| *end <= size)
            .ok_or("state blob exceeds file")?;
        Ok(n)
    }
    fn trailer(
        file: &mut fs::File,
        mut at: u64,
        size: u64,
        draft_required: bool,
    ) -> Result<Value, String> {
        let mut checkpoints = Vec::new();
        if read::<4>(file, at)? == *b"SCKP" {
            if u32_at(file, at + 4)? != 1 {
                return Err("unsupported SCKP version".into());
            }
            let count = u32_at(file, at + 8)?;
            if !(1..=1024).contains(&count) {
                return Err("invalid SCKP count".into());
            }
            at += 12;
            for _ in 0..count {
                let tokens = u64_at(file, at)?;
                at = at.checked_add(16).ok_or("invalid checkpoint offset")?;
                let target = blob(file, &mut at, size)?;
                let draft = blob(file, &mut at, size)?;
                let speculative = blob(file, &mut at, size)?;
                if target == 0 || tokens == 0 || (draft_required && draft == 0) {
                    return Err("incomplete checkpoint payload".into());
                }
                checkpoints.push(json!({"tokens":tokens,"target_bytes":target,"draft_bytes":draft,"speculative_bytes":speculative}));
            }
        }
        let mut draft_bytes = 0;
        let mut speculative_bytes = 0;
        if at < size && read::<4>(file, at)? == *b"SDFT" {
            if u32_at(file, at + 4)? != 1 {
                return Err("unsupported SDFT version".into());
            }
            at += 8;
            draft_bytes = blob(file, &mut at, size)?;
            speculative_bytes = blob(file, &mut at, size)?;
        }
        if at != size || (draft_required && draft_bytes == 0) {
            return Err("incomplete or unrecognized state trailer".into());
        }
        Ok(
            json!({"checkpoints":checkpoints,"draft_bytes":draft_bytes,"speculative_bytes":speculative_bytes}),
        )
    }
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size < 13 || u32_at(&mut file, 0)? != 0x67677371 || u32_at(&mut file, 4)? != 4 {
        return Err("unsupported saved-state header".into());
    }
    let body = 12 + 4 * u32_at(&mut file, 8)? as u64;
    if body >= size {
        return Err("missing main-model state".into());
    }
    let mut at = body;
    let mut scan = vec![0_u8; 1024 * 1024];
    let mut found = None;
    while at < size {
        file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
        let n = file.read(&mut scan).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        for (i, magic) in scan[..n].windows(4).enumerate() {
            if (magic == b"SCKP" || magic == b"SDFT") && at + i as u64 > body {
                if let Ok(result) = trailer(&mut file, at + i as u64, size, draft_required) {
                    if magic == b"SCKP" {
                        return Ok(result);
                    }
                    found = Some(result);
                }
            }
        }
        if n < 4 {
            break;
        }
        at += (n - 3) as u64;
    }
    found
        .or_else(|| {
            (!draft_required)
                .then(|| json!({"checkpoints":[],"draft_bytes":0,"speculative_bytes":0}))
        })
        .ok_or_else(|| "no complete SDFT payload was found".into())
}

/// Owned, isolated process pair. Does not edit an instance, its slots, or its cache.
pub(crate) fn run(
    store: CheckpointStore,
    config: InstanceConfig,
    engine: String,
    fingerprint: CheckpointFingerprint,
    require_checkpoints: bool,
    _lease: Lease,
) -> Result<Receipt, String> {
    if CANCEL.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("checkpoint validation cancelled".into());
    }
    store
        .prepare_qualification_storage()
        .map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(store.root().join(".qualification.lock"))
        .map_err(|e| e.to_string())?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| "another manager process is validating this data directory")?;
    let receipt_file = receipt_path(&store, &fingerprint)?;

    crate::persistence::atomic_write(
        &receipt_file.with_extension("blocked.json"),
        b"{\"reason\":\"validation_pending_or_failed\"}",
        None,
    )
    .map_err(|e| e.to_string())?;
    let hardware = hardware_identity()?;
    let root = store
        .prepare_qualification_run()
        .map_err(|e| e.to_string())?;
    let outcome = run_pair(
        &store,
        &config,
        &engine,
        &fingerprint,
        &hardware,
        &root,
        require_checkpoints,
    );
    if let Err(error) = &outcome {
        let _ = fs::write(
            root.join("failure.json"),
            json!({"schema":SCHEMA,"error":error}).to_string(),
        );
    }
    if outcome.is_ok()
        || outcome
            .as_ref()
            .is_err_and(|error| error == "checkpoint validation cancelled")
    {
        fs::remove_file(receipt_file.with_extension("blocked.json")).map_err(|e| e.to_string())?;
    }
    outcome
}

fn run_pair(
    store: &CheckpointStore,
    original: &InstanceConfig,
    engine: &str,
    fingerprint: &CheckpointFingerprint,
    hardware: &str,
    root: &Path,
    require_checkpoints: bool,
) -> Result<Receipt, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let key = uuid::Uuid::new_v4().to_string();
    let mut config = original.clone();
    config.host = "127.0.0.1".into();
    config.port = port as _;
    config.api_key = key.clone();
    config.api_key_file.clear();
    config.slot_save_path = root.to_string_lossy().into_owned();
    config.slots_enabled = true;
    if let Some(fields) = config.explicit_overrides.as_mut() {
        for key in [
            "host",
            "port",
            "api_key",
            "api_key_file",
            "slot_save_path",
            "slots_enabled",
        ] {
            if !fields.iter().any(|field| field == key) {
                fields.push(key.into());
            }
        }
    }
    let argv = crate::commands::server::generate_normalized_command(&config, engine);
    let probe = Probe {
        client: reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(180))
            .no_proxy()
            .build()
            .map_err(|e| e.to_string())?,
        base: format!("http://127.0.0.1:{port}"),
        key,
        deadline: Instant::now() + Duration::from_secs(1200),
    };
    drop(listener);
    probe.check()?;
    let mut processes = Vec::new();
    let mixed = crate::speculative::parse_speculative_types(&config.spec_type).len() > 1;
    let rollback_prompt = "Write the result of 17 times 23 as 391, then repeat 391 separated by spaces. Answer: 391 391 391 ";
    let rollback_prefix = ["391"; 15]
        .into_iter()
        .chain(["397"])
        .collect::<Vec<_>>()
        .join(" ");
    let rollback_grammar = format!("root ::= \"{rollback_prefix}\" (\" \" \"397\")*");
    let reference = if mixed {
        let mut baseline = config.clone();
        baseline.spec_type = "none".into();
        baseline.draft_model_path.clear();
        let baseline_argv = crate::commands::server::generate_normalized_command(&baseline, engine);
        let mut child = EngineProcess::spawn(&baseline_argv, root, "no-draft-reference")?;
        probe.ready(&mut child, config.parallel, config.ctx_size)?;
        let rollback = probe.generate(0, rollback_prompt, 128, false, Some(&rollback_grammar))?;
        let following = probe.completion(
            0,
            &format!(
                "{rollback_prompt}{}",
                rollback["content"]
                    .as_str()
                    .ok_or("missing rollback text")?
            ),
            128,
            true,
        )?;
        child.stop()?;
        processes.push(json!({"pid":child.child.id(),"role":"no_draft_reference","exited":true}));
        Some((rollback, following))
    } else {
        None
    };
    let mut first = EngineProcess::spawn(&argv, root, "before")?;
    let effective_slots = probe.ready(&mut first, config.parallel, config.ctx_size)?;
    let log = fs::read_to_string(root.join("before.log")).map_err(|e| e.to_string())?;
    let offload = regex_lite::Regex::new(r"offloaded (\d+)/(\d+) layers to GPU")
        .map_err(|e| e.to_string())?;
    let layers = offload.captures_iter(&log).collect::<Vec<_>>();
    if !log.contains("ROCm") && !log.contains("HIP")
        || layers.is_empty()
        || !layers.iter().all(|row| row[1] == row[2] && &row[1] != "0")
    {
        return Err("Windows/ROCm full GPU offload was not observed; see before.log".into());
    }
    let mut cases = Vec::new();
    let mut rewrites = Vec::new();
    let records = ((config.ctx_size / config.parallel as u32) / 32).clamp(32, 512);
    for id in 0..config.parallel {
        let marker = [391, 437, 493, 551][id as usize];
        let revised_marker = marker + 6;
        let secret = ["cobalt", "amber", "violet", "silver"][id as usize];
        let prefix = (0..records)
            .map(|row| format!("Record {row}: code {marker}; secret {secret} for slot {id}.\n"))
            .collect::<String>();
        let prompt = format!("{prefix}Slot {id}. Repeat that answer separated by spaces. Answer: {marker} {marker} {marker} ");
        let revised_prefix = (0..records)
            .map(|row| {
                format!(
                    "Record {row}: code {}; secret {secret} for slot {id}.\n",
                    if row + 20 >= records {
                        revised_marker
                    } else {
                        marker
                    }
                )
            })
            .collect::<String>();
        let revised = format!("{revised_prefix}Slot {id}. Use the last record. Repeat that answer separated by spaces. Answer: {revised_marker} {revised_marker} {revised_marker} ");
        let reference = probe.completion(id, &revised, 64, false)?;
        rewrites.push((revised, reference));
        let warm = probe.completion(id, &prompt, 24, false)?;
        let continuation = format!(
            "{prompt}{}\nNow repeat only the secret word from the records, separated by spaces. Answer:",
            warm["content"].as_str().ok_or("missing generated prefix")?
        );
        let filename = format!("slot-{id}.bin");
        let save = probe.request(
            &format!("/slots/{id}?action=save"),
            Some(json!({"filename":filename})),
        )?;
        let path = root.join(&filename);
        let bytes = fs::metadata(&path).map_err(|e| e.to_string())?.len();
        if save["id_slot"] != id
            || save["filename"] != filename
            || bytes == 0
            || save["n_saved"].as_u64().unwrap_or(0) == 0
            || save["n_written"].as_u64() != Some(bytes)
        {
            return Err("incomplete slot save evidence".into());
        }
        let digest = store.content_sha256(&path).map_err(|e| e.to_string())?;
        let format = inspect_slot(
            &path,
            crate::speculative::checkpoint_uses_draft_state(&config.spec_type),
        )?;
        if require_checkpoints
            && format["checkpoints"]
                .as_array()
                .map_or(true, |rows| rows.is_empty())
        {
            return Err("windowed/recurrent model did not persist its context checkpoints".into());
        }
        fs::write(
            root.join(format!("slot-{id}-format.json")),
            serde_json::to_vec_pretty(&format).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let live = probe.completion(id, &continuation, 64, true)?;
        if !live["content"]
            .as_str()
            .is_some_and(|text| text.contains(secret))
        {
            return Err(
                "the uncached/live reference did not retrieve its slot-specific secret".into(),
            );
        }
        if crate::speculative::checkpoint_uses_draft_state(&config.spec_type)
            && (live["timings"]["draft_n"].as_u64().unwrap_or(0) == 0
                || live["timings"]["draft_n_accepted"].as_u64().unwrap_or(0) == 0)
        {
            return Err("draft proposal/acceptance evidence is missing".into());
        }
        cases.push((id, filename, bytes, digest, save, continuation, live));
    }
    if let Some((expected, following)) = reference {
        probe.completion(0, rollback_prompt, 128, false)?;
        let rollback = probe.generate(0, rollback_prompt, 128, false, Some(&rollback_grammar))?;
        same_tokens(&expected, &rollback)?;
        let next = probe.completion(
            0,
            &format!(
                "{rollback_prompt}{}",
                rollback["content"]
                    .as_str()
                    .ok_or("missing rollback text")?
            ),
            128,
            true,
        )?;
        same_tokens(&following, &next)?;
        let proposed = rollback["timings"]["draft_n"].as_u64().unwrap_or(0);
        let accepted = rollback["timings"]["draft_n_accepted"]
            .as_u64()
            .unwrap_or(0);
        if proposed == 0 || accepted >= proposed {
            return Err("mixed rollback test did not exercise rejected draft tokens".into());
        }
        fs::write(root.join("mixed-rollback.json"), serde_json::to_vec_pretty(&json!({"expected":expected,"actual":rollback,"following_reference":following,"following":next})).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    let old_pid = first.child.id();
    first.stop()?;
    processes.push(json!({"pid":old_pid,"role":"saved","exited":true}));
    let mut second = EngineProcess::spawn(&argv, root, "after")?;
    probe.ready(&mut second, config.parallel, config.ctx_size)?;
    if second.child.id() == old_pid {
        return Err("cannot establish a distinct fresh process identity".into());
    }
    let mut restores = Vec::new();
    for (id, filename, bytes, _, save, _, _) in &cases {
        let restored = probe.request(
            &format!("/slots/{id}?action=restore"),
            Some(json!({"filename":filename})),
        )?;
        if restored["id_slot"] != *id
            || restored["filename"] != *filename
            || restored["n_read"].as_u64() != Some(*bytes)
            || restored["n_restored"] != save["n_saved"]
        {
            return Err("restore did not consume the complete saved state".into());
        }
        restores.push(restored);
    }
    let mut slots = Vec::new();
    for ((id, filename, _, digest, save, continuation, live), restored) in
        cases.iter().zip(&restores)
    {
        let generated = probe.completion(*id, continuation, 64, true)?;
        same_output(live, &generated)?;
        let reused = generated["timings"]["cache_n"].as_u64().unwrap_or(0);
        if reused
            < save["n_saved"]
                .as_u64()
                .unwrap_or(u64::MAX)
                .saturating_sub(1)
                .max(1)
        {
            return Err(
                "restored inference replayed the saved prefix instead of reusing it".into(),
            );
        }
        if store
            .content_sha256(&root.join(filename))
            .map_err(|e| e.to_string())?
            != *digest
        {
            return Err("restore modified an immutable saved cache".into());
        }
        let (revised, cold_reference) = &rewrites[*id as usize];
        let rewrite = probe.completion(*id, revised, 64, true)?;
        same_tokens(cold_reference, &rewrite)?;
        let recomputed = probe.completion(*id, revised, 64, false)?;
        same_tokens(cold_reference, &recomputed)?;
        slots.push(json!({"rewrite":rewrite,"rewrite_reference":cold_reference,"recomputed":recomputed,"id":id,"passed":true,"save":save,"restore":restored,"live":live,"generated":generated,"file_sha256":digest}));
    }
    second.stop()?;
    processes.push(json!({"pid":second.child.id(),"role":"restored","exited":true}));
    // A late, partial restore failure contaminates the whole process. Discard it;
    // never infer on its other slots or publish a partially recovered generation.
    let last = cases.last().ok_or("no slot test cases")?;
    let corrupt = root.join("truncated-test.bin");
    fs::copy(root.join(&last.1), &corrupt).map_err(|e| e.to_string())?;
    fs::OpenOptions::new()
        .write(true)
        .open(&corrupt)
        .and_then(|file| file.set_len(last.2.saturating_sub(32)))
        .map_err(|e| e.to_string())?;
    let mut failed = EngineProcess::spawn(&argv, root, "failed-restore")?;
    probe.ready(&mut failed, config.parallel, config.ctx_size)?;
    let failed_restore = probe.request(
        &format!("/slots/{}?action=restore", last.0),
        Some(json!({"filename":"truncated-test.bin"})),
    );
    failed.stop()?;
    processes.push(json!({"pid":failed.child.id(),"role":"failed_restore","exited":true}));
    fs::write(
        root.join("failed-restore-response.json"),
        serde_json::to_vec_pretty(&failed_restore).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let corrupt_bytes = fs::metadata(&corrupt).map_err(|e| e.to_string())?.len();
    if failed_restore.as_ref().is_ok_and(|response| {
        response["n_read"].as_u64() == Some(corrupt_bytes)
            && response["n_restored"] == last.4["n_saved"]
    }) {
        return Err("truncated state passed the manager full-read/token-count guard".into());
    }
    let mut clean = EngineProcess::spawn(&argv, root, "clean-recovery")?;
    probe.ready(&mut clean, config.parallel, config.ctx_size)?;
    for (id, filename, bytes, digest, save, _, _) in &cases {
        if store
            .content_sha256(&root.join(filename))
            .map_err(|e| e.to_string())?
            != *digest
        {
            return Err("failed restore changed the old cache".into());
        }
        let restored = probe.request(
            &format!("/slots/{id}?action=restore"),
            Some(json!({"filename":filename})),
        )?;
        if restored["id_slot"] != *id
            || restored["filename"] != *filename
            || restored["n_read"].as_u64() != Some(*bytes)
            || restored["n_restored"] != save["n_saved"]
        {
            return Err("clean recovery did not consume complete state".into());
        }
    }
    for (id, _, _, _, _, continuation, live) in &cases {
        same_output(live, &probe.completion(*id, continuation, 64, true)?)?;
    }
    clean.stop()?;
    processes.push(json!({"pid":clean.child.id(),"role":"clean_recovery","exited":true}));
    // No certificate is issued after a hardware, runtime, model or configuration race.
    let current = store
        .build_fingerprint(
            original,
            Path::new(&original.model_path),
            Path::new(engine),
            &fingerprint.engine_version,
            &fingerprint.backend,
        )
        .map_err(|e| e.to_string())?;
    if current != *fingerprint || hardware_identity()? != hardware {
        return Err("validation inputs changed during the test".into());
    }
    probe.check()?;
    let receipt = Receipt {
        schema: SCHEMA,
        fingerprint: fingerprint.clone(),
        hardware: hardware.into(),
        completed_at: chrono::Utc::now().to_rfc3339(),
        old_process_exited: true,
        processes,
        effective_slots,
        slots,
        evidence_directory: root.to_string_lossy().into_owned(),
    };
    let path = receipt_path(store, fingerprint)?;
    store
        .prepare_qualification_storage()
        .map_err(|e| e.to_string())?;
    crate::persistence::atomic_write(
        &path,
        &serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?,
        None,
    )
    .map_err(|e| e.to_string())?;
    // Only owned test payloads are removed, after both child processes have exited.
    // Keep small logs and the receipt for review; a failure keeps evidence intact.
    for (_, filename, _, _, _, _, _) in cases {
        let _ = fs::remove_file(root.join(filename));
    }
    let _ = fs::remove_file(corrupt);
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::{build_checkpoint_fingerprint, FingerprintMaterials};

    fn config() -> InstanceConfig {
        InstanceConfig {
            model_path: "target.gguf".into(),
            draft_model_path: "different-draft.gguf".into(),
            spec_type: "ngram-mod,draft-mtp".into(),
            ctx_size: 65536,
            parallel: 4,
            gpu_layers: 999,
            gpu_layers_auto: false,
            draft_gpu_layers: 999,
            ctx_size_auto: false,
            flash_attn: "on".into(),
            fit_mode: "off".into(),
            ctx_checkpoints: 2,
            draft_tokens: 3,
            kv_unified: true,
            kv_unified_mode: "on".into(),
            cache_type_k: "q8_0".into(),
            cache_type_v: "q8_0".into(),
            cache_type_draft_k: "q8_0".into(),
            cache_type_draft_v: "q8_0".into(),
            ..InstanceConfig::default()
        }
    }
    fn fingerprint(config: &InstanceConfig) -> CheckpointFingerprint {
        build_checkpoint_fingerprint(
            config,
            &FingerprintMaterials {
                model_sha256: "a".repeat(64),
                draft_model_sha256: Some("b".repeat(64)),
                engine_sha256: "c".repeat(64),
                engine_version: "local-test".into(),
                backend: "ROCm".into(),
                chat_template_file_sha256: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn unreviewed_build_and_model_can_be_candidates_but_unsafe_shape_cannot() {
        let mut config = config();
        let caps = EngineCapabilities {
            context_checkpoint_persistence: true,
            speculative_types: vec!["ngram-mod".into(), "draft-mtp".into()],
            ..Default::default()
        };
        assert_eq!(candidate(&config, &caps), cfg!(windows));
        config.parallel = 8;
        assert!(!candidate(&config, &caps));
        config.parallel = 4;
        config.cache_type_v = "q4_0".into();
        assert!(!candidate(&config, &caps));
        config.cache_type_v = "q8_0".into();
        config.explicit_overrides = Some(vec!["spec_type".into()]);
        assert!(
            !candidate(&config, &caps),
            "omitted context/GPU fields cannot qualify retained form values"
        );
    }

    #[test]
    fn admission_opt_in_does_not_change_cache_identity_but_active_argv_intent_does() {
        let mut config = config();
        let initial = fingerprint(&config);
        config.kv_checkpoint.allow_local_validation = true;
        assert_eq!(initial, fingerprint(&config));
        config.explicit_overrides = Some(vec!["ctx_size".into(), "ubatch_size".into()]);
        let explicit = fingerprint(&config);
        assert_ne!(initial, explicit);
        config.explicit_overrides = Some(vec![
            "ubatch_size".into(),
            "ctx_size".into(),
            "ctx_size".into(),
        ]);
        assert_eq!(explicit, fingerprint(&config));
        config.explicit_overrides.as_mut().unwrap().extend([
            "metrics".into(),
            "verbose".into(),
            "host".into(),
        ]);
        assert_eq!(
            explicit,
            fingerprint(&config),
            "unrelated server controls must not invalidate cached state"
        );
        config.explicit_overrides = Some(vec!["ctx_size_auto".into(), "ubatch_size".into()]);
        assert_eq!(
            explicit,
            fingerprint(&config),
            "equivalent emitted field groups have one identity"
        );
        config.explicit_overrides = Some(vec!["ctx_size".into()]);
        assert_ne!(explicit, fingerprint(&config));
    }

    #[test]
    fn qualification_cannot_be_reused_for_another_context_build_model_driver_or_slot_count() {
        let config = config();
        let fingerprint = fingerprint(&config);
        let receipt = Receipt {
            schema: SCHEMA,
            fingerprint: fingerprint.clone(),
            hardware: "machine-driver".into(),
            completed_at: "test".into(),
            old_process_exited: true,
            effective_slots: json!([]),
            processes: (0..4).map(|pid| json!({"pid":pid,"exited":true})).collect(),
            evidence_directory: "test".into(),
            slots: (0..4).map(|id| json!({"id":id,"passed":true})).collect(),
        };
        assert!(matches(&receipt, &fingerprint, "machine-driver", 4));
        assert!(!matches(&receipt, &fingerprint, "new-driver", 4));
        assert!(!matches(&receipt, &fingerprint, "machine-driver", 2));
        let mut changed = fingerprint.clone();
        changed.engine_sha256 = "d".repeat(64);
        assert!(!matches(&receipt, &changed, "machine-driver", 4));
        changed = fingerprint.clone();
        changed.draft_model_sha256 = Some("e".repeat(64));
        assert!(!matches(&receipt, &changed, "machine-driver", 4));
        let mut altered = config.clone();
        altered.ctx_size *= 2;
        assert!(!matches(
            &receipt,
            &super::tests::fingerprint(&altered),
            "machine-driver",
            4
        ));
        let mut incomplete = receipt.clone();
        incomplete.old_process_exited = false;
        assert!(!matches(&incomplete, &fingerprint, "machine-driver", 4));
        incomplete = receipt;
        incomplete.slots[3]["passed"] = json!(false);
        assert!(!matches(&incomplete, &fingerprint, "machine-driver", 4));
    }

    #[test]
    fn bounded_state_reader_rejects_truncation_unknown_versions_and_unbounded_lengths() {
        let root =
            std::env::temp_dir().join(format!("checkpoint-format-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let file = root.join("state.bin");
        let mut data = Vec::new();
        for n in [0x67677371_u32, 4, 1, 391] {
            data.extend(n.to_le_bytes());
        }
        data.extend([0_u8; 32]);
        data.extend(b"SDFT");
        data.extend(1_u32.to_le_bytes());
        let length_offset = data.len();
        data.extend(32_u64.to_le_bytes());
        data.extend([7_u8; 32]);
        data.extend(0_u64.to_le_bytes());
        fs::write(&file, &data).unwrap();
        assert_eq!(inspect_slot(&file, true).unwrap()["draft_bytes"], 32);
        fs::write(&file, &data[..data.len() - 1]).unwrap();
        assert!(inspect_slot(&file, true).is_err());
        let mut unknown = data.clone();
        unknown[length_offset - 4..length_offset].copy_from_slice(&2_u32.to_le_bytes());
        fs::write(&file, unknown).unwrap();
        assert!(inspect_slot(&file, true).is_err());
        data[length_offset..length_offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        fs::write(&file, data).unwrap();
        assert!(inspect_slot(&file, true).is_err());
        fs::remove_file(file).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn qualification_storage_cannot_collide_with_an_instance_namespace() {
        let root = std::env::temp_dir().join(format!(
            "checkpoint-namespace-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = CheckpointStore::from_root(&root);
        store.prepare_qualification_storage().unwrap();
        let run = store.prepare_qualification_run().unwrap();
        let regular_instance = store.prepare_instance("qualifications").unwrap();
        assert!(!regular_instance.starts_with(store.qualification_root()));
        assert!(store.prepare_instance(".qualifications").is_err());
        assert!(store.prepare_instance(".qualification-runs").is_err());
        assert!(run.starts_with(root.join(".qualification-runs")));
        // Only the absolute UUID test root created above is removed.
        assert!(root.is_absolute() && root.starts_with(std::env::temp_dir()));
        fs::remove_dir_all(root).unwrap();
    }
}
