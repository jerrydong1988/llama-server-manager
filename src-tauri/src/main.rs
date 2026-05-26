#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use futures_util::StreamExt;

// ── 数据结构 ──────────────────────────────────────────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub size: u64,
    pub architecture: Option<String>,
    pub context_length: Option<u32>,
    pub quant_type: Option<String>,
    pub file_type: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EngineInfo {
    pub id: String,
    pub name: String,
    pub dir: String,
    pub exe: String,
    pub version: String,
    pub backend: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstanceConfig {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub engine_id: String,
    pub model_path: String,
    pub alias: String,
    pub lora_path: String,
    pub mmproj_path: String,
    pub chat_template: String,
    pub reasoning_format: String,
    pub reasoning_effort: String,
    pub reasoning: String,
    pub jinja: bool,
    pub reasoning_budget: String,
    pub grammar_file: String,
    pub ctx_size: u32,
    pub ctx_size_auto: bool,
    pub gpu_layers: u32,
    pub threads: u32,
    pub batch_size: u32,
    pub ubatch_size: u32,
    pub parallel: u32,
    pub cont_batching: bool,
    pub cache_prompt: bool,
    pub threads_batch: u32,
    pub flash_attn: String,
    pub moe_cpu_layers: u32,
    pub mlock: bool,
    pub no_mmap: bool,
    pub numa: bool,
    pub cache_type_k: String,
    pub cache_type_v: String,
    pub draft_model_path: String,
    pub draft_gpu_layers: u32,
    pub draft_tokens: u32,
    pub spec_draft_n_min: u32,
    pub spec_type: String,
    pub host: String,
    pub port: u16,
    pub api_key: String,
    pub ssl_key_file: String,
    pub ssl_cert_file: String,
    pub no_ui: bool,
    pub embedding: bool,
    pub pooling: String,
    pub reranking: bool,
    pub n_predict: i32,
    pub ignore_eos: bool,
    pub json_schema: String,
    pub temp: f32,
    pub top_k: u32,
    pub top_p: f32,
    pub repeat_penalty: f32,
    pub seed: i64,
    pub min_p: f32,
    pub presence_penalty: f32,
    pub frequency_penalty: f32,
    pub repeat_last_n: i32,
    pub mirostat: u8,
    pub mirostat_lr: f32,
    pub mirostat_ent: f32,
    pub xtc_probability: f32,
    pub xtc_threshold: f32,
    pub dynatemp_range: f32,
    pub dynatemp_exp: f32,
    pub typical_p: f32,
    pub dry_multiplier: f32,
    pub dry_base: f32,
    pub dry_allowed_length: u32,
    pub dry_penalty_last_n: i32,
    pub dry_sequence_breaker: String,
    pub timeout: u32,
    pub sleep_idle: i32,
    pub context_shift: bool,
    pub verbose: bool,
    pub custom_args: Vec<String>,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            id: String::new(), name: String::new(), engine_id: String::new(), model_path: String::new(),
            alias: String::new(), lora_path: String::new(), mmproj_path: String::new(),
            chat_template: String::new(), reasoning_format: String::new(),
            reasoning_effort: String::new(), reasoning: String::new(), jinja: false,
            reasoning_budget: String::new(), grammar_file: String::new(),
            ctx_size: 4096, ctx_size_auto: false, gpu_layers: 99, threads: 0,
            batch_size: 2048, ubatch_size: 512, parallel: 1, cont_batching: false,
            cache_prompt: true, threads_batch: 0, flash_attn: "auto".into(),
            moe_cpu_layers: 0, mlock: false, no_mmap: false, numa: false,
            cache_type_k: String::new(), cache_type_v: String::new(),
            draft_model_path: String::new(), draft_gpu_layers: 99, draft_tokens: 16,
            spec_draft_n_min: 0, spec_type: String::new(),
            host: "127.0.0.1".into(), port: 8080, api_key: String::new(),
            ssl_key_file: String::new(), ssl_cert_file: String::new(),
            no_ui: false, embedding: false, pooling: String::new(), reranking: false,
            n_predict: -1, ignore_eos: false, json_schema: String::new(),
            temp: 0.8, top_k: 40, top_p: 0.9, repeat_penalty: 1.1, seed: -1,
            min_p: 0.05, presence_penalty: 0.0, frequency_penalty: 0.0,
            repeat_last_n: 64, mirostat: 0, mirostat_lr: 0.1, mirostat_ent: 5.0,
            xtc_probability: 0.0, xtc_threshold: 0.1, dynatemp_range: 0.0,
            dynatemp_exp: 1.0, typical_p: 1.0, dry_multiplier: 0.0, dry_base: 1.75,
            dry_allowed_length: 2, dry_penalty_last_n: -1,
            dry_sequence_breaker: String::new(), timeout: 600, sleep_idle: -1,
            context_shift: false, verbose: false, custom_args: vec![],
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunningInstance {
    pub instance_id: String,
    pub pid: u32,
    pub port: u16,
    pub host: String,
}

pub struct AppState {
    pub models: Mutex<Vec<ModelInfo>>,
    pub engines: Mutex<Vec<EngineInfo>>,
    pub instances: Mutex<HashMap<String, InstanceConfig>>,
    pub running: Mutex<HashMap<String, RunningInstance>>,
    pub config_dir: Mutex<PathBuf>,
    pub cancel_flags: Mutex<HashMap<String, bool>>,
    pub pause_flags: Mutex<HashMap<String, bool>>,
}

// ── GGUF 元信息解析（复刻原程序 _read_gguf_metadata） ─────────
fn parse_gguf_metadata(path: &Path) -> Result<(Option<String>, Option<u32>, Option<String>), String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}", e))?;
    let size = f.metadata().map(|m| m.len()).unwrap_or(0).min(2_000_000) as usize;
    if size < 24 { return Err("文件太小".into()); }
    let mut data = vec![0u8; size];
    use std::io::Read;
    f.read_exact(&mut data).map_err(|e| format!("{}", e))?;
    if &data[0..4] != b"GGUF" { return Err("不是有效的 GGUF 文件".into()); }

    let metadata_kv_count = u64::from_le_bytes(data[16..24].try_into().unwrap()) as usize;
    let mut pos: usize = 24;

    fn read_string(data: &[u8], pos: &mut usize) -> Option<String> {
        if *pos + 8 > data.len() { return None; }
        let len = u64::from_le_bytes(data[*pos..*pos + 8].try_into().unwrap()) as usize;
        *pos += 8;
        // Sanity check: string length should be reasonable
        if len > 10_000_000 || *pos + len > data.len() { return None; }
        let s = String::from_utf8_lossy(&data[*pos..*pos + len]).to_string();
        *pos += len;
        Some(s)
    }

    let mut architecture: Option<String> = None;
    let mut context_length: Option<u32> = None;
    let mut file_type: Option<u32> = None;

    for _ in 0..metadata_kv_count.min(200) {
        if pos + 2 > data.len() { break; }
        let key = match read_string(&data, &mut pos) {
            Some(k) => k,
            None => break,
        };
        if pos + 4 > data.len() { break; }
        let vtype = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
        pos += 4;

        match vtype {
            8 => {
                if pos + 8 > data.len() { break; }
                if let Some(s) = read_string(&data, &mut pos) {
                    if key == "general.architecture" { architecture = Some(s); }
                } else { break; }
            }
            4 => {
                if pos + 4 > data.len() { break; }
                let v = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
                pos += 4;
                if key == "general.file_type" { file_type = Some(v); }
                if key.contains("context_length") { context_length = Some(v); }
            }
            5 => {
                if pos + 4 > data.len() { break; }
                let v = i32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
                pos += 4;
                if key.contains("context_length") { context_length = Some(v as u32); }
            }
            10 => {
                if pos + 8 > data.len() { break; }
                let v = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
                pos += 8;
                if key.contains("context_length") { context_length = Some(v as u32); }
            }
            7 => { pos += 1; }
            0 | 1 => { pos += 1; }
            2 | 3 => { pos += 2; }
            6 | 12 => { pos += if vtype == 6 { 4 } else { 8 }; }
            9 => {
                if pos + 12 > data.len() { break; }
                let _arr_type = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
                pos += 4;
                let arr_len = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap()) as usize;
                pos += 8;
                if arr_len > 100_000 { break; } // sanity
                let elem_size = match _arr_type {
                    0..=3 => 1_usize, 4|5 => 4, 6|12 => 4, 7 => 1, 8 => 0, 10|11 => 8, _ => 0,
                };
                if elem_size > 0 { pos += (arr_len * elem_size).min(1_000_000); }
                else if _arr_type == 8 {
                    for _ in 0..arr_len.min(1000) {
                        if read_string(&data, &mut pos).is_none() { break; }
                    }
                }
            }
            _ => {}
        }
    }

    // quant_type from file_type number (GGML ftype enum, 0-indexed)
    let quant_type = match file_type {
        Some(0) => Some("F32".into()),
        Some(1) => Some("F16".into()),
        Some(2) => Some("Q4_0".into()),
        Some(3) => Some("Q4_1".into()),
        Some(7) => Some("Q8_0".into()),
        Some(8) => Some("Q5_0".into()),
        Some(9) => Some("Q5_1".into()),
        Some(10) => Some("Q2_K".into()),
        Some(11) => Some("Q3_K_S".into()),
        Some(12) => Some("Q3_K_M".into()),
        Some(13) => Some("Q3_K_L".into()),
        Some(14) => Some("Q4_K_S".into()),
        Some(15) => Some("Q4_K_M".into()),
        Some(16) => Some("Q5_K_S".into()),
        Some(17) => Some("Q5_K_M".into()),
        Some(18) => Some("Q6_K".into()),
        Some(19) => Some("IQ2_XXS".into()),
        Some(20) => Some("IQ2_XS".into()),
        Some(21) => Some("Q2_K_S".into()),
        Some(22) => Some("IQ3_XS".into()),
        Some(23) => Some("IQ3_XXS".into()),
        Some(24) => Some("IQ1_S".into()),
        Some(25) => Some("IQ4_NL".into()),
        Some(26) => Some("IQ3_S".into()),
        Some(27) => Some("IQ3_M".into()),
        Some(28) => Some("IQ2_S".into()),
        Some(29) => Some("IQ2_M".into()),
        Some(30) => Some("IQ4_XS".into()),
        Some(31) => Some("IQ1_M".into()),
        Some(32) => Some("BF16".into()),
        _ => {
            let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            if fname.contains("bf16") { Some("BF16".into()) }
            else if fname.contains("f16") { Some("F16".into()) }
            else if fname.contains("f32") { Some("F32".into()) }
            else if fname.contains("q8_k") || fname.contains("q8k") { Some("Q8_K".into()) }
            else if fname.contains("q8_0") || fname.contains("q8o") { Some("Q8_0".into()) }
            else if fname.contains("q6_k") || fname.contains("q6k") { Some("Q6_K".into()) }
            else if fname.contains("q5_k") || fname.contains("q5k") { Some("Q5_K".into()) }
            else if fname.contains("q5_1") { Some("Q5_1".into()) }
            else if fname.contains("q5_0") { Some("Q5_0".into()) }
            else if fname.contains("q4_k") || fname.contains("q4k") { Some("Q4_K".into()) }
            else if fname.contains("q4_1") { Some("Q4_1".into()) }
            else if fname.contains("q4_0") { Some("Q4_0".into()) }
            else if fname.contains("q3_k") || fname.contains("q3k") { Some("Q3_K".into()) }
            else if fname.contains("q2_k") || fname.contains("q2k") { Some("Q2_K".into()) }
            else if fname.contains("iq4") { Some("IQ4".into()) }
            else if fname.contains("iq3") { Some("IQ3".into()) }
            else if fname.contains("iq2") { Some("IQ2".into()) }
            else if fname.contains("iq1") { Some("IQ1".into()) }
            else if fname.contains("mxfp4") { Some("MXFP4".into()) }
            else if fname.contains("mxfp6") { Some("MXFP6".into()) }
            else if fname.contains("mxfp8") { Some("MXFP8".into()) }
            else if fname.contains("gguf") { Some("原始精度".into()) }
            else { None }
        }
    };

    Ok((architecture, context_length, quant_type))
}

fn classify_gguf_file(path: &Path) -> &'static str {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    if name.contains("mmproj") || name.contains("clip") { "mmproj" }
    else if name.contains("imatrix") { "imatrix" }
    else { "model" }
}

// ── 模型扫描 ──────────────────────────────────────────────────────
#[tauri::command]
async fn scan_models(paths: Vec<String>, state: tauri::State<'_, AppState>) -> Result<Vec<ModelInfo>, String> {
    let mut models: Vec<ModelInfo> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut errors = Vec::new();
    let app_dir = get_app_dir(&state);
    let default_path = app_dir.join("models");

    let scan_paths: Vec<PathBuf> = if paths.is_empty() {
        vec![default_path]
    } else {
        paths.iter().map(PathBuf::from).collect()
    };

    for scan_root in &scan_paths {
        let root_str = scan_root.display().to_string();
        if !scan_root.exists() { errors.push(format!("{} 不存在", root_str)); continue; }
        if !scan_root.is_dir() { errors.push(format!("{} 不是目录", root_str)); continue; }

        let mut file_count = 0;
        for entry in walkdir::WalkDir::new(scan_root).max_depth(5).into_iter().flatten() {
            let path = entry.path();
            if !path.is_file() { continue; }
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            if ext != "gguf" { continue; }
            let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if fname.starts_with('.') { continue; }

            let name = fname.to_string();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let ftype = classify_gguf_file(path);

            let file_path = path.to_string_lossy().to_string();
            if !seen.insert(file_path.clone()) { continue; }

            let (architecture, context_length, quant_type) =
                parse_gguf_metadata(path).unwrap_or_else(|_| (None, None, None));

            file_count += 1;
            models.push(ModelInfo {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                path: file_path,
                size,
                architecture,
                context_length,
                quant_type,
                file_type: ftype.to_string(),
            });
        }
        if file_count == 0 { errors.push(format!("{} 中未找到 .gguf 模型文件", root_str)); }
    }

    if models.is_empty() && !errors.is_empty() {
        return Err(errors.join("; "));
    }

    let mut state_models = state.models.lock().unwrap();
    *state_models = models.clone();
    Ok(models)
}

#[tauri::command]
async fn get_models(state: tauri::State<'_, AppState>) -> Result<Vec<ModelInfo>, String> {
    Ok(state.models.lock().unwrap().clone())
}

// ── 模型操作 ──────────────────────────────────────────────────────
#[tauri::command]
async fn delete_model_file(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    // 尝试物理删除，文件不存在也算成功
    let _ = std::fs::remove_file(&path);
    let mut models = state.models.lock().unwrap();
    models.retain(|m| m.path != path);
    Ok(())
}

#[tauri::command]
async fn open_model_folder(path: String) -> Result<(), String> {
    let parent = Path::new(&path).parent().unwrap_or(Path::new("."));
    #[cfg(target_os = "windows")]
    { std::process::Command::new("explorer").arg(parent).spawn().map_err(|e| format!("{}", e))?; }
    #[cfg(not(target_os = "windows"))]
    { std::process::Command::new("open").arg(parent).spawn().map_err(|e| format!("{}", e))?; }
    Ok(())
}

#[tauri::command]
async fn read_gguf_metadata(path: String) -> Result<(Option<String>, Option<u32>, Option<String>), String> {
    parse_gguf_metadata(Path::new(&path))
}

// ── 引擎管理 ──────────────────────────────────────────────────────
#[tauri::command]
async fn scan_engines(paths: Vec<String>, state: tauri::State<'_, AppState>) -> Result<Vec<EngineInfo>, String> {
    let mut engines: Vec<EngineInfo> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let app_dir = get_app_dir(&state);

    // 1. Scan engines/ directory under app root
    let engines_dir = app_dir.join("engines");
    if engines_dir.exists() {
        for entry in std::fs::read_dir(&engines_dir).map_err(|e| format!("{}", e))?.flatten() {
            let dir = entry.path();
            if !dir.is_dir() { continue; }
            let exe = dir.join("llama-server.exe");
            if exe.exists() {
                let norm = dir.to_string_lossy().to_lowercase();
                if seen.insert(norm) {
                    if let Some(info) = build_engine_info(&dir, &exe, "本地") {
                        engines.push(info);
                    }
                }
            }
        }
    }

    // 2. Scan custom paths — recursively search subdirectories for llama-server.exe
    for p in &paths {
        let root = PathBuf::from(p);
        if !root.exists() || !root.is_dir() { continue; }

        // Check if the root itself contains llama-server.exe
        let direct_exe = root.join("llama-server.exe");
        if direct_exe.exists() {
            let norm = root.to_string_lossy().to_lowercase();
            if seen.insert(norm) {
                if let Some(info) = build_engine_info(&root, &direct_exe, "自定义") {
                    engines.push(info);
                }
            }
        }

        // Recursively scan 1-2 levels of subdirectories
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let sub = entry.path();
                if !sub.is_dir() { continue; }
                let exe = sub.join("llama-server.exe");
                if exe.exists() {
                    let norm = sub.to_string_lossy().to_lowercase();
                    if seen.insert(norm) {
                        if let Some(info) = build_engine_info(&sub, &exe, "自定义") {
                            engines.push(info);
                        }
                    }
                }
                // Check one more level deep
                if let Ok(sub_entries) = std::fs::read_dir(&sub) {
                    for se in sub_entries.flatten() {
                        let sub2 = se.path();
                        if !sub2.is_dir() { continue; }
                        let exe2 = sub2.join("llama-server.exe");
                        if exe2.exists() {
                            let norm = sub2.to_string_lossy().to_lowercase();
                            if seen.insert(norm) {
                                if let Some(info) = build_engine_info(&sub2, &exe2, "自定义") {
                                    engines.push(info);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut state_engines = state.engines.lock().unwrap();
    *state_engines = engines.clone();
    Ok(engines)
}

fn build_engine_info(dir: &Path, exe: &Path, _source: &str) -> Option<EngineInfo> {
    let name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("llama-server").to_string();
    let version = name.clone();
    let backend = detect_backend(dir);
    Some(EngineInfo {
        id: exe.parent().unwrap_or(Path::new(".")).to_string_lossy().to_string(),
        name: format!("{} ({})", name, backend),
        dir: dir.to_string_lossy().to_string(),
        exe: exe.to_string_lossy().to_string(),
        version,
        backend,
    })
}

fn detect_backend(dir: &Path) -> String {
    let entries: Vec<String> = std::fs::read_dir(dir).ok().map(|rd| {
        rd.flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect()
    }).unwrap_or_default();
    let joined = entries.join(" ");
    if joined.contains("roc") || joined.contains("hip") || joined.contains("amd") { "ROCm".into() }
    else if joined.contains("vulkan") || joined.contains("vk") { "Vulkan".into() }
    else if joined.contains("cuda") || joined.contains("cublas") { "CUDA".into() }
    else { "CPU".into() }
}

#[tauri::command]
async fn get_engines(state: tauri::State<'_, AppState>) -> Result<Vec<EngineInfo>, String> {
    Ok(state.engines.lock().unwrap().clone())
}

#[tauri::command]
async fn delete_engine(id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut engines = state.engines.lock().unwrap();
    engines.retain(|e| e.id != id);
    Ok(())
}

#[tauri::command]
async fn open_engine_folder(dir: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    { std::process::Command::new("explorer").arg(&dir).spawn().map_err(|e| format!("{}", e))?; }
    #[cfg(not(target_os = "windows"))]
    { std::process::Command::new("open").arg(&dir).spawn().map_err(|e| format!("{}", e))?; }
    Ok(())
}

// ── 生成 CLI 命令 ────────────────────────────────────────────────
fn generate_command(config: &InstanceConfig, engine_path: &str) -> Vec<String> {
    let exe = if engine_path.is_empty() { "llama-server".to_string() } else { engine_path.to_string() };
    let mut cmd = vec![exe, "-m".into(), config.model_path.clone()];
    let is_emb = config.embedding;

    if !config.alias.is_empty() { cmd.extend_from_slice(&["-a".into(), config.alias.clone()]); }
    if !is_emb {
        if !config.lora_path.is_empty() { cmd.extend_from_slice(&["--lora".into(), config.lora_path.clone()]); }
        if !config.mmproj_path.is_empty() { cmd.extend_from_slice(&["--mmproj".into(), config.mmproj_path.clone()]); }
        if !config.chat_template.is_empty() { cmd.extend_from_slice(&["--chat-template".into(), config.chat_template.clone()]); }
        if !config.reasoning_format.is_empty() { cmd.extend_from_slice(&["--reasoning-format".into(), config.reasoning_format.clone()]); }
        if !config.reasoning.is_empty() { cmd.extend_from_slice(&["--reasoning".into(), config.reasoning.clone()]); }
        if !config.reasoning_budget.is_empty() { cmd.extend_from_slice(&["--reasoning-budget".into(), config.reasoning_budget.clone()]); }
        if !config.reasoning_effort.is_empty() {
            let re = format!("{{\"reasoning_effort\": \"{}\"}}", config.reasoning_effort);
            cmd.extend_from_slice(&["--chat-template-kwargs".into(), re]);
        }
        if config.jinja { cmd.push("--jinja".into()); }
        if !config.grammar_file.is_empty() { cmd.extend_from_slice(&["--grammar-file".into(), config.grammar_file.clone()]); }
    }

    // Performance
    if !config.ctx_size_auto { cmd.extend_from_slice(&["-c".into(), config.ctx_size.to_string()]); }
    cmd.extend_from_slice(&["-ngl".into(), config.gpu_layers.to_string()]);
    if config.threads > 0 { cmd.extend_from_slice(&["-t".into(), config.threads.to_string()]); }
    if config.batch_size > 0 { cmd.extend_from_slice(&["-b".into(), config.batch_size.to_string()]); }
    if config.ubatch_size > 0 { cmd.extend_from_slice(&["-ub".into(), config.ubatch_size.to_string()]); }
    if config.parallel > 0 { cmd.extend_from_slice(&["-np".into(), config.parallel.to_string()]); }
    if config.cont_batching { cmd.push("-cb".into()); }
    if !config.cache_prompt { cmd.push("--no-cache-prompt".into()); }
    if config.threads_batch > 0 { cmd.extend_from_slice(&["--threads-batch".into(), config.threads_batch.to_string()]); }

    // Flash Attention
    if !is_emb {
        let fa = config.flash_attn.as_str();
        if fa != "auto" && !fa.is_empty() { cmd.extend_from_slice(&["-fa".into(), fa.to_string()]); }
    }

    if config.moe_cpu_layers > 0 { cmd.extend_from_slice(&["--n-cpu-moe".into(), config.moe_cpu_layers.to_string()]); }
    if config.mlock { cmd.push("--mlock".into()); }
    if config.no_mmap { cmd.push("--no-mmap".into()); }
    if config.numa { cmd.extend_from_slice(&["--numa".into(), "distribute".into()]); }
    if !config.cache_type_k.is_empty() { cmd.extend_from_slice(&["-ctk".into(), config.cache_type_k.clone()]); }
    if !config.cache_type_v.is_empty() { cmd.extend_from_slice(&["-ctv".into(), config.cache_type_v.clone()]); }

    // Speculative decoding
    if !is_emb {
        if !config.draft_model_path.is_empty() { cmd.extend_from_slice(&["-md".into(), config.draft_model_path.clone()]); }
        if config.draft_gpu_layers > 0 { cmd.extend_from_slice(&["-ngld".into(), config.draft_gpu_layers.to_string()]); }
        if config.draft_tokens > 0 { cmd.extend_from_slice(&["--spec-draft-n-max".into(), config.draft_tokens.to_string()]); }
        if config.spec_draft_n_min > 0 { cmd.extend_from_slice(&["--spec-draft-n-min".into(), config.spec_draft_n_min.to_string()]); }
        if !config.spec_type.is_empty() { cmd.extend_from_slice(&["--spec-type".into(), config.spec_type.clone()]); }
    }

    // Network
    cmd.extend_from_slice(&["--host".into(), config.host.clone(), "--port".into(), config.port.to_string()]);
    if !config.api_key.is_empty() { cmd.extend_from_slice(&["--api-key".into(), config.api_key.clone()]); }
    if !config.ssl_key_file.is_empty() { cmd.extend_from_slice(&["--ssl-key-file".into(), config.ssl_key_file.clone()]); }
    if !config.ssl_cert_file.is_empty() { cmd.extend_from_slice(&["--ssl-cert-file".into(), config.ssl_cert_file.clone()]); }
    if config.no_ui { cmd.push("--no-ui".into()); }

    // Embedding / Generation
    if config.embedding {
        cmd.push("--embedding".into());
        if !config.pooling.is_empty() { cmd.extend_from_slice(&["--pooling".into(), config.pooling.clone()]); }
        if config.reranking { cmd.push("--reranking".into()); }
    } else {
        if config.n_predict > 0 { cmd.extend_from_slice(&["-n".into(), config.n_predict.to_string()]); }
        else if config.n_predict == 0 {} else { cmd.extend_from_slice(&["-n".into(), "-1".into()]); }
        if config.ignore_eos { cmd.push("--ignore-eos".into()); }
        if !config.json_schema.is_empty() { cmd.extend_from_slice(&["--json-schema".into(), config.json_schema.clone()]); }
        if config.temp > 0.0 { cmd.extend_from_slice(&["--temp".into(), config.temp.to_string()]); }
        if config.top_k > 0 { cmd.extend_from_slice(&["--top-k".into(), config.top_k.to_string()]); }
        if config.top_p > 0.0 { cmd.extend_from_slice(&["--top-p".into(), config.top_p.to_string()]); }
        if config.repeat_penalty > 0.0 { cmd.extend_from_slice(&["--repeat-penalty".into(), config.repeat_penalty.to_string()]); }
        if config.seed >= 0 { cmd.extend_from_slice(&["--seed".into(), config.seed.to_string()]); }
        if config.min_p > 0.0 { cmd.extend_from_slice(&["--min-p".into(), config.min_p.to_string()]); }
        if config.presence_penalty > 0.0 { cmd.extend_from_slice(&["--presence-penalty".into(), config.presence_penalty.to_string()]); }
        if config.frequency_penalty > 0.0 { cmd.extend_from_slice(&["--frequency-penalty".into(), config.frequency_penalty.to_string()]); }
        if config.repeat_last_n > 0 { cmd.extend_from_slice(&["--repeat-last-n".into(), config.repeat_last_n.to_string()]); }

        // Advanced sampling
        if config.mirostat > 0 { cmd.extend_from_slice(&["--mirostat".into(), config.mirostat.to_string()]); }
        if config.mirostat_lr > 0.0 { cmd.extend_from_slice(&["--mirostat-lr".into(), config.mirostat_lr.to_string()]); }
        if config.mirostat_ent > 0.0 { cmd.extend_from_slice(&["--mirostat-ent".into(), config.mirostat_ent.to_string()]); }
        if config.xtc_probability > 0.0 { cmd.extend_from_slice(&["--xtc-probability".into(), config.xtc_probability.to_string()]); }
        if config.xtc_threshold > 0.0 { cmd.extend_from_slice(&["--xtc-threshold".into(), config.xtc_threshold.to_string()]); }
        if config.dynatemp_range > 0.0 { cmd.extend_from_slice(&["--dynatemp-range".into(), config.dynatemp_range.to_string()]); }
        if config.dynatemp_exp > 0.0 { cmd.extend_from_slice(&["--dynatemp-exp".into(), config.dynatemp_exp.to_string()]); }
        if config.typical_p < 1.0 && config.typical_p > 0.0 { cmd.extend_from_slice(&["--typical-p".into(), config.typical_p.to_string()]); }
        if config.dry_multiplier > 0.0 { cmd.extend_from_slice(&["--dry-multiplier".into(), config.dry_multiplier.to_string()]); }
        if config.dry_base > 0.0 { cmd.extend_from_slice(&["--dry-base".into(), config.dry_base.to_string()]); }
        if config.dry_allowed_length > 0 { cmd.extend_from_slice(&["--dry-allowed-length".into(), config.dry_allowed_length.to_string()]); }
        if config.dry_penalty_last_n > 0 { cmd.extend_from_slice(&["--dry-penalty-last-n".into(), config.dry_penalty_last_n.to_string()]); }
        if !config.dry_sequence_breaker.is_empty() { cmd.extend_from_slice(&["--dry-sequence-breaker".into(), config.dry_sequence_breaker.clone()]); }
    }

    // Server reliability
    if config.timeout > 0 { cmd.extend_from_slice(&["-to".into(), config.timeout.to_string()]); }
    if config.sleep_idle >= 0 { cmd.extend_from_slice(&["--sleep-idle-seconds".into(), config.sleep_idle.to_string()]); }
    if config.context_shift { cmd.push("--context-shift".into()); }
    if config.verbose { cmd.push("-v".into()); }

    // Custom args
    for arg in &config.custom_args {
        if !arg.is_empty() {
            cmd.extend(arg.split_whitespace().map(String::from));
        }
    }

    cmd
}

#[tauri::command]
async fn generate_server_command(
    config: InstanceConfig,
    engine_exe: String,
) -> Result<Vec<String>, String> {
    Ok(generate_command(&config, &engine_exe))
}

#[tauri::command]
async fn start_server(
    instance_id: String,
    config: InstanceConfig,
    engine_exe: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    {
        let running = state.running.lock().unwrap();
        if running.contains_key(&instance_id) {
            return Err("该实例已在运行中".to_string());
        }
    }

    let cmd = generate_command(&config, &engine_exe);
    let cmd_str = cmd.join(" ");

    let mut child = {
        let mut c = Command::new(&cmd[0]);
        c.args(&cmd[1..]).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        { use std::os::windows::process::CommandExt; c.creation_flags(0x08000000); }
        c.spawn().map_err(|e| format!("启动服务器失败: {}\n命令: {}", e, cmd_str))?
    };

    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    state.running.lock().unwrap().insert(instance_id.clone(), RunningInstance {
        instance_id: instance_id.clone(),
        pid,
        port: config.port,
        host: config.host.clone(),
    });

    app.emit("server-started", serde_json::json!({
        "instanceId": instance_id,
        "pid": pid,
        "port": config.port,
        "command": cmd_str,
    })).ok();

    // 监控线程：读 stdout/stderr + 1 秒快速检测进程是否存活 + 等待退出
    let id = instance_id.clone();
    let app_clone = app.clone();
    std::thread::spawn(move || {
        let id1 = id.clone();
        let app1 = app_clone.clone();
        let stdout_handle = stdout.map(|s| {
            std::thread::spawn(move || {
                let reader = BufReader::new(s);
                for line in reader.lines() {
                    if let Ok(text) = line {
                        let _ = app1.emit("server-log", serde_json::json!({
                            "instanceId": id1, "text": format!("{}\n", text),
                        }));
                    }
                }
            })
        });

        let id2 = id.clone();
        let app2 = app_clone.clone();
        let stderr_handle = stderr.map(|s| {
            std::thread::spawn(move || {
                let reader = BufReader::new(s);
                for line in reader.lines() {
                    if let Ok(text) = line {
                        let _ = app2.emit("server-log", serde_json::json!({
                            "instanceId": id2, "text": format!("{}\n", text),
                        }));
                    }
                }
            })
        });

        // 1 秒后快速检测：进程是否已经死了（参数错误等立刻退出）
        std::thread::sleep(std::time::Duration::from_secs(1));
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => {
                // 清理 running 状态（仅当 PID 匹配自己）
                let st = app_clone.state::<AppState>();
                { let mut r = st.running.lock().unwrap();
                  if r.get(&id).map(|ri| ri.pid == pid).unwrap_or(false) { r.remove(&id); } }
                let _ = app_clone.emit("server-error", serde_json::json!({
                    "instanceId": id,
                    "error": format!("进程启动后立即退出 (exit code: {:?})", status.code()),
                }));
                return;
            }
            Ok(None) => {} // still running — good
            _ => {} // error checking — let health check handle it
        }

        // 等待进程退出（正常情况），只清理，发停止事件
        let _ = child.wait();
        let st2 = app_clone.state::<AppState>();
        { let mut r = st2.running.lock().unwrap();
          if r.get(&id).map(|ri| ri.pid == pid).unwrap_or(false) {
            r.remove(&id);
            let _ = app_clone.emit("server-stopped", serde_json::json!({ "instanceId": id }));
          } }
        if let Some(h) = stdout_handle { h.join().ok(); }
        if let Some(h) = stderr_handle { h.join().ok(); }
    });

    // 启动健康检查（独立于监控线程），带上 PID 防止旧线程误判
    let id = instance_id.clone();
    let app_for_health = app.clone();
    let host = if config.host == "0.0.0.0" { "localhost".to_string() } else { config.host.clone() };
    let port = config.port;
    std::thread::spawn(move || {
        health_check_loop(&id, &host, port, pid, app_for_health);
    });

    Ok(())
}

fn health_check_loop(instance_id: &str, host: &str, port: u16, expected_pid: u32, app: tauri::AppHandle) {
    let url = format!("http://{}:{}/health", host, port);
    let client = reqwest::blocking::Client::new();

    // 检查是否还在 running 状态，且 PID 匹配（防止旧线程误操作新实例）
    let is_my_instance = || {
        let st = app.state::<AppState>();
        let guard = st.running.lock().unwrap();
        guard.get(instance_id).map(|r| r.pid == expected_pid).unwrap_or(false)
    };

    // Phase 1: quick retries, up to 30 attempts (30s), give model time to load
    for _ in 0..30 {
        if !is_my_instance() { return; }
        if let Ok(resp) = client.get(&url).timeout(std::time::Duration::from_secs(2)).send() {
            if resp.status().is_success() {
                let _ = app.emit("health-status", serde_json::json!({
                    "instanceId": instance_id, "status": "ok",
                }));
                // Phase 2: indefinite monitoring, only exit when stopped by user
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    if !is_my_instance() { return; }
                    if let Ok(resp) = client.get(&url).timeout(std::time::Duration::from_secs(5)).send() {
                        if !resp.status().is_success() {
                            // 暂时不可用，继续等
                            std::thread::sleep(std::time::Duration::from_secs(3));
                        }
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    // Phase 1 exhausted (30s) but model still loading → Phase 2: extended wait every 3s
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3));
        if !is_my_instance() { return; }
        if let Ok(resp) = client.get(&url).timeout(std::time::Duration::from_secs(5)).send() {
            if resp.status().is_success() {
                let _ = app.emit("health-status", serde_json::json!({
                    "instanceId": instance_id, "status": "ok",
                }));
                // Back to Phase 2 monitoring
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    if !is_my_instance() { return; }
                    if let Ok(resp) = client.get(&url).timeout(std::time::Duration::from_secs(5)).send() {
                        if !resp.status().is_success() {
                            std::thread::sleep(std::time::Duration::from_secs(3));
                        }
                    }
                }
            }
        }
    }
}

#[tauri::command]
async fn stop_server(
    instance_id: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    if let Some(ri) = state.running.lock().unwrap().remove(&instance_id) {
        #[cfg(windows)]
        {
            let _ = std::os::windows::process::CommandExt::creation_flags(
                &mut Command::new("taskkill"), 0x08000000)
                .args(["/F", "/PID", &ri.pid.to_string()])
                .output();
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = Command::new("kill").arg(ri.pid.to_string()).output();
        }
    }

    app.emit("server-stopped", serde_json::json!({
        "instanceId": instance_id,
    })).ok();

    Ok(())
}

#[tauri::command]
async fn open_browser(host: String, port: u16) -> Result<(), String> {
    let url = format!("http://{}:{}", host, port);
    #[cfg(windows)]
    { std::os::windows::process::CommandExt::creation_flags(
        &mut std::process::Command::new("cmd"), 0x08000000)
        .args(["/c", "start", &url]).spawn().map_err(|e| format!("{}", e))?; }
    #[cfg(not(target_os = "windows"))]
    { std::process::Command::new("open").arg(&url).spawn().map_err(|e| format!("{}", e))?; }
    Ok(())
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct GlobalConfig {
    instances: HashMap<String, InstanceConfig>,
    model_dirs: Vec<String>,
    engine_dirs: Vec<String>,
    default_engine_id: String,
    running: HashMap<String, RunningInstance>,
    instance_order: Vec<String>,
    last_tab: String,
    dark_mode: bool,
}
// ── 配置持久化 ────────────────────────────────────────────────────
#[tauri::command]
async fn save_config(
    instances: HashMap<String, InstanceConfig>,
    model_dirs: Vec<String>,
    engine_dirs: Vec<String>,
    default_engine_id: String,
    instance_order: Vec<String>,
    last_tab: String,
    dark_mode: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut stored = state.instances.lock().unwrap();
    *stored = instances.clone();
    let running_snapshot = state.running.lock().unwrap().clone();
    let global = GlobalConfig { instances, model_dirs, engine_dirs, default_engine_id, running: running_snapshot, instance_order, last_tab, dark_mode };
    let config_dir = state.config_dir.lock().unwrap().clone();
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("{}", e))?;
    let path = config_dir.join("instances.json");
    let json = serde_json::to_string_pretty(&global).map_err(|e| format!("{}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("保存失败: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn load_config(state: tauri::State<'_, AppState>, app: tauri::AppHandle) -> Result<GlobalConfig, String> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let path = config_dir.join("instances.json");
    if !path.exists() {
        return Ok(GlobalConfig { instances: HashMap::new(), model_dirs: vec![], engine_dirs: vec![], default_engine_id: String::new(), running: HashMap::new(), instance_order: vec![], last_tab: "model-repo".into(), dark_mode: true });
    }
    let json = std::fs::read_to_string(&path).map_err(|e| format!("{}", e))?;
    let mut global: GlobalConfig = serde_json::from_str(&json).map_err(|e| format!("解析配置失败: {}", e))?;
    let mut stored = state.instances.lock().unwrap();
    *stored = global.instances.clone();

    // 恢复仍在运行的后台进程
    let mut restored = HashMap::new();
    for (id, ri) in &global.running {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let alive = std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {}", ri.pid), "/NH"])
                .creation_flags(0x08000000)
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains(&ri.pid.to_string()))
                .unwrap_or(false);
            if alive { restored.insert(id.clone(), ri.clone()); }
        }
        #[cfg(not(windows))]
        {
            // Linux/macOS: 用 kill -0 检测
            let alive = std::process::Command::new("kill").arg("-0").arg(ri.pid.to_string()).status().map(|s| s.success()).unwrap_or(false);
            if alive { restored.insert(id.clone(), ri.clone()); }
        }
    }
    // 只在状态中保留仍然活着的进程
    let mut running = state.running.lock().unwrap();
    *running = restored;

    // 清理配置中不再运行的进程记录并返回
    global.running = running.clone();
    // 为恢复的运行中实例启动健康检查
    for (id, ri) in &*running {
        let id = id.clone();
        let host = if ri.host == "0.0.0.0" { "localhost".to_string() } else { ri.host.clone() };
        let port = ri.port;
        let pid = ri.pid;
        let app = app.clone();
        std::thread::spawn(move || {
            health_check_loop(&id, &host, port, pid, app);
        });
    }
    Ok(global)
}

// ── ModelScope 浏览 ──────────────────────────────────────────────
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct MsFileEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub file_type: String,
}

#[tauri::command]
async fn browse_modelscope(repo_id: String) -> Result<Vec<MsFileEntry>, String> {
    let url = format!("https://www.modelscope.cn/api/v1/models/{}/repo/files?Recursive=true", repo_id);
    let client = reqwest::Client::new();
    let resp = client.get(&url).send().await.map_err(|e| format!("网络错误: {}", e))?;
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("{}", e))?;

    if !body.get("Success").and_then(|v| v.as_bool()).unwrap_or(false) {
        let msg = body.get("Message").and_then(|v| v.as_str()).unwrap_or("未知错误");
        return Err(msg.to_string());
    }

    let empty_vec = vec![];
    let files = body["Data"]["Files"].as_array().unwrap_or(&empty_vec);
    let mut result: Vec<MsFileEntry> = files.iter().filter_map(|f| {
        if f.get("Type")?.as_str()? != "blob" { return None; }
        let name = f.get("Name")?.as_str()?.to_string();
        if !name.ends_with(".gguf") && !name.ends_with(".txt") { return None; }
        Some(MsFileEntry {
            file_type: classify_gguf_file(Path::new(&name)).to_string(),
            name,
            path: f.get("Path")?.as_str()?.to_string(),
            size: f.get("Size")?.as_u64().unwrap_or(0),
        })
    }).collect();

    result.sort_by_key(|e| {
        match e.file_type.as_str() {
            "mmproj" => 0,
            "model" => 1,
            "imatrix" => 2,
            _ => 9,
        }
    });

    Ok(result)
}

// ── ModelScope 并行下载 ─────────────────────────────────────────
#[tauri::command]
async fn download_modelscope_files(
    repo_id: String,
    files: Vec<MsFileEntry>,
    save_dir: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let save_path = if Path::new(&save_dir).is_absolute() {
        PathBuf::from(&save_dir)
    } else {
        let app_handle = app.clone();
        let managed = app_handle.state::<AppState>();
        let config_dir = managed.config_dir.lock().unwrap();
        config_dir.parent().unwrap_or(Path::new(".")).to_path_buf().join(&save_dir)
    };
    // Create subfolder from repo ID: "unsloth/Qwen-GGUF" → "unsloth/Qwen-GGUF"
    let save_path = save_path.join(repo_id.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
    std::fs::create_dir_all(&save_path).map_err(|e| format!("创建目录失败: {}", e))?;
    app.state::<AppState>().cancel_flags.lock().unwrap().clear();

    let total_files = files.len();
    let mut handles = Vec::new();

    for file in files {
        let url = format!(
            "https://modelscope.cn/models/{}/resolve/master/{}",
            repo_id, file.path
        );
        let save_path = save_path.clone();
        let app = app.clone();
        let file_name = file.name.clone();
        let file_size = file.size;

        let handle = tokio::spawn({
            let app = app.clone();
            async move {
            let shared = app.state::<AppState>();
            if shared.cancel_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
                if !shared.pause_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
                    let _ = app.emit("download-cancelled", serde_json::json!({ "fileName": file_name }));
                }
                return;
            }

            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()
                .unwrap_or_default();

            // 检查是否存在断点文件
            let dest = save_path.join(&file_name);
            let resume_from = dest.metadata().map(|m| m.len()).unwrap_or(0);

            let mut req = client.get(&url).header("User-Agent", "Mozilla/5.0");
            if resume_from > 0 {
                req = req.header("Range", format!("bytes={}-", resume_from));
            }

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    let _ = app.emit("download-error", serde_json::json!({
                        "fileName": file_name, "error": e.to_string()
                    }));
                    return;
                }
            };
            if !resp.status().is_success() && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                let _ = app.emit("download-error", serde_json::json!({
                    "fileName": file_name, "error": format!("HTTP {}", resp.status())
                }));
                return;
            }

            let total = if resume_from > 0 {
                resp.content_length().unwrap_or(0) + resume_from
            } else {
                resp.content_length().unwrap_or(file_size)
            };
            let mut downloaded = resume_from;
            let dl_start = std::time::Instant::now();
            let mut last_emit = dl_start;
            let mut last_bytes = resume_from;

            use std::io::Write;
            // 以追加模式打开文件
            let mut file = std::fs::OpenOptions::new()
                .create(true).append(resume_from == 0).write(true)
                .open(&dest).unwrap();

            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                if shared.cancel_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
                    let _ = app.emit("download-cancelled", serde_json::json!({ "fileName": file_name }));
                    return;
                }
                match chunk {
                    Ok(bytes) => {
                        let len = bytes.len() as u64;
                        file.write_all(&bytes).unwrap();
                        downloaded += len;
                        let now = std::time::Instant::now();
                        let elapsed = now.duration_since(last_emit).as_secs_f64().max(0.1);
                        let speed = if elapsed > 0.0 { (downloaded - last_bytes) as f64 / elapsed } else { 0.0 };
                        last_emit = now;
                        last_bytes = downloaded;
                        let _ = app.emit("download-progress", serde_json::json!({
                            "fileName": file_name, "downloaded": downloaded,
                            "total": total, "totalFiles": total_files,
                            "speed": speed,
                        }));
                    }
                    Err(e) => {
                        let _ = app.emit("download-error", serde_json::json!({
                            "fileName": file_name, "error": e.to_string()
                        }));
                        return;
                    }
                }
            }
            let _ = app.emit("download-complete", serde_json::json!({
                "fileName": file_name, "path": dest.to_string_lossy(),
            }));
            }
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }
    Ok(())
}

// ── 下载取消（按文件名） ─────────────────────────────────────────
#[tauri::command]
async fn cancel_file_download(file_name: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.cancel_flags.lock().unwrap().insert(file_name, true);
    Ok(())
}

#[tauri::command]
async fn pause_file_download(file_name: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.pause_flags.lock().unwrap().insert(file_name.clone(), true);
    state.cancel_flags.lock().unwrap().insert(file_name, true);
    Ok(())
}

#[tauri::command]
async fn cancel_and_cleanup_download(file_name: String, file_path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.cancel_flags.lock().unwrap().insert(file_name.clone(), true);
    state.pause_flags.lock().unwrap().remove(&file_name);
    let _ = std::fs::remove_file(&file_path);
    Ok(())
}

// ── 端口检测 ─────────────────────────────────────────────────────
#[tauri::command]
async fn check_port(port: u16) -> Result<bool, String> {
    use std::net::TcpListener;
    match TcpListener::bind(format!("127.0.0.1:{}", port)) {
        Ok(_) => Ok(true), // 端口空闲
        Err(_) => Ok(false), // 端口被占用
    }
}

// ── 窗口状态 ─────────────────────────────────────────────────────
#[derive(serde::Serialize, serde::Deserialize)]
struct WindowState { x: i32, y: i32, width: u32, height: u32 }

#[tauri::command]
fn save_window_state(x: i32, y: i32, width: u32, height: u32, state: tauri::State<'_, AppState>) {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let _ = std::fs::create_dir_all(&config_dir);
    let ws = WindowState { x, y, width, height };
    if let Ok(json) = serde_json::to_string(&ws) {
        let _ = std::fs::write(config_dir.join("window_state.json"), json);
    }
}

#[tauri::command]
fn load_window_state(state: tauri::State<'_, AppState>) -> Option<WindowState> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let path = config_dir.join("window_state.json");
    if path.exists() {
        std::fs::read_to_string(&path).ok()
            .and_then(|s| serde_json::from_str::<WindowState>(&s).ok())
    } else { None }
}
#[tauri::command]
async fn test_connection(host: String, port: u16) -> Result<String, String> {
    let url = format!("http://{}:{}/health", if host == "0.0.0.0" { "localhost" } else { &host }, port);
    let client = reqwest::Client::new();
    match client.get(&url).timeout(std::time::Duration::from_secs(3)).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                Ok("✓ 连接成功，服务正常".into())
            } else {
                Err(format!("服务返回 HTTP {}", resp.status()))
            }
        }
        Err(e) => Err(format!("无法连接: {}", e))
    }
}
fn get_app_dir(_state: &AppState) -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."))
        .parent().unwrap_or(Path::new(".")).to_path_buf()
}

fn main() {
    let default_models: Vec<ModelInfo> = vec![];
    let default_engines: Vec<EngineInfo> = vec![];

    let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let exe_dir = exe_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let config_dir = exe_dir.join("configs");

    let instances = {
        let path = config_dir.join("instances.json");
        if path.exists() {
            std::fs::read_to_string(&path).ok()
                .and_then(|s| serde_json::from_str::<HashMap<String, InstanceConfig>>(&s).ok())
                .unwrap_or_default()
        } else {
            HashMap::new()
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 保存窗口状态
                if let Ok(pos) = window.outer_position() {
                    if let Ok(size) = window.outer_size() {
                        let ws = WindowState { x: pos.x, y: pos.y, width: size.width, height: size.height };
                        if let Some(s) = window.try_state::<AppState>() {
                            let config_dir = s.config_dir.lock().unwrap().clone();
                            let _ = std::fs::create_dir_all(&config_dir);
                            if let Ok(json) = serde_json::to_string(&ws) {
                                let _ = std::fs::write(config_dir.join("window_state.json"), json);
                            }
                        }
                    }
                }
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
            use tauri::menu::{MenuBuilder, MenuItemBuilder};

            let show = MenuItemBuilder::with_id("show", "显示窗口")
                .build(app.handle())?;
            let quit = MenuItemBuilder::with_id("quit", "退出")
                .build(app.handle())?;
            let menu = MenuBuilder::new(app.handle())
                .item(&show)
                .item(&quit)
                .build()?;

            if let Some(icon) = app.default_window_icon().cloned() {
                TrayIconBuilder::new()
                    .icon(icon)
                    .menu(&menu)
                    .on_menu_event(|app, event| {
                    match event.id().as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => { app.exit(0); }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app.handle())?;
            }

            // 关闭窗口时隐藏到托盘
            // (handled by Builder::on_window_event below)
            Ok(())
        })
        .manage(AppState {
            models: Mutex::new(default_models),
            engines: Mutex::new(default_engines),
            instances: Mutex::new(instances),
            running: Mutex::new(HashMap::new()),
            config_dir: Mutex::new(config_dir),
            cancel_flags: Mutex::new(HashMap::new()),
            pause_flags: Mutex::new(HashMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            scan_models,
            get_models,
            delete_model_file,
            open_model_folder,
            read_gguf_metadata,
            scan_engines,
            get_engines,
            delete_engine,
            open_engine_folder,
            generate_server_command,
            start_server,
            stop_server,
            open_browser,
            save_config,
            load_config,
            browse_modelscope,
            download_modelscope_files,
            cancel_file_download,
            pause_file_download,
            cancel_and_cleanup_download,
            test_connection,
            check_port,
            save_window_state,
            load_window_state,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
