use crate::models::{GgufMetadataSummary, ModelCapabilities};
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
            8 => {
                let value = read_gguf_string(&mut file)?;
                if key == "general.architecture" {
                    architecture = Some(value.clone());
                }
                if family_hint.is_none() {
                    family_hint = detect_model_family(&value);
                }
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
    let is_mmproj = file_kind == "mmproj" || has_projector_key && architecture.is_none();
    let is_vision_model = !is_mmproj && (has_vision_key || is_vision_family(family.as_deref()));

    Ok(GgufMetadataSummary {
        architecture,
        context_length,
        quant_type: quant_type_from_file_type(file_type, path),
        capabilities: ModelCapabilities {
            metadata_complete: true,
            has_builtin_mtp: mtp_layers.unwrap_or(0) > 0,
            mtp_layers,
            is_vision_model,
            vision_family: if is_vision_model {
                family.clone()
            } else {
                None
            },
            is_mmproj,
            projector_family: if is_mmproj { family } else { None },
        },
    })
}

fn read_u32(file: &mut std::fs::File) -> Result<u32, String> {
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf).map_err(|e| format!("{}", e))?;
    Ok(u32::from_le_bytes(buf))
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

fn skip_bytes(file: &mut std::fs::File, bytes: u64) -> Result<(), String> {
    file.seek(SeekFrom::Current(bytes as i64))
        .map_err(|e| format!("{}", e))?;
    Ok(())
}

fn skip_gguf_value(file: &mut std::fs::File, value_type: u32) -> Result<(), String> {
    match value_type {
        0 | 1 | 7 => skip_bytes(file, 1),
        2 | 3 => skip_bytes(file, 2),
        4 | 5 | 6 => skip_bytes(file, 4),
        10 | 11 | 12 => skip_bytes(file, 8),
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
            match array_type {
                0 | 1 | 7 => skip_bytes(file, array_len),
                2 | 3 => skip_bytes(file, array_len.saturating_mul(2)),
                4 | 5 | 6 => skip_bytes(file, array_len.saturating_mul(4)),
                10 | 11 | 12 => skip_bytes(file, array_len.saturating_mul(8)),
                8 => {
                    for _ in 0..array_len {
                        let _ = read_gguf_string(file)?;
                    }
                    Ok(())
                }
                _ => Err(format!("unsupported GGUF array type {}", array_type)),
            }
        }
        _ => Err(format!("unsupported GGUF value type {}", value_type)),
    }
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
        _ => quant_type_from_name(path),
    }
}

fn quant_type_from_name(path: &Path) -> Option<String> {
    let fname = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if fname.contains("bf16") {
        Some("BF16".into())
    } else if fname.contains("f16") {
        Some("F16".into())
    } else if fname.contains("f32") {
        Some("F32".into())
    } else if fname.contains("q8_k") || fname.contains("q8k") {
        Some("Q8_K".into())
    } else if fname.contains("q8_0") || fname.contains("q8o") {
        Some("Q8_0".into())
    } else if fname.contains("q6_k") || fname.contains("q6k") {
        Some("Q6_K".into())
    } else if fname.contains("q5_k") || fname.contains("q5k") {
        Some("Q5_K".into())
    } else if fname.contains("q5_1") {
        Some("Q5_1".into())
    } else if fname.contains("q5_0") {
        Some("Q5_0".into())
    } else if fname.contains("q4_k_xl") || fname.contains("q4k_xl") {
        Some("Q4_K_XL".into())
    } else if fname.contains("q4_k_l") || fname.contains("q4k_l") {
        Some("Q4_K_L".into())
    } else if fname.contains("q4_k") || fname.contains("q4k") {
        Some("Q4_K".into())
    } else if fname.contains("q4_1") {
        Some("Q4_1".into())
    } else if fname.contains("q4_0") {
        Some("Q4_0".into())
    } else if fname.contains("q3_k") || fname.contains("q3k") {
        Some("Q3_K".into())
    } else if fname.contains("q2_k") || fname.contains("q2k") {
        Some("Q2_K".into())
    } else if fname.contains("iq4") {
        Some("IQ4".into())
    } else if fname.contains("iq3") {
        Some("IQ3".into())
    } else if fname.contains("iq2") {
        Some("IQ2".into())
    } else if fname.contains("iq1") {
        Some("IQ1".into())
    } else if fname.contains("mxfp4") {
        Some("MXFP4".into())
    } else if fname.contains("mxfp6") {
        Some("MXFP6".into())
    } else if fname.contains("mxfp8") {
        Some("MXFP8".into())
    } else if fname.contains("apex") {
        Some("IQ (APEX)".into())
    } else if fname.contains("-i-compact") || fname.contains("_i-compact") {
        Some("IQ (Compact)".into())
    } else if fname.contains("-i-static") || fname.contains("_i-static") {
        Some("IQ (Static)".into())
    } else if fname.contains("-i-dynamic") || fname.contains("_i-dynamic") {
        Some("IQ (Dynamic)".into())
    } else if fname.contains("gguf") {
        Some("Original Precision".into())
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
