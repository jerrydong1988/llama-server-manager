//! Gufo's managed LLM launch contract. No llama.cpp argument projection applies here.
use crate::error::{AppError, AppResult};
use crate::models::InstanceConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    #[default]
    LlamaCpp,
    Gufo,
}

impl EngineKind {
    pub fn for_executable(path: &str) -> Self {
        if Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("gufo.exe") || s == "gufo")
        {
            Self::Gufo
        } else {
            Self::LlamaCpp
        }
    }
}

/// Gufo's Windows socket reuse can otherwise bind an already occupied endpoint.
/// Check immediately before spawning, under the runtime's lifecycle lock.
pub fn ensure_port_available(config: &InstanceConfig) -> AppResult<()> {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], config.port));
    if std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(300)).is_ok()
        || std::net::TcpListener::bind(address).is_err()
    {
        return Err(AppError::new(
            "GUFO_PORT_IN_USE",
            format!("Gufo 端口 {} 已被占用。请选择其他端口；不会停止占用该端口的程序。", config.port),
            false,
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GufoConfig {
    pub model_family: String,
    pub speculative: String,
    pub mtp_model: String,
    pub dflash_model: String,
    pub draft_tokens: u32,
    pub sessions: u32,
    pub think: String,
    pub reasoning_effort: String,
    pub preserve_thinking: bool,
    pub cache_ram_mode: String,
    pub cache_ram_gib: u32,
    pub cache_disk: bool,
    pub cache_disk_gib: u32,
    pub startup_timeout_seconds: u32,
}

impl Default for GufoConfig {
    fn default() -> Self {
        Self {
            model_family: "qwen38_flash_next".into(),
            speculative: "off".into(),
            mtp_model: String::new(),
            dflash_model: String::new(),
            draft_tokens: 7,
            sessions: 1,
            think: "on".into(),
            reasoning_effort: "auto".into(),
            preserve_thinking: true,
            cache_ram_mode: "auto".into(),
            cache_ram_gib: 4,
            cache_disk: false,
            cache_disk_gib: 10,
            startup_timeout_seconds: 600,
        }
    }
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new("GUFO_CONFIG_INVALID", message, false)
}

pub fn is_gufo(config: &InstanceConfig) -> bool {
    config.engine_kind == EngineKind::Gufo
}

pub fn instance_directory(id: &str) -> PathBuf {
    crate::utils::get_data_dir().join("gufo").join(id)
}

fn check_gguf(value: &str, label: &str, shards: bool) -> AppResult<()> {
    use std::io::Read;
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(invalid(format!("{label}: select an absolute GGUF path")));
    }
    let mut file = std::fs::File::open(path).map_err(|e| invalid(format!("{label}: {e}")))?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic)
        .map_err(|e| invalid(format!("{label}: {e}")))?;
    if magic != *b"GGUF" {
        return Err(invalid(format!("{label}: not a GGUF file")));
    }
    if shards {
        let name = path.file_name().unwrap().to_string_lossy();
        if let Some((stem, total)) = name
            .strip_suffix(".gguf")
            .and_then(|s| s.rsplit_once("-of-"))
        {
            let (prefix, index) = stem
                .rsplit_once('-')
                .ok_or_else(|| invalid("Invalid shard name"))?;
            let total: u32 = total.parse().map_err(|_| invalid("Invalid shard count"))?;
            if index != "00001" || !(1..=1024).contains(&total) {
                return Err(invalid("Select the first model shard"));
            }
            for part in 1..=total {
                let shard = path.with_file_name(format!("{prefix}-{part:05}-of-{total:05}.gguf"));
                check_gguf(&shard.to_string_lossy(), "Model shard", false)?;
            }
        }
    }
    Ok(())
}

pub fn prepare(
    mut config: InstanceConfig,
    executable: &str,
) -> AppResult<(InstanceConfig, Vec<String>)> {
    crate::commands::server::validate_instance_id(&config.id)?;
    if EngineKind::for_executable(executable) != config.engine_kind {
        return Err(invalid(
            "Engine kind does not match the selected executable",
        ));
    }
    if !cfg!(windows) {
        return Err(invalid(
            "Gufo managed instances are currently qualified on Windows only",
        ));
    }
    if config.launch_mode != "managed"
        || !config.custom_args.is_empty()
        || config.embedding
        || config.reranking
        || config.kv_checkpoint.enabled
        || !config.rpc_servers.is_empty()
        || !config.slot_save_path.is_empty()
        || !config.api_key_file.is_empty()
        || !config.ssl_key_file.is_empty()
        || !config.ssl_cert_file.is_empty()
        || !config.path_prefix.is_empty()
        || !config.api_prefix.is_empty()
        || !config.lora_path.is_empty()
        || !config.lookup_cache_dynamic.is_empty()
        || !config.lookup_cache_static.is_empty()
    {
        return Err(invalid(
            "Gufo phase 1 supports managed local LLM instances, without llama.cpp extensions",
        ));
    }
    const FIELDS: &[&str] = &[
        "engine_kind",
        "gufo",
        "engine_id",
        "id",
        "name",
        "model_path",
        "alias",
        "mmproj_path",
        "host",
        "port",
        "api_key",
        "ctx_size",
        "ctx_size_auto",
        "n_predict",
        "temp",
        "top_k",
        "top_p",
        "min_p",
        "seed",
        "repeat_penalty",
        "repeat_last_n",
        "frequency_penalty",
        "presence_penalty",
    ];
    if config
        .explicit_overrides
        .as_ref()
        .is_some_and(|keys| keys.iter().any(|k| !FIELDS.contains(&k.as_str())))
    {
        return Err(invalid(
            "Remove llama.cpp parameter overrides before selecting Gufo",
        ));
    }
    let g = &config.gufo;
    if !matches!(g.model_family.as_str(), "qwen38_flash_next" | "qwen38_27b")
        || !matches!(g.speculative.as_str(), "off" | "mtp" | "dflash2")
        || (g.speculative == "mtp" && g.model_family != "qwen38_flash_next")
        || (g.speculative == "dflash2" && g.model_family != "qwen38_27b")
    {
        return Err(invalid(
            "Select a supported model family and matching speculative mode",
        ));
    }
    if !(1..=7).contains(&g.draft_tokens)
        || g.sessions != 1
        || !(60..=3600).contains(&g.startup_timeout_seconds)
        || !(1..=65536).contains(&g.cache_disk_gib)
        || !matches!(g.cache_ram_mode.as_str(), "auto" | "off" | "limit")
        || (g.cache_ram_mode == "limit" && !(1..=65536).contains(&g.cache_ram_gib))
        || !matches!(g.think.as_str(), "auto" | "on" | "off")
        || !matches!(
            g.reasoning_effort.as_str(),
            "auto" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        )
    {
        return Err(invalid(
            "Invalid Gufo session, cache, thinking or startup settings",
        ));
    }
    if config.host != "127.0.0.1"
        || config.port == 0
        || config.ctx_size == 0
        || config.ctx_size_auto
        || config.n_predict == 0
        || config.n_predict < -1
        || !config.temp.is_finite()
        || !(0.0..=2.0).contains(&config.temp)
        || !config.top_p.is_finite()
        || !(0.0..=1.0).contains(&config.top_p)
        || !config.min_p.is_finite()
        || !(0.0..=1.0).contains(&config.min_p)
        || !config.repeat_penalty.is_finite()
        || config.repeat_penalty <= 0.0
        || !config.frequency_penalty.is_finite()
        || !(-2.0..=2.0).contains(&config.frequency_penalty)
        || !config.presence_penalty.is_finite()
        || !(-2.0..=2.0).contains(&config.presence_penalty)
        || config.repeat_last_n < 0
        || config.seed < -1
    {
        return Err(invalid(
            "Gufo requires loopback, an explicit positive context, and valid sampling limits",
        ));
    }
    if config.alias.trim().is_empty() {
        config.alias = config.id.clone();
    }
    check_gguf(&config.model_path, "Model", true)?;
    if !config.mmproj_path.is_empty() {
        check_gguf(&config.mmproj_path, "Projector", false)?;
    }
    if g.speculative == "mtp" {
        check_gguf(&g.mtp_model, "MTP sidecar", false)?;
    }
    if g.speculative == "dflash2" {
        check_gguf(&g.dflash_model, "DFlash2 draft", false)?;
    }
    let mut command = vec![
        executable.into(),
        "serve".into(),
        "--host".into(),
        config.host.clone(),
        "--port".into(),
        config.port.to_string(),
        "--sessions".into(),
        "1".into(),
    ];
    if !config.api_key.is_empty() {
        command.extend(["--api-key".into(), config.api_key.clone()]);
    }
    command.extend(["llm".into(), "--model".into(), config.model_path.clone()]);
    for (flag, value) in [
        ("--context", config.ctx_size.to_string()),
        ("--max-tokens", config.n_predict.to_string()),
        ("--served-model-name", config.alias.clone()),
        ("--think", g.think.clone()),
        (
            "--preserve-thinking",
            if g.preserve_thinking { "on" } else { "off" }.into(),
        ),
        ("--temperature", config.temp.to_string()),
        ("--top-k", config.top_k.to_string()),
        ("--top-p", config.top_p.to_string()),
        ("--min-p", config.min_p.to_string()),
        ("--seed", config.seed.to_string()),
        ("--repeat-penalty", config.repeat_penalty.to_string()),
        ("--repeat-last-n", config.repeat_last_n.to_string()),
        ("--frequency-penalty", config.frequency_penalty.to_string()),
        ("--presence-penalty", config.presence_penalty.to_string()),
        ("--speculative", g.speculative.clone()),
    ] {
        command.extend([flag.into(), value]);
    }
    if g.think != "off" && g.reasoning_effort != "auto" {
        command.extend(["--reasoning-effort".into(), g.reasoning_effort.clone()]);
    }
    if !config.mmproj_path.is_empty() {
        command.extend(["--mmproj".into(), config.mmproj_path.clone()]);
    }
    if g.speculative == "mtp" {
        command.extend([
            "--mtp-model".into(),
            g.mtp_model.clone(),
            "--draft-tokens".into(),
            g.draft_tokens.to_string(),
        ]);
    }
    if g.speculative == "dflash2" {
        command.extend([
            "--dflash-model".into(),
            g.dflash_model.clone(),
            "--draft-tokens".into(),
            g.draft_tokens.to_string(),
        ]);
    }
    let ram = match g.cache_ram_mode.as_str() {
        "off" => "off".into(),
        "limit" => (u64::from(g.cache_ram_gib) << 30).to_string(),
        _ => "0".into(),
    };
    command.extend(["--cache-ram-bytes".into(), ram]);
    if g.cache_disk {
        command.extend([
            "--cache-disk".into(),
            instance_directory(&config.id)
                .join("snapshots")
                .to_string_lossy()
                .into_owned(),
            "--cache-disk-bytes".into(),
            (u64::from(g.cache_disk_gib) << 30).to_string(),
        ]);
    }
    config.parallel = 1;
    Ok((config, command))
}

/// Isolate identity/GPU caches as well as snapshot files, without changing parent environment.
pub fn environment(config: &InstanceConfig) -> std::io::Result<Vec<(String, String)>> {
    if !is_gufo(config) {
        return Ok(Vec::new());
    }
    let root = instance_directory(&config.id);
    let local = root.join("local");
    let temp = root.join("temp");
    std::fs::create_dir_all(&local)?;
    std::fs::create_dir_all(&temp)?;
    Ok(vec![
        ("LOCALAPPDATA".into(), local.to_string_lossy().into_owned()),
        (
            "XDG_CACHE_HOME".into(),
            local.to_string_lossy().into_owned(),
        ),
        ("TEMP".into(), temp.to_string_lossy().into_owned()),
        ("TMP".into(), temp.to_string_lossy().into_owned()),
    ])
}

pub fn startup_grace(config: &InstanceConfig) -> std::time::Duration {
    if is_gufo(config) {
        std::time::Duration::from_secs(u64::from(
            config.gufo.startup_timeout_seconds.clamp(60, 3600),
        ))
    } else {
        crate::commands::server::INITIAL_HEALTH_GRACE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gufo_rejects_occupied_port_without_touching_its_owner() {
        let owner = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let config = InstanceConfig { port: owner.local_addr().unwrap().port(), ..InstanceConfig::default() };
        assert!(ensure_port_available(&config).is_err());
        assert!(owner.local_addr().is_ok());
        drop(owner);
        assert!(ensure_port_available(&config).is_ok());
    }

    #[test]
    fn legacy_configs_keep_llama_defaults_and_gufo_is_distinct_from_backend() {
        let legacy: InstanceConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.engine_kind, EngineKind::LlamaCpp);
        assert_eq!(startup_grace(&legacy).as_secs(), 120);
        assert_eq!(EngineKind::for_executable("gufo.exe"), EngineKind::Gufo);
        assert_eq!(
            EngineKind::for_executable("llama-server.exe"),
            EngineKind::LlamaCpp
        );
        assert!(serde_json::from_str::<GufoConfig>("{\"unknown\":true}").is_err());
    }

    #[test]
    fn startup_timeout_is_bounded_without_changing_llama() {
        let mut config = InstanceConfig {
            engine_kind: EngineKind::Gufo,
            ..Default::default()
        };
        assert_eq!(startup_grace(&config).as_secs(), 600);
        config.gufo.startup_timeout_seconds = u32::MAX;
        assert_eq!(startup_grace(&config).as_secs(), 3600);
    }

    #[cfg(windows)]
    #[test]
    fn managed_command_preserves_paths_and_rejects_cross_engine_options() {
        let root = std::env::temp_dir().join(format!("lsm-gufo-contract-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let model = root.join("模型 with spaces-00001-of-00002.gguf");
        std::fs::write(&model, b"GGUF").unwrap();
        let mut config = InstanceConfig {
            id: "gufo-contract".into(),
            engine_kind: EngineKind::Gufo,
            model_path: model.to_string_lossy().into_owned(),
            ctx_size: 32768,
            explicit_overrides: Some(Vec::new()),
            ..Default::default()
        };
        assert!(
            prepare(config.clone(), "gufo.exe").is_err(),
            "missing shard must fail before spawn"
        );
        std::fs::write(root.join("模型 with spaces-00002-of-00002.gguf"), b"GGUF").unwrap();
        let (_, command) = prepare(config.clone(), "gufo.exe").unwrap();
        assert!(command
            .windows(2)
            .any(|p| p[0] == "--model" && p[1] == config.model_path));
        assert_eq!(&command[1..3], &["serve", "--host"]);
        assert!(!command
            .iter()
            .any(|p| ["--parallel", "-ngl", "--slot-save-path"].contains(&p.as_str())));
        config.gufo.speculative = "mtp".into();
        assert!(
            prepare(config.clone(), "gufo.exe").is_err(),
            "missing MTP must fail"
        );
        config.gufo.mtp_model = model.to_string_lossy().into_owned();
        assert!(prepare(config.clone(), "gufo.exe")
            .unwrap()
            .1
            .contains(&"--mtp-model".into()));
        config.gufo.model_family = "qwen38_27b".into();
        assert!(
            prepare(config.clone(), "gufo.exe").is_err(),
            "wrong family must fail"
        );
        config.gufo.speculative = "dflash2".into();
        config.gufo.dflash_model = model.to_string_lossy().into_owned();
        assert!(prepare(config.clone(), "gufo.exe")
            .unwrap()
            .1
            .contains(&"--dflash-model".into()));
        config
            .custom_args
            .push("--slot-save-path C:/existing".into());
        assert!(prepare(config, "gufo.exe").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
