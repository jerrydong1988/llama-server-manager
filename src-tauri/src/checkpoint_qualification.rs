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
static ACTIVE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn cancel(id: &str) -> bool {
    let active = ACTIVE.lock().unwrap();
    if active.as_deref() != Some(id) {
        return false;
    }
    CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
    true
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
        && !crate::checkpoint::has_native_argument_overrides()
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
        && (kinds.is_empty() || emits("spec_type"))
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
        .map(|(key, value)| (key.to_ascii_uppercase(), value))
        .filter(|(key, _)| {
            [
                "GGML_",
                "HIP_",
                "HIPBLASLT_",
                "ROCR_",
                "ROCBLAS_",
                "TENSILE_",
                "CUDA_",
                "LLAMA_ARG_",
            ]
            .iter()
            .any(|prefix| key.starts_with(prefix))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut digest = Sha256::new();
    digest.update(result.stdout);
    digest.update(serde_json::to_vec(&environment).map_err(|e| e.to_string())?);
    Ok(format!("{:x}", digest.finalize()))
}

pub(crate) struct Lease(String);
pub(crate) fn reserve(id: &str) -> Result<Lease, String> {
    let mut active = ACTIVE.lock().unwrap();
    if active.is_some() {
        return Err("another checkpoint validation is running".into());
    }
    *active = Some(id.to_owned());
    CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
    Ok(Lease(id.to_owned()))
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut active = ACTIVE.lock().unwrap();
        if active.as_deref() == Some(&self.0) {
            *active = None;
        }
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

// A free-port reservation has to be released before llama-server binds it.
// Check the OS listener owner before every HTTP request, so another local
// service cannot receive slot mutations if it wins that interval or reuses it.
fn owns_listener(port: u16, expected_pid: u32) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::ERROR_INSUFFICIENT_BUFFER, NetworkManagement::IpHelper::*,
            Networking::WinSock::AF_INET,
        };
        let mut size = 0_u32;
        let mut buffer: Vec<u32> = Vec::new();
        for _ in 0..3 {
            let status = GetExtendedTcpTable(
                if buffer.is_empty() {
                    std::ptr::null_mut()
                } else {
                    buffer.as_mut_ptr().cast()
                },
                &mut size,
                0,
                AF_INET as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            );
            if status == ERROR_INSUFFICIENT_BUFFER {
                if !(4..=1024 * 1024).contains(&size) {
                    return false;
                }
                buffer.resize(size.div_ceil(4) as usize, 0);
                continue;
            }
            if status != 0 || size < 4 || size as usize > buffer.len() * 4 {
                return false;
            }
            let count = buffer[0] as usize;
            let row_size = std::mem::size_of::<MIB_TCPROW_OWNER_PID>();
            if count
                .checked_mul(row_size)
                .and_then(|n| n.checked_add(4))
                .map_or(true, |end| end > size as usize)
            {
                return false;
            }
            for index in 0..count {
                let row = std::ptr::read_unaligned(
                    buffer
                        .as_ptr()
                        .cast::<u8>()
                        .add(4 + index * row_size)
                        .cast::<MIB_TCPROW_OWNER_PID>(),
                );
                if row.dwLocalAddr.to_ne_bytes() == [127, 0, 0, 1]
                    && u16::from_be(row.dwLocalPort as u16) == port
                {
                    return row.dwOwningPid == expected_pid;
                }
            }
            return false;
        }
        false
    }
    #[cfg(not(windows))]
    {
        let _ = (port, expected_pid);
        false
    }
}

struct Probe {
    client: reqwest::blocking::Client,
    base: String,
    port: u16,
    owner_pid: std::cell::Cell<u32>,
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
        if !owns_listener(self.port, self.owner_pid.get()) {
            return Err("validation port is not owned by this test process".into());
        }
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
        self.owner_pid.set(process.child.id());
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
        Self::inference_evidence(self.request("/completion", Some(body))?)
    }
    fn number_answer(
        &self,
        id: i32,
        prompt: &str,
        cache: bool,
        expected: i32,
    ) -> Result<Value, String> {
        // The grammar permits every three-digit answer; it does not force the
        // expected answer. End at the answer rather than forcing a model to
        // continue beyond its natural response and comparing unrelated prose.
        let result = Self::inference_evidence(self.request(
            "/completion",
            Some(json!({
                "id_slot":id,"prompt":prompt,"n_predict":8,"temperature":0,"seed":42,
                "cache_prompt":cache,"return_tokens":true,"ignore_eos":false,
                "grammar":"root ::= [0-9] [0-9] [0-9]"
            })),
        )?)?;
        if !cache && result["timings"]["cache_n"].as_u64() != Some(0) {
            return Err("cold control unexpectedly reused cached tokens".into());
        }
        if result["content"].as_str().map(str::trim) != Some(expected.to_string().as_str()) {
            return Err(format!(
                "slot {id} numeric control did not retrieve the final record's code {expected}"
            ));
        }
        Ok(result)
    }
    fn inference_evidence(result: Value) -> Result<Value, String> {
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

fn draft_acceptance(output: &Value) -> Option<f64> {
    let proposed = output["timings"]["draft_n"].as_u64()?;
    let accepted = output["timings"]["draft_n_accepted"].as_u64()?;
    (proposed > 0 && accepted <= proposed).then(|| accepted as f64 / proposed as f64)
}

fn same_tokens(expected: &Value, actual: &Value) -> Result<(), String> {
    if expected["tokens"] != actual["tokens"] || expected["content"] != actual["content"] {
        return Err("restored generation differs from the live reference".into());
    }
    Ok(())
}

// Proposal batches can differ at a rejection/end boundary while producing
// identical target tokens. Permit at most one configured draft batch of count
// drift and no more than five percentage points of acceptance-rate loss.
fn same_output(expected: &Value, actual: &Value, batch: u32) -> Result<(), String> {
    same_tokens(expected, actual)?;
    let counts = |value: &Value| {
        [
            value["timings"]["draft_n"].as_u64(),
            value["timings"]["draft_n_accepted"].as_u64(),
        ]
    };
    let before = counts(expected);
    let after = counts(actual);
    if [expected, actual].iter().all(|value| {
        ["draft_n", "draft_n_accepted"]
            .iter()
            .all(|key| value["timings"][key].is_null())
    }) {
        return Ok(());
    }
    let ([Some(proposed), Some(accepted)], [Some(restored_proposed), Some(restored_accepted)]) =
        (before, after)
    else {
        return Err("restored speculative counters are missing or invalid".into());
    };
    if accepted > proposed || restored_accepted > restored_proposed {
        return Err("restored speculative counters are inconsistent".into());
    }
    if proposed == 0 || restored_proposed == 0 {
        return if before == after {
            Ok(())
        } else {
            Err("restored speculation stopped proposing tokens".into())
        };
    }
    if accepted > 0 && restored_accepted == 0 {
        return Err("restored speculation stopped accepting tokens".into());
    }
    if proposed.abs_diff(restored_proposed) > u64::from(batch.max(1))
        || accepted.abs_diff(restored_accepted) > u64::from(batch.max(1))
        || restored_accepted as f64 / restored_proposed as f64 + 0.05
            < accepted as f64 / proposed as f64
    {
        return Err(
            "restored draft efficiency differs beyond one batch or five percentage points".into(),
        );
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
    let blocked_file = receipt_file.with_extension("blocked.json");
    let previously_blocked = blocked_file.exists();

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
    finish_attempt_marker(&blocked_file, previously_blocked, &outcome)?;
    outcome
}

fn finish_attempt_marker(
    path: &Path,
    previously_blocked: bool,
    outcome: &Result<Receipt, String>,
) -> Result<(), String> {
    // Cancelling a later attempt must not revive a receipt already revoked by
    // an earlier failed validation. Only a successful test can clear that block.
    if outcome.is_ok()
        || (!previously_blocked
            && outcome
                .as_ref()
                .is_err_and(|error| error == "checkpoint validation cancelled"))
    {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
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
    if !fingerprint.backend.eq_ignore_ascii_case("ROCm")
        && !fingerprint.backend.eq_ignore_ascii_case("HIP")
    {
        return Err("local qualification requires a Windows/ROCm engine".into());
    }
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
        port,
        owner_pid: std::cell::Cell::new(0),
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
        let revised = format!("{revised_prefix}Slot {id}. What is the code in the LAST record? Return only the three digits. Answer:");
        let reference = probe.number_answer(id, &revised, false, revised_marker)?;
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
        if !live["content"].as_str().is_some_and(|text| {
            text.contains(secret)
                && ["cobalt", "amber", "violet", "silver"]
                    .iter()
                    .filter(|other| **other != secret)
                    .all(|other| !text.contains(other))
        }) {
            return Err(
                "the uncached/live reference did not retrieve its slot-specific secret".into(),
            );
        }
        if !crate::speculative::parse_speculative_types(&config.spec_type).is_empty()
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
        same_tokens(&expected, &rollback)
            .map_err(|error| format!("mixed rejection rollback: {error}"))?;
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
        same_tokens(&following, &next)
            .map_err(|error| format!("mixed rollback continuation: {error}"))?;
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
        fs::write(
            root.join(format!("restore-{id}.json")),
            serde_json::to_vec_pretty(&json!({"expected":live,"actual":generated}))
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        same_output(live, &generated, original.draft_tokens)
            .map_err(|error| format!("slot {id} fresh restore: {error}"))?;
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
        let expected = [397, 443, 499, 557][*id as usize];
        let rewrite = probe.number_answer(*id, revised, true, expected)?;
        let recomputed = probe.number_answer(*id, revised, false, expected)?;
        slots.push(json!({"draft_acceptance_before":draft_acceptance(live),"draft_acceptance_after":draft_acceptance(&generated),"counter_tolerance_tokens":original.draft_tokens,"acceptance_drop_tolerance":0.05,"rewrite":rewrite,"rewrite_reference":cold_reference,"recomputed":recomputed,"id":id,"passed":true,"save":save,"restore":restored,"live":live,"generated":generated,"file_sha256":digest}));
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
    let corrupt_bytes = fs::metadata(&corrupt).map_err(|e| e.to_string())?.len();
    let corrupt_digest = store.content_sha256(&corrupt).map_err(|e| e.to_string())?;
    if corrupt_bytes == last.2 || corrupt_digest == last.3 {
        return Err("truncation fixture did not change the saved payload".into());
    }
    let claimed_complete = failed_restore.as_ref().is_ok_and(|response| {
        crate::checkpoint::restore_counts_match_manifest(
            response["n_read"].as_u64().unwrap_or(0),
            response["n_restored"].as_u64().unwrap_or(0),
            last.2,
            last.4["n_saved"].as_u64().unwrap_or(0),
        )
    });
    fs::write(root.join("failed-restore-response.json"), serde_json::to_vec_pretty(&json!({
        "response":failed_restore,"trusted_bytes":last.2,"truncated_bytes":corrupt_bytes,
        "trusted_sha256":last.3,"truncated_sha256":corrupt_digest,"preflight_rejected":true,"full_read_guard_rejected":!claimed_complete
    })).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if claimed_complete {
        return Err(
            "truncated state claimed the original manifest's complete byte/token counts".into(),
        );
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
        same_output(
            live,
            &probe.completion(*id, continuation, 64, true)?,
            original.draft_tokens,
        )?;
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
    fn cancellation_during_preparation_survives_until_the_worker_and_releases_the_lease() {
        let lease = reserve("first").unwrap();
        assert!(!cancel("unrelated"));
        assert!(!CANCEL.load(std::sync::atomic::Ordering::SeqCst));
        assert!(cancel("first"));
        assert!(
            reserve("second").is_err(),
            "a second request cannot reset cancellation"
        );
        let root =
            std::env::temp_dir().join(format!("checkpoint-cancel-test-{}", uuid::Uuid::new_v4()));
        let config = config();
        let identity = fingerprint(&config);
        assert_eq!(
            run(
                CheckpointStore::from_root(&root),
                config,
                "unused.exe".into(),
                identity,
                true,
                lease
            )
            .unwrap_err(),
            "checkpoint validation cancelled"
        );
        assert!(
            !root.exists(),
            "an early cancellation must not create data or start an engine"
        );
        let next = reserve("second").unwrap();
        assert!(
            !cancel("first"),
            "a stale cancellation cannot stop the next validation"
        );
        assert!(!CANCEL.load(std::sync::atomic::Ordering::SeqCst));
        drop(next);
    }

    #[cfg(windows)]
    #[test]
    fn listener_ownership_rejects_a_different_process_and_a_released_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(owns_listener(port, std::process::id()));
        assert!(!owns_listener(port, std::process::id().wrapping_add(1)));
        drop(listener);
        assert!(!owns_listener(port, std::process::id()));
    }

    #[test]
    fn cancellation_cannot_revive_a_previously_failed_qualification() {
        let root =
            std::env::temp_dir().join(format!("checkpoint-block-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let blocked = root.join("previous-failure.blocked.json");
        let pending = root.join("fresh-attempt.blocked.json");
        fs::write(&blocked, b"failed").unwrap();
        fs::write(&pending, b"pending").unwrap();
        let cancelled = Err("checkpoint validation cancelled".to_string());
        finish_attempt_marker(&blocked, true, &cancelled).unwrap();
        finish_attempt_marker(&pending, false, &cancelled).unwrap();
        assert_eq!(fs::read(&blocked).unwrap(), b"failed");
        assert!(!pending.exists());
        fs::remove_file(blocked).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn small_draft_boundary_drift_is_allowed_but_output_changes_and_material_regressions_fail() {
        let output = |proposed, accepted| json!({"content":"violet", "tokens":[12], "timings":{"draft_n":proposed,"draft_n_accepted":accepted}});
        let before = output(58, 52);
        assert!(same_output(&before, &output(57, 52), 3).is_ok());
        assert!(same_output(&before, &output(62, 52), 3).is_err());
        assert!(same_output(&before, &output(58, 49), 3).is_err());
        assert!(same_output(&before, &output(0, 0), 3).is_err());
        assert!(same_output(&before, &output(50, 51), 3).is_err());
        let malformed = json!({"content":"violet", "tokens":[12], "timings":{"draft_n":"invalid","draft_n_accepted":"invalid"}});
        assert!(same_output(&malformed, &malformed, 3).is_err());
        let mut changed = output(58, 52);
        changed["tokens"] = json!([99]);
        assert!(same_output(&before, &changed, 3).is_err());
    }

    #[test]
    fn inference_receipts_keep_tokens_and_timings_without_echoing_long_prompts() {
        let result = Probe::inference_evidence(json!({"content":"cobalt", "tokens":[12,34],
            "timings":{"cache_n":8000,"draft_n":2,"draft_n_accepted":2},
            "prompt":"large private prompt".repeat(100_000),"generation_settings":{"unneeded":true}})).unwrap();
        assert_eq!(result["tokens"], json!([12, 34]));
        assert_eq!(result["timings"]["cache_n"], 8000);
        assert!(result.get("prompt").is_none() && result.get("generation_settings").is_none());
        assert!(serde_json::to_vec(&result).unwrap().len() < 1024);
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
