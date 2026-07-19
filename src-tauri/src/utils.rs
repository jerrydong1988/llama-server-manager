use crate::models::{GgufMetadataSummary, ModelCapabilities};
use crate::vector_policy::{classify_model_workload, ModelWorkload};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub fn parse_gguf_metadata(path: &Path) -> Result<GgufMetadataSummary, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}", e))?;
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    if file_size < 24 {
        return Err("file is too small".into());
    }

    let mut header = [0u8; 24];
    file.read_exact(&mut header).map_err(|e| format!("{}", e))?;
    if &header[0..4] != b"GGUF" {
        return Err("not a valid GGUF file".into());
    }

    let metadata_kv_count = u64::from_le_bytes(header[16..24].try_into().unwrap()) as usize;
    let mut architecture: Option<String> = None;
    let mut context_length: Option<u32> = None;
    let mut file_type: Option<u32> = None;
    let mut mtp_layers: Option<u32> = None;
    let mut general_type: Option<String> = None;
    let mut model_name: Option<String> = None;
    let mut model_basename: Option<String> = None;
    let mut model_repo: Option<String> = None;
    let mut base_model_name: Option<String> = None;
    let mut base_model_repo: Option<String> = None;
    let mut tags = Vec::new();
    let mut projector_type: Option<String> = None;
    let mut projector_has_vision = false;
    let mut template_has_vision = false;
    let mut has_vision_key = false;
    let mut has_projector_key = false;
    let mut family_hint = path
        .file_name()
        .and_then(|s| s.to_str())
        .and_then(detect_model_family);

    for _ in 0..metadata_kv_count {
        let key = read_gguf_string(&mut file)?;
        let key_lower = key.to_lowercase();
        let value_type = read_u32(&mut file)?;

        if key_lower.contains("vision")
            || key_lower.contains("image")
            || key_lower.contains("mmproj")
            || key_lower.contains("clip")
            || key_lower.contains("patch")
        {
            has_vision_key = true;
        }
        if key_lower.contains("mmproj")
            || key_lower.contains("projector")
            || key_lower.contains("clip")
        {
            has_projector_key = true;
        }
        if family_hint.is_none() {
            family_hint = detect_model_family(&key_lower);
        }

        match value_type {
            7 => {
                let value = read_bool(&mut file)?;
                if key_lower == "clip.has_vision_encoder" && value {
                    projector_has_vision = true;
                }
            }
            8 => {
                let value = read_gguf_string(&mut file)?;
                if key_lower == "general.architecture" {
                    architecture = Some(value.clone());
                }
                match key_lower.as_str() {
                    "general.type" => general_type = Some(value.clone()),
                    "general.name" => model_name = Some(value.clone()),
                    "general.basename" => model_basename = Some(value.clone()),
                    "general.repo_url" => model_repo = Some(value.clone()),
                    "general.base_model.0.name" => base_model_name = Some(value.clone()),
                    "general.base_model.0.repo_url" => base_model_repo = Some(value.clone()),
                    "clip.projector_type" | "clip.vision.projector_type" => {
                        projector_type = Some(value.clone())
                    }
                    "tokenizer.chat_template" => {
                        template_has_vision = chat_template_has_vision_markers(&value)
                    }
                    _ => {}
                }
                if family_hint.is_none() {
                    family_hint = detect_model_family(&value);
                }
            }
            9 if key_lower == "general.tags" => {
                tags = read_gguf_string_array(&mut file)?;
            }
            4 => {
                let value = read_u32(&mut file)?;
                if key == "general.file_type" {
                    file_type = Some(value);
                }
                if key_lower.contains("context_length") {
                    context_length = Some(value);
                }
                if key_lower.contains("nextn_predict_layers") && value > 0 {
                    mtp_layers = Some(value);
                }
            }
            5 => {
                let value = read_i32(&mut file)?;
                if key_lower.contains("context_length") && value > 0 {
                    context_length = Some(value as u32);
                }
                if key_lower.contains("nextn_predict_layers") && value > 0 {
                    mtp_layers = Some(value as u32);
                }
            }
            10 => {
                let value = read_u64(&mut file)?;
                if key_lower.contains("context_length") {
                    context_length = Some(value as u32);
                }
                if key_lower.contains("nextn_predict_layers") && value > 0 {
                    mtp_layers = Some(value as u32);
                }
            }
            11 => {
                let value = read_i64(&mut file)?;
                if key_lower.contains("context_length") && value > 0 {
                    context_length = Some(value as u32);
                }
                if key_lower.contains("nextn_predict_layers") && value > 0 {
                    mtp_layers = Some(value as u32);
                }
            }
            _ => skip_gguf_value(&mut file, value_type)?,
        }
    }

    let file_kind = classify_gguf_file(path);
    let arch_family = architecture.as_deref().and_then(detect_model_family);
    let family = arch_family.or(family_hint);
    let is_mmproj = file_kind == "mmproj"
        || general_type
            .as_deref()
            .map(|value| value.eq_ignore_ascii_case("mmproj"))
            .unwrap_or(false)
        || has_projector_key && architecture.is_none();
    let tags_have_vision = tags.iter().any(|tag| tag_indicates_vision(tag));
    let family_has_vision = is_vision_family(family.as_deref());
    let is_vision_model = !is_mmproj
        && (has_vision_key || tags_have_vision || template_has_vision || family_has_vision);
    let mut vision_evidence = Vec::new();
    if has_vision_key && !is_mmproj {
        vision_evidence.push("metadata-key".to_string());
    }
    if tags_have_vision {
        vision_evidence.push("general.tags".to_string());
    }
    if template_has_vision {
        vision_evidence.push("chat-template".to_string());
    }
    if family_has_vision {
        vision_evidence.push("architecture-family".to_string());
    }
    if projector_has_vision {
        vision_evidence.push("projector-vision-encoder".to_string());
    }
    let inferred_family = architecture
        .as_deref()
        .and_then(infer_visual_family)
        .or_else(|| model_name.as_deref().and_then(infer_visual_family))
        .or_else(|| base_model_name.as_deref().and_then(infer_visual_family));
    let vision_family = if is_vision_model {
        family.clone().or(inferred_family)
    } else {
        None
    };
    let projector_family = if is_mmproj {
        projector_type
            .as_deref()
            .and_then(infer_visual_family)
            .or_else(|| family.clone())
    } else {
        None
    };
    let workload = classify_model_workload(architecture.as_deref(), path);

    Ok(GgufMetadataSummary {
        architecture,
        context_length,
        quant_type: quant_type_from_file_type(file_type, path),
        capabilities: ModelCapabilities {
            metadata_complete: true,
            is_embedding_model: Some(workload == ModelWorkload::Embedding),
            is_reranker_model: Some(workload == ModelWorkload::Reranker),
            has_builtin_mtp: mtp_layers.unwrap_or(0) > 0,
            mtp_layers,
            is_vision_model,
            vision_status: Some(
                if is_vision_model {
                    "confirmed"
                } else {
                    "unknown"
                }
                .into(),
            ),
            vision_evidence,
            vision_family,
            is_mmproj,
            projector_family,
            projector_type,
            model_name,
            model_basename,
            model_repo,
            base_model_name,
            base_model_repo,
            tags,
        },
    })
}

fn read_u32(file: &mut std::fs::File) -> Result<u32, String> {
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf).map_err(|e| format!("{}", e))?;
    Ok(u32::from_le_bytes(buf))
}

fn read_bool(file: &mut std::fs::File) -> Result<bool, String> {
    let mut buf = [0u8; 1];
    file.read_exact(&mut buf).map_err(|e| format!("{}", e))?;
    Ok(buf[0] != 0)
}

fn read_i32(file: &mut std::fs::File) -> Result<i32, String> {
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf).map_err(|e| format!("{}", e))?;
    Ok(i32::from_le_bytes(buf))
}

fn read_u64(file: &mut std::fs::File) -> Result<u64, String> {
    let mut buf = [0u8; 8];
    file.read_exact(&mut buf).map_err(|e| format!("{}", e))?;
    Ok(u64::from_le_bytes(buf))
}

fn read_i64(file: &mut std::fs::File) -> Result<i64, String> {
    let mut buf = [0u8; 8];
    file.read_exact(&mut buf).map_err(|e| format!("{}", e))?;
    Ok(i64::from_le_bytes(buf))
}

fn read_gguf_string(file: &mut std::fs::File) -> Result<String, String> {
    let len = read_u64(file)? as usize;
    if len > 10_000_000 {
        return Err("GGUF string is too large".into());
    }
    let mut bytes = vec![0u8; len];
    file.read_exact(&mut bytes).map_err(|e| format!("{}", e))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn read_gguf_string_array(file: &mut std::fs::File) -> Result<Vec<String>, String> {
    let array_type = read_u32(file)?;
    let array_len = read_u64(file)?;
    if array_len > 10_000_000 {
        return Err("GGUF array is too large".into());
    }
    if array_type != 8 {
        skip_gguf_array(file, array_type, array_len)?;
        return Ok(Vec::new());
    }
    let mut values = Vec::with_capacity(array_len.min(256) as usize);
    for index in 0..array_len {
        let value = read_gguf_string(file)?;
        if index < 256 {
            values.push(value);
        }
    }
    Ok(values)
}

fn skip_bytes(file: &mut std::fs::File, bytes: u64) -> Result<(), String> {
    file.seek(SeekFrom::Current(bytes as i64))
        .map_err(|e| format!("{}", e))?;
    Ok(())
}

fn skip_gguf_array(
    file: &mut std::fs::File,
    array_type: u32,
    array_len: u64,
) -> Result<(), String> {
    match array_type {
        0 | 1 | 7 => skip_bytes(file, array_len),
        2 | 3 => skip_bytes(file, array_len.saturating_mul(2)),
        4..=6 => skip_bytes(file, array_len.saturating_mul(4)),
        10..=12 => skip_bytes(file, array_len.saturating_mul(8)),
        8 => {
            for _ in 0..array_len {
                let _ = read_gguf_string(file)?;
            }
            Ok(())
        }
        _ => Err(format!("unsupported GGUF array type {}", array_type)),
    }
}

fn skip_gguf_value(file: &mut std::fs::File, value_type: u32) -> Result<(), String> {
    match value_type {
        0 | 1 | 7 => skip_bytes(file, 1),
        2 | 3 => skip_bytes(file, 2),
        4..=6 => skip_bytes(file, 4),
        10..=12 => skip_bytes(file, 8),
        8 => {
            let _ = read_gguf_string(file)?;
            Ok(())
        }
        9 => {
            let array_type = read_u32(file)?;
            let array_len = read_u64(file)?;
            if array_len > 10_000_000 {
                return Err("GGUF array is too large".into());
            }
            skip_gguf_array(file, array_type, array_len)
        }
        _ => Err(format!("unsupported GGUF value type {}", value_type)),
    }
}

fn tag_indicates_vision(tag: &str) -> bool {
    matches!(
        tag.trim().to_lowercase().as_str(),
        "multimodal" | "image-text-to-text" | "any-to-any" | "vision"
    )
}

fn chat_template_has_vision_markers(template: &str) -> bool {
    let normalized = template.to_lowercase();
    [
        "image_url",
        "<|vision_start|>",
        "<|image_pad|>",
        "<image>",
        "image_token",
        "video_url",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn infer_visual_family(text: &str) -> Option<String> {
    if let Some(family) = detect_model_family(text) {
        return Some(family);
    }
    let normalized = text.to_lowercase().replace(['_', '-', '.'], "");
    let families = [
        ("qwen35", "qwen-vl"),
        ("qwen36", "qwen-vl"),
        ("gemma4", "gemma-vision"),
        ("mistral3", "pixtral"),
        ("ministral3", "pixtral"),
        ("pixtral", "pixtral"),
    ];
    families
        .iter()
        .find(|(needle, _)| normalized.contains(*needle))
        .map(|(_, family)| (*family).to_string())
}

fn detect_model_family(text: &str) -> Option<String> {
    let normalized = text.to_lowercase().replace(['_', '-', '.'], "");
    let families = [
        ("qwen25vl", "qwen-vl"),
        ("qwen2vl", "qwen-vl"),
        ("qwen3vl", "qwen-vl"),
        ("llava", "llava"),
        ("minicpmv", "minicpm-v"),
        ("internvl", "intern-vl"),
        ("gemma3", "gemma-vision"),
        ("glm4v", "glm-v"),
        ("phi3v", "phi-v"),
        ("phi4v", "phi-v"),
        ("smolvlm", "smolvlm"),
        ("idefics", "idefics"),
        ("moondream", "moondream"),
        ("clip", "clip"),
        ("vit", "vit"),
    ];
    families
        .iter()
        .find(|(needle, _)| normalized.contains(*needle))
        .map(|(_, family)| (*family).to_string())
}

fn is_vision_family(family: Option<&str>) -> bool {
    matches!(
        family,
        Some(
            "qwen-vl"
                | "llava"
                | "minicpm-v"
                | "intern-vl"
                | "gemma-vision"
                | "glm-v"
                | "phi-v"
                | "smolvlm"
                | "idefics"
                | "moondream"
        )
    )
}

fn quant_type_from_file_type(file_type: Option<u32>, path: &Path) -> Option<String> {
    if let Some(named_quant) = quant_type_from_name(path) {
        return Some(named_quant);
    }

    match file_type {
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
        Some(33) => Some("Q4_0_4_4".into()),
        Some(34) => Some("Q4_0_4_8".into()),
        Some(35) => Some("Q4_0_8_8".into()),
        Some(36) => Some("TQ1_0".into()),
        Some(37) => Some("TQ2_0".into()),
        Some(38) => Some("MXFP4_MOE".into()),
        Some(39) => Some("NVFP4".into()),
        Some(40) => Some("Q1_0".into()),
        Some(41) => Some("Q2_0".into()),
        _ => quant_type_from_name(path),
    }
}

pub fn normalize_quant_type_for_path(quant_type: Option<String>, path: &Path) -> Option<String> {
    quant_type_from_name(path).or(quant_type)
}

fn quant_type_from_name(path: &Path) -> Option<String> {
    let fname = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let normalized = fname.replace(['-', '.', ' '], "_");
    let patterns = [
        ("mxfp4_moe", "MXFP4_MOE"),
        ("mxfp4", "MXFP4"),
        ("mxfp6", "MXFP6"),
        ("mxfp8", "MXFP8"),
        ("nvfp4", "NVFP4"),
        ("tq1_0", "TQ1_0"),
        ("tq2_0", "TQ2_0"),
        ("iq4_xs", "IQ4_XS"),
        ("iq4_nl", "IQ4_NL"),
        ("iq3_xxs", "IQ3_XXS"),
        ("iq3_xs", "IQ3_XS"),
        ("iq3_m", "IQ3_M"),
        ("iq3_s", "IQ3_S"),
        ("iq2_xxs", "IQ2_XXS"),
        ("iq2_xs", "IQ2_XS"),
        ("iq2_m", "IQ2_M"),
        ("iq2_s", "IQ2_S"),
        ("iq1_m", "IQ1_M"),
        ("iq1_s", "IQ1_S"),
        ("iq4", "IQ4"),
        ("iq3", "IQ3"),
        ("iq2", "IQ2"),
        ("iq1", "IQ1"),
        ("q8_k", "Q8_K"),
        ("q8k", "Q8_K"),
        ("q8_0", "Q8_0"),
        ("q8o", "Q8_0"),
        ("q6_k", "Q6_K"),
        ("q6k", "Q6_K"),
        ("q5_k_m", "Q5_K_M"),
        ("q5k_m", "Q5_K_M"),
        ("q5_k_s", "Q5_K_S"),
        ("q5k_s", "Q5_K_S"),
        ("q5_k", "Q5_K"),
        ("q5k", "Q5_K"),
        ("q5_1", "Q5_1"),
        ("q5_0", "Q5_0"),
        ("q4_k_xl", "Q4_K_XL"),
        ("q4k_xl", "Q4_K_XL"),
        ("q4_k_l", "Q4_K_L"),
        ("q4k_l", "Q4_K_L"),
        ("q4_k_m", "Q4_K_M"),
        ("q4k_m", "Q4_K_M"),
        ("q4_k_s", "Q4_K_S"),
        ("q4k_s", "Q4_K_S"),
        ("q4_k", "Q4_K"),
        ("q4k", "Q4_K"),
        ("q4_1", "Q4_1"),
        ("q4_0", "Q4_0"),
        ("q3_k_l", "Q3_K_L"),
        ("q3k_l", "Q3_K_L"),
        ("q3_k_m", "Q3_K_M"),
        ("q3k_m", "Q3_K_M"),
        ("q3_k_s", "Q3_K_S"),
        ("q3k_s", "Q3_K_S"),
        ("q3_k", "Q3_K"),
        ("q3k", "Q3_K"),
        ("q2_k_s", "Q2_K_S"),
        ("q2k_s", "Q2_K_S"),
        ("q2_k", "Q2_K"),
        ("q2k", "Q2_K"),
        ("q2_0", "Q2_0"),
        ("q1_0", "Q1_0"),
        ("bf16", "BF16"),
        ("f16", "F16"),
        ("f32", "F32"),
    ];

    if let Some((_, label)) = patterns
        .iter()
        .find(|(needle, _)| normalized.contains(*needle))
    {
        Some((*label).into())
    } else if fname.contains("apex") {
        Some("IQ (APEX)".into())
    } else if fname.contains("-i-compact") || fname.contains("_i-compact") {
        Some("IQ (Compact)".into())
    } else if fname.contains("-i-static") || fname.contains("_i-static") {
        Some("IQ (Static)".into())
    } else if fname.contains("-i-dynamic") || fname.contains("_i-dynamic") {
        Some("IQ (Dynamic)".into())
    } else {
        None
    }
}

pub fn classify_gguf_file(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if name.contains("mmproj") || name.contains("clip") {
        "mmproj"
    } else if name.contains("imatrix") {
        "imatrix"
    } else {
        "model"
    }
}

pub fn detect_backend(dir: &Path) -> String {
    let entries: Vec<String> = std::fs::read_dir(dir)
        .ok()
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let joined = entries.join(" ");
    if joined.contains("roc") || joined.contains("hip") || joined.contains("amd") {
        "ROCm".into()
    } else if joined.contains("vulkan") || joined.contains("vk") {
        "Vulkan".into()
    } else if joined.contains("cuda") || joined.contains("cublas") {
        "CUDA".into()
    } else {
        "CPU".into()
    }
}

pub fn get_app_dir() -> PathBuf {
    std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("."))
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

fn unbracket_host(host: &str) -> &str {
    let trimmed = host.trim();
    trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed)
}

pub fn format_host_port(host: &str, port: u16) -> String {
    let normalized = unbracket_host(host);
    if normalized.contains(':') {
        format!("[{}]:{}", normalized, port)
    } else {
        format!("{}:{}", normalized, port)
    }
}

pub fn service_url(scheme: &str, host: &str, port: u16, api_prefix: &str, path: &str) -> String {
    let scheme = if scheme.eq_ignore_ascii_case("https") {
        "https"
    } else {
        "http"
    };
    let prefix = api_prefix.trim_matches('/');
    let path = path.trim_start_matches('/');
    let suffix = match (prefix.is_empty(), path.is_empty()) {
        (true, true) => String::new(),
        (true, false) => format!("/{path}"),
        (false, true) => format!("/{prefix}"),
        (false, false) => format!("/{prefix}/{path}"),
    };
    format!("{scheme}://{}{}", format_host_port(host, port), suffix)
}

pub fn resolve_socket_addrs(host: &str, port: u16) -> Result<Vec<std::net::SocketAddr>, String> {
    use std::net::ToSocketAddrs;

    let normalized = unbracket_host(host);
    let addresses = (normalized, port)
        .to_socket_addrs()
        .map_err(|e| {
            format!(
                "Unable to resolve {}: {}",
                format_host_port(normalized, port),
                e
            )
        })?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        Err(format!(
            "No address found for {}",
            format_host_port(normalized, port)
        ))
    } else {
        Ok(addresses)
    }
}

pub fn connect_tcp(host: &str, port: u16, timeout: std::time::Duration) -> Result<(), String> {
    let addresses = resolve_socket_addrs(host, port)?;
    let mut last_error = None;
    for address in addresses {
        match std::net::TcpStream::connect_timeout(&address, timeout) {
            Ok(_) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error
        .map(|error| error.to_string())
        .unwrap_or_else(|| "Connection failed".to_string()))
}

pub fn get_data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(appdata).join("LlamaServerManager");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join("Library/Application Support/LlamaServerManager");
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(xdg).join("LlamaServerManager");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".local/share/LlamaServerManager");
        }
    }
    get_app_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    enum TestMetadataValue<'a> {
        String(&'a str),
        Bool(bool),
        Strings(&'a [&'a str]),
    }

    fn push_test_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }

    fn write_test_gguf(path: &Path, entries: &[(&str, TestMetadataValue<'_>)]) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GGUF");
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u64).to_le_bytes());
        for (key, value) in entries {
            push_test_string(&mut bytes, key);
            match value {
                TestMetadataValue::String(value) => {
                    bytes.extend_from_slice(&8_u32.to_le_bytes());
                    push_test_string(&mut bytes, value);
                }
                TestMetadataValue::Bool(value) => {
                    bytes.extend_from_slice(&7_u32.to_le_bytes());
                    bytes.push(u8::from(*value));
                }
                TestMetadataValue::Strings(values) => {
                    bytes.extend_from_slice(&9_u32.to_le_bytes());
                    bytes.extend_from_slice(&8_u32.to_le_bytes());
                    bytes.extend_from_slice(&(values.len() as u64).to_le_bytes());
                    for value in *values {
                        push_test_string(&mut bytes, value);
                    }
                }
            }
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn multimodal_metadata_uses_tags_templates_and_projector_source_identity() {
        let dir = std::env::temp_dir().join(format!(
            "lsm-multimodal-capability-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let model_path = dir.join("Qwen3.6-35B-A3B-Q8_0.gguf");
        write_test_gguf(
            &model_path,
            &[
                (
                    "general.architecture",
                    TestMetadataValue::String("qwen35moe"),
                ),
                ("general.type", TestMetadataValue::String("model")),
                ("general.name", TestMetadataValue::String("Qwen3.6-35B-A3B")),
                (
                    "general.base_model.0.repo_url",
                    TestMetadataValue::String("https://huggingface.co/Qwen/Qwen3.6-35B-A3B"),
                ),
                (
                    "general.tags",
                    TestMetadataValue::Strings(&["qwen3_5_moe", "image-text-to-text"]),
                ),
            ],
        );
        let model = parse_gguf_metadata(&model_path).unwrap();
        assert!(model.capabilities.is_vision_model);
        assert_eq!(
            model.capabilities.vision_status.as_deref(),
            Some("confirmed")
        );
        assert_eq!(model.capabilities.vision_family.as_deref(), Some("qwen-vl"));
        assert!(model
            .capabilities
            .vision_evidence
            .contains(&"general.tags".into()));
        assert_eq!(
            model.capabilities.base_model_repo.as_deref(),
            Some("https://huggingface.co/Qwen/Qwen3.6-35B-A3B")
        );

        let projector_path = dir.join("mmproj-BF16.gguf");
        write_test_gguf(
            &projector_path,
            &[
                ("general.architecture", TestMetadataValue::String("clip")),
                ("general.type", TestMetadataValue::String("mmproj")),
                ("general.name", TestMetadataValue::String("Qwen3.6-35B-A3B")),
                (
                    "general.base_model.0.repo_url",
                    TestMetadataValue::String("https://huggingface.co/Qwen/Qwen3.6-35B-A3B"),
                ),
                ("clip.has_vision_encoder", TestMetadataValue::Bool(true)),
                (
                    "clip.projector_type",
                    TestMetadataValue::String("qwen3vl_merger"),
                ),
            ],
        );
        let projector = parse_gguf_metadata(&projector_path).unwrap();
        assert!(projector.capabilities.is_mmproj);
        assert_eq!(
            projector.capabilities.projector_type.as_deref(),
            Some("qwen3vl_merger")
        );
        assert_eq!(
            projector.capabilities.projector_family.as_deref(),
            Some("qwen-vl")
        );
        assert!(projector
            .capabilities
            .vision_evidence
            .contains(&"projector-vision-encoder".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn multimodal_metadata_recognizes_chat_templates_and_new_projector_keys() {
        let dir = std::env::temp_dir().join(format!(
            "lsm-multimodal-template-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let model_path = dir.join("Ministral-3-8B.gguf");
        write_test_gguf(
            &model_path,
            &[
                (
                    "general.architecture",
                    TestMetadataValue::String("mistral3"),
                ),
                ("general.type", TestMetadataValue::String("model")),
                (
                    "tokenizer.chat_template",
                    TestMetadataValue::String("{{ image_url }}"),
                ),
            ],
        );
        let model = parse_gguf_metadata(&model_path).unwrap();
        assert!(model.capabilities.is_vision_model);
        assert_eq!(model.capabilities.vision_family.as_deref(), Some("pixtral"));

        let projector_path = dir.join("mmproj.gguf");
        write_test_gguf(
            &projector_path,
            &[
                ("general.architecture", TestMetadataValue::String("clip")),
                ("general.type", TestMetadataValue::String("mmproj")),
                (
                    "clip.vision.projector_type",
                    TestMetadataValue::String("gemma4uv"),
                ),
            ],
        );
        let projector = parse_gguf_metadata(&projector_path).unwrap();
        assert_eq!(
            projector.capabilities.projector_type.as_deref(),
            Some("gemma4uv")
        );
        assert_eq!(
            projector.capabilities.projector_family.as_deref(),
            Some("gemma-vision")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[ignore = "requires LLAMA_MULTIMODAL_TEST_ROOT"]
    fn multimodal_metadata_parses_external_corpus() {
        let root = std::env::var("LLAMA_MULTIMODAL_TEST_ROOT")
            .expect("LLAMA_MULTIMODAL_TEST_ROOT must point to a GGUF model directory");
        let mut parsed = 0_u32;
        let mut vision_models = 0_u32;
        let mut projectors = 0_u32;
        let mut vision_sources = std::collections::HashSet::new();
        let mut projector_sources = std::collections::HashSet::new();
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !entry.file_type().is_file()
                || !path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| extension.eq_ignore_ascii_case("gguf"))
                    .unwrap_or(false)
            {
                continue;
            }
            let metadata = parse_gguf_metadata(path)
                .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()));
            parsed += 1;
            vision_models += u32::from(metadata.capabilities.is_vision_model);
            projectors += u32::from(metadata.capabilities.is_mmproj);
            if let Some(source) = metadata.capabilities.base_model_repo.as_deref() {
                let normalized = source.trim().trim_end_matches('/').to_lowercase();
                if metadata.capabilities.is_mmproj {
                    projector_sources.insert(normalized);
                } else if metadata.capabilities.is_vision_model {
                    vision_sources.insert(normalized);
                }
            }
        }
        assert!(parsed > 0, "the external corpus contains no GGUF files");
        assert!(
            vision_models > 0,
            "the external corpus contains no recognized vision model"
        );
        assert!(
            projectors > 0,
            "the external corpus contains no recognized projector"
        );
        assert!(
            vision_sources
                .iter()
                .any(|source| projector_sources.contains(source)),
            "the external corpus contains no source-confirmed model/projector pair"
        );
    }

    #[test]
    fn parsed_metadata_sets_explicit_vector_capabilities() {
        let dir =
            std::env::temp_dir().join(format!("lsm-vector-capability-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("model.gguf");
        let key = b"general.architecture";
        let value = b"bert";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GGUF");
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&1_u64.to_le_bytes());
        bytes.extend_from_slice(&(key.len() as u64).to_le_bytes());
        bytes.extend_from_slice(key);
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value);
        std::fs::write(&path, bytes).unwrap();

        let summary = parse_gguf_metadata(&path).unwrap();

        assert_eq!(summary.capabilities.is_embedding_model, Some(true));
        assert_eq!(summary.capabilities.is_reranker_model, Some(false));
        assert!(!summary.capabilities.is_vision_model);
        assert_eq!(
            summary.capabilities.vision_status.as_deref(),
            Some("unknown")
        );
        assert!(summary.capabilities.vision_evidence.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn path(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    #[test]
    fn quant_name_preserves_extended_q4_k_variants_over_metadata() {
        assert_eq!(
            quant_type_from_file_type(
                Some(15),
                &path("nvidia_Nemotron-3-Super-120B-A12B-Q4_K_L-00001-of-00003.gguf")
            )
            .as_deref(),
            Some("Q4_K_L")
        );
        assert_eq!(
            quant_type_from_file_type(Some(15), &path("model-Q4_K_XL.gguf")).as_deref(),
            Some("Q4_K_XL")
        );
        assert_eq!(
            quant_type_from_file_type(Some(15), &path("plain-model.gguf")).as_deref(),
            Some("Q4_K_M")
        );
    }

    #[test]
    fn quant_name_detects_specific_k_quant_suffixes() {
        let cases = [
            ("model-Q5_K_M.gguf", "Q5_K_M"),
            ("model-Q5_K_S.gguf", "Q5_K_S"),
            ("model-Q4_K_M.gguf", "Q4_K_M"),
            ("model-Q4_K_S.gguf", "Q4_K_S"),
            ("model-Q3_K_L.gguf", "Q3_K_L"),
            ("model-Q3_K_M.gguf", "Q3_K_M"),
            ("model-Q3_K_S.gguf", "Q3_K_S"),
            ("model-Q2_K_S.gguf", "Q2_K_S"),
            ("model-Q2_K.gguf", "Q2_K"),
        ];

        for (name, expected) in cases {
            assert_eq!(
                quant_type_from_name(&path(name)).as_deref(),
                Some(expected),
                "{name}"
            );
        }
    }

    #[test]
    fn quant_name_detects_newer_and_iq_variants() {
        let cases = [
            ("model-IQ4_XS.gguf", "IQ4_XS"),
            ("model-IQ4_NL.gguf", "IQ4_NL"),
            ("model-IQ3_XXS.gguf", "IQ3_XXS"),
            ("model-IQ3_XS.gguf", "IQ3_XS"),
            ("model-IQ3_M.gguf", "IQ3_M"),
            ("model-IQ3_S.gguf", "IQ3_S"),
            ("model-IQ2_XXS.gguf", "IQ2_XXS"),
            ("model-IQ2_XS.gguf", "IQ2_XS"),
            ("model-IQ2_M.gguf", "IQ2_M"),
            ("model-IQ2_S.gguf", "IQ2_S"),
            ("model-IQ1_M.gguf", "IQ1_M"),
            ("model-IQ1_S.gguf", "IQ1_S"),
            ("model-TQ1_0.gguf", "TQ1_0"),
            ("model-TQ2_0.gguf", "TQ2_0"),
            ("model-MXFP4_MOE.gguf", "MXFP4_MOE"),
            ("model-NVFP4.gguf", "NVFP4"),
            ("model-Q1_0.gguf", "Q1_0"),
            ("model-Q2_0.gguf", "Q2_0"),
        ];

        for (name, expected) in cases {
            assert_eq!(
                quant_type_from_name(&path(name)).as_deref(),
                Some(expected),
                "{name}"
            );
        }
    }

    #[test]
    fn quant_name_does_not_guess_precision_for_unknown_gguf() {
        assert_eq!(quant_type_from_name(&path("model.gguf")), None);
    }

    #[test]
    fn classify_file_distinguishes_models_projectors_and_imatrix() {
        assert_eq!(classify_gguf_file(&path("mmproj-model-f16.gguf")), "mmproj");
        assert_eq!(
            classify_gguf_file(&path("model-imatrix.dat.gguf")),
            "imatrix"
        );
        assert_eq!(classify_gguf_file(&path("model-Q4_K_M.gguf")), "model");
    }

    #[test]
    fn network_authority_brackets_ipv6_hosts() {
        assert_eq!(format_host_port("127.0.0.1", 8080), "127.0.0.1:8080");
        assert_eq!(
            format_host_port("worker.local", 50052),
            "worker.local:50052"
        );
        assert_eq!(format_host_port("::1", 50052), "[::1]:50052");
        assert_eq!(format_host_port("[::1]", 50052), "[::1]:50052");
        assert_eq!(
            service_url("http", "::1", 8080, "", "/health"),
            "http://[::1]:8080/health"
        );
    }

    #[test]
    fn socket_resolution_accepts_ipv6_and_hostnames() {
        assert!(resolve_socket_addrs("::1", 50052)
            .unwrap()
            .iter()
            .any(std::net::SocketAddr::is_ipv6));
        assert!(!resolve_socket_addrs("localhost", 50052).unwrap().is_empty());
    }
}
