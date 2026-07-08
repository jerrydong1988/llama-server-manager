use std::path::{Path, PathBuf};

// ── GGUF 元信息解析 ───────────────────────────────────────────────
pub fn parse_gguf_metadata(
    path: &Path,
) -> Result<(Option<String>, Option<u32>, Option<String>, bool), String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}", e))?;
    let file_size = f.metadata().map(|m| m.len()).unwrap_or(0) as usize;
    if file_size < 24 {
        return Err("文件太小".into());
    }

    // #8: 先读头部获取 metadata_kv_count，动态计算需要读取的字节
    let mut header = [0u8; 24];
    use std::io::Read;
    f.read_exact(&mut header).map_err(|e| format!("{}", e))?;
    if &header[0..4] != b"GGUF" {
        return Err("不是有效的 GGUF 文件".into());
    }

    let metadata_kv_count = u64::from_le_bytes(header[16..24].try_into().unwrap()) as usize;
    // 每个 KV 对最多 ~512 字节（key + type + value），最多 200 个
    let estimate = 24 + (metadata_kv_count.min(200) * 512);
    let read_size = estimate.min(file_size);
    let mut data = vec![0u8; read_size];
    data[..24].copy_from_slice(&header);
    f.read_exact(&mut data[24..])
        .map_err(|e| format!("{}", e))?;
    let mut pos: usize = 24;

    fn read_string(data: &[u8], pos: &mut usize) -> Option<String> {
        if *pos + 8 > data.len() {
            return None;
        }
        let len = u64::from_le_bytes(data[*pos..*pos + 8].try_into().unwrap()) as usize;
        *pos += 8;
        if len > 10_000_000 || *pos + len > data.len() {
            return None;
        }
        let s = String::from_utf8_lossy(&data[*pos..*pos + len]).to_string();
        *pos += len;
        Some(s)
    }

    let mut architecture: Option<String> = None;
    let mut context_length: Option<u32> = None;
    let mut file_type: Option<u32> = None;
    let mut has_mtp: bool = false;

    for _ in 0..metadata_kv_count.min(200) {
        if pos + 2 > data.len() {
            break;
        }
        let key = match read_string(&data, &mut pos) {
            Some(k) => k,
            None => break,
        };
        if pos + 4 > data.len() {
            break;
        }
        let vtype = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
        pos += 4;

        match vtype {
            8 => {
                if pos + 8 > data.len() {
                    break;
                }
                if let Some(s) = read_string(&data, &mut pos) {
                    if key == "general.architecture" {
                        architecture = Some(s);
                    }
                } else {
                    break;
                }
            }
            4 => {
                if pos + 4 > data.len() {
                    break;
                }
                let v = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
                pos += 4;
                if key == "general.file_type" {
                    file_type = Some(v);
                }
                if key.contains("context_length") {
                    context_length = Some(v);
                }
                if key.contains("nextn_predict_layers") && v > 0 {
                    has_mtp = true;
                }
            }
            5 => {
                if pos + 4 > data.len() {
                    break;
                }
                let v = i32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
                pos += 4;
                if key.contains("context_length") {
                    context_length = Some(v as u32);
                }
                if key.contains("nextn_predict_layers") && v > 0 {
                    has_mtp = true;
                }
            }
            10 => {
                if pos + 8 > data.len() {
                    break;
                }
                let v = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
                pos += 8;
                if key.contains("context_length") {
                    context_length = Some(v as u32);
                }
                if key.contains("nextn_predict_layers") && v > 0 {
                    has_mtp = true;
                }
            }
            7 => {
                pos += 1;
            }
            0 | 1 => {
                pos += 1;
            }
            2 | 3 => {
                pos += 2;
            }
            6 | 12 => {
                pos += if vtype == 6 { 4 } else { 8 };
            }
            9 => {
                if pos + 12 > data.len() {
                    break;
                }
                let _arr_type = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
                pos += 4;
                let arr_len = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap()) as usize;
                pos += 8;
                if arr_len > 100_000 {
                    break;
                }
                let elem_size = match _arr_type {
                    0..=3 => 1_usize,
                    4 | 5 => 4,
                    6 | 12 => 4,
                    7 => 1,
                    8 => 0,
                    10 | 11 => 8,
                    _ => 0,
                };
                if elem_size > 0 {
                    pos += (arr_len * elem_size).min(1_000_000);
                } else if _arr_type == 8 {
                    for _ in 0..arr_len.min(1000) {
                        if read_string(&data, &mut pos).is_none() {
                            break;
                        }
                    }
                }
            }
            _ => break,
        }
    }

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
                Some("原始精度".into())
            } else {
                None
            }
        }
    };

    Ok((architecture, context_length, quant_type, has_mtp))
}

// ── 文件类型分类 ──────────────────────────────────────────────────
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

// ── 后端类型检测 ──────────────────────────────────────────────────
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

// ── 获取应用目录 ──────────────────────────────────────────────────
pub fn get_app_dir() -> PathBuf {
    std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("."))
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

// ── 获取数据目录 (ASCII-safe, 避免中文路径) ──────────────────────
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
