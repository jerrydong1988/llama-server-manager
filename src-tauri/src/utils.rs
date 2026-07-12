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
}
