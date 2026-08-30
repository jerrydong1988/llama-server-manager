use crate::commands::engine_capabilities::capabilities_match_executable;
use crate::commands::model_inventory::{
    self, InventoryDirectoryRecord, InventoryEngineRecord, InventoryModelRecord,
};
use crate::models::{
    AppState, EngineInfo, InstanceConfig, ModelCapabilities, ModelInfo, RunningInstance,
};
use crate::path_utils::{path_identity_key, path_is_within, paths_equal};
use crate::utils;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

static MODEL_SCAN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static ENGINE_SCAN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MAX_MODEL_PRESET_FILES: usize = 64;
const MAX_MODEL_PRESET_BYTES: u64 = 16 * 1024 * 1024;

fn file_mtime(path: &Path) -> u64 {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn canonical_key(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn engine_path_identity(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    path_identity_key(&canonical)
}

#[derive(Debug, Default)]
struct ModelArtifactReferences {
    exact_paths: BTreeSet<String>,
    model_directories: BTreeSet<String>,
    preset_files: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelArgumentKind {
    ExactPath,
    ModelDirectory,
    PresetFile,
}

fn model_argument_kind(flag: &str) -> Option<ModelArgumentKind> {
    match flag {
        "-m"
        | "--model"
        | "-md"
        | "--model-draft"
        | "--spec-draft-model"
        | "-mm"
        | "--mmproj"
        | "--lora"
        | "--lora-scaled"
        | "--control-vector"
        | "--control-vector-scaled" => Some(ModelArgumentKind::ExactPath),
        "--models-dir" => Some(ModelArgumentKind::ModelDirectory),
        "--models-preset" => Some(ModelArgumentKind::PresetFile),
        _ => None,
    }
}

fn insert_possible_gguf_path(references: &mut BTreeSet<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    references.insert(value.trim_matches(['\"', '\'']).to_string());
    let lower = value.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(relative_end) = lower[search_from..].find(".gguf") {
        let end = search_from + relative_end + ".gguf".len();
        let mut start = 0;
        let mut quote = None;
        for (index, character) in value[..end].char_indices() {
            match quote {
                Some(active) if character == active => quote = None,
                Some(_) => {}
                None if matches!(character, '\"' | '\'') => {
                    quote = Some(character);
                    start = index + character.len_utf8();
                }
                None if matches!(character, ',' | ';' | '=') => {
                    start = index + character.len_utf8();
                }
                None => {}
            }
        }
        let candidate = value[start..end].trim().trim_matches(['\"', '\'']).trim();
        if !candidate.is_empty() {
            references.insert(candidate.to_string());
        }
        search_from = end;
    }
}

fn insert_path_with_working_directory(
    references: &mut BTreeSet<String>,
    value: &str,
    working_directory: &Path,
) {
    let value = value.trim().trim_matches(['\"', '\'']).trim();
    if value.is_empty() {
        return;
    }
    references.insert(value.to_string());
    let path = Path::new(value);
    if path.is_relative() {
        references.insert(working_directory.join(path).to_string_lossy().to_string());
    }
}

fn insert_gguf_paths_with_working_directory(
    references: &mut BTreeSet<String>,
    value: &str,
    working_directory: &Path,
) {
    let mut candidates = BTreeSet::new();
    insert_possible_gguf_path(&mut candidates, value);
    for candidate in candidates {
        insert_path_with_working_directory(references, &candidate, working_directory);
    }
}

fn preset_argument_kind(key: &str) -> Option<ModelArgumentKind> {
    match key {
        "m"
        | "model"
        | "LLAMA_ARG_MODEL"
        | "md"
        | "model-draft"
        | "spec-draft-model"
        | "LLAMA_ARG_SPEC_DRAFT_MODEL"
        | "mm"
        | "mmproj"
        | "LLAMA_ARG_MMPROJ"
        | "lora"
        | "lora-scaled"
        | "control-vector"
        | "control-vector-scaled" => Some(ModelArgumentKind::ExactPath),
        "models-dir" | "LLAMA_ARG_MODELS_DIR" => Some(ModelArgumentKind::ModelDirectory),
        "models-preset" | "LLAMA_ARG_MODELS_PRESET" => Some(ModelArgumentKind::PresetFile),
        _ => None,
    }
}

fn valid_preset_key(key: &str) -> bool {
    let mut characters = key.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
        })
}

fn strip_preset_inline_comment(value: &str) -> &str {
    for (index, character) in value.char_indices() {
        if matches!(character, ';' | '#')
            && (index == 0
                || value[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace))
        {
            return value[..index].trim_end();
        }
    }
    value.trim_end()
}

fn collect_references_from_model_preset(
    references: &mut ModelArtifactReferences,
    preset_path: &Path,
    working_directory: &Path,
) -> Result<u64, String> {
    let metadata = std::fs::metadata(preset_path).map_err(|error| {
        format!(
            "无法读取模型路由预设 {}，已拒绝在引用状态不明时删除模型: {error}",
            preset_path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "模型路由预设 {} 不是普通文件，已拒绝在引用状态不明时删除模型",
            preset_path.display()
        ));
    }
    if metadata.len() > MAX_MODEL_PRESET_BYTES {
        return Err(format!(
            "模型路由预设 {} 超过 {} MiB 安全读取上限，已拒绝删除模型",
            preset_path.display(),
            MAX_MODEL_PRESET_BYTES / 1024 / 1024
        ));
    }
    let contents = std::fs::read_to_string(preset_path).map_err(|error| {
        format!(
            "无法按 UTF-8 读取模型路由预设 {}，已拒绝在引用状态不明时删除模型: {error}",
            preset_path.display()
        )
    })?;
    let normalized = contents.replace("\r\n", "\n").replace('\r', "\n");
    for (line_index, raw_line) in normalized.lines().enumerate() {
        let line_number = line_index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let Some(close) = line.find(']') else {
                return Err(format!(
                    "模型路由预设 {} 第 {line_number} 行的节标题未闭合，已拒绝删除模型",
                    preset_path.display()
                ));
            };
            if line[1..close].trim().is_empty() {
                return Err(format!(
                    "模型路由预设 {} 第 {line_number} 行的节标题为空，已拒绝删除模型",
                    preset_path.display()
                ));
            }
            let trailing = line[close + 1..].trim_start();
            if !trailing.is_empty() && !trailing.starts_with(';') && !trailing.starts_with('#') {
                return Err(format!(
                    "模型路由预设 {} 第 {line_number} 行的节标题后存在无法解析的内容，已拒绝删除模型",
                    preset_path.display()
                ));
            }
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            return Err(format!(
                "模型路由预设 {} 第 {line_number} 行不是有效的 key=value，已拒绝删除模型",
                preset_path.display()
            ));
        };
        let key = key.trim();
        if !valid_preset_key(key) {
            return Err(format!(
                "模型路由预设 {} 第 {line_number} 行包含无效参数名，已拒绝删除模型",
                preset_path.display()
            ));
        }
        let value = strip_preset_inline_comment(raw_value.trim_start()).trim();
        if value.to_ascii_lowercase().contains(".gguf") {
            insert_gguf_paths_with_working_directory(
                &mut references.exact_paths,
                value,
                working_directory,
            );
        }
        let Some(kind) = preset_argument_kind(key) else {
            continue;
        };
        if value.is_empty() {
            return Err(format!(
                "模型路由预设 {} 第 {line_number} 行的参数 {key} 缺少路径值，已拒绝删除模型",
                preset_path.display()
            ));
        }
        match kind {
            ModelArgumentKind::ExactPath => insert_gguf_paths_with_working_directory(
                &mut references.exact_paths,
                value,
                working_directory,
            ),
            ModelArgumentKind::ModelDirectory => insert_path_with_working_directory(
                &mut references.model_directories,
                value,
                working_directory,
            ),
            ModelArgumentKind::PresetFile => insert_path_with_working_directory(
                &mut references.preset_files,
                value,
                working_directory,
            ),
        }
    }
    Ok(metadata.len())
}

fn collect_model_preset_references(references: &mut ModelArtifactReferences) -> Result<(), String> {
    if references.preset_files.is_empty() {
        return Ok(());
    }
    let working_directory = std::env::current_dir().map_err(|error| {
        format!("无法确定 llama-server 工作目录，已拒绝在引用状态不明时删除模型: {error}")
    })?;
    let mut pending = references.preset_files.iter().cloned().collect::<Vec<_>>();
    let mut queued = references.preset_files.clone();
    let mut visited = BTreeSet::new();
    let mut total_bytes = 0u64;
    while let Some(configured_path) = pending.pop() {
        let configured_path = configured_path.trim().trim_matches(['\"', '\'']).trim();
        let path = Path::new(configured_path);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            working_directory.join(path)
        };
        let identity = engine_path_identity(&resolved);
        if !visited.insert(identity) {
            continue;
        }
        if visited.len() > MAX_MODEL_PRESET_FILES {
            return Err(format!(
                "模型路由预设链超过 {MAX_MODEL_PRESET_FILES} 个文件，已拒绝删除模型"
            ));
        }
        total_bytes = total_bytes
            .checked_add(collect_references_from_model_preset(
                references,
                &resolved,
                &working_directory,
            )?)
            .ok_or_else(|| "模型路由预设总大小溢出，已拒绝删除模型".to_string())?;
        if total_bytes > MAX_MODEL_PRESET_BYTES {
            return Err(format!(
                "模型路由预设链总大小超过 {} MiB，已拒绝删除模型",
                MAX_MODEL_PRESET_BYTES / 1024 / 1024
            ));
        }
        for nested in &references.preset_files {
            if queued.insert(nested.clone()) {
                pending.push(nested.clone());
            }
        }
    }
    Ok(())
}

fn collect_model_argument_references(
    references: &mut ModelArtifactReferences,
    tokens: &[String],
    source: &str,
) -> Result<(), String> {
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if token.to_ascii_lowercase().contains(".gguf") {
            insert_possible_gguf_path(&mut references.exact_paths, token);
        }
        let (flag, inline_value) = token
            .split_once('=')
            .map_or((token.as_str(), None), |(flag, value)| (flag, Some(value)));
        let Some(kind) = model_argument_kind(flag) else {
            index += 1;
            continue;
        };
        let value = if let Some(value) = inline_value {
            if value.trim().is_empty() {
                return Err(format!("{source} 中的模型参数 {flag} 缺少路径值"));
            }
            value
        } else {
            let Some(value) = tokens.get(index + 1) else {
                return Err(format!("{source} 中的模型参数 {flag} 缺少路径值"));
            };
            if value.starts_with('-') {
                return Err(format!("{source} 中的模型参数 {flag} 缺少路径值"));
            }
            index += 1;
            value
        };
        match kind {
            ModelArgumentKind::ExactPath => {
                insert_possible_gguf_path(&mut references.exact_paths, value)
            }
            ModelArgumentKind::ModelDirectory => {
                references
                    .model_directories
                    .insert(value.trim().to_string());
            }
            ModelArgumentKind::PresetFile => {
                references.preset_files.insert(value.trim().to_string());
            }
        }
        index += 1;
    }
    Ok(())
}

fn model_artifact_references(instance: &InstanceConfig) -> Result<ModelArtifactReferences, String> {
    let mut references = ModelArtifactReferences::default();
    for value in [
        instance.model_path.as_str(),
        instance.draft_model_path.as_str(),
        instance.mmproj_path.as_str(),
        instance.lora_path.as_str(),
    ] {
        insert_possible_gguf_path(&mut references.exact_paths, value);
    }
    insert_possible_gguf_path(&mut references.exact_paths, &instance.lora_scaled);
    if !instance.models_dir.trim().is_empty() {
        references
            .model_directories
            .insert(instance.models_dir.trim().to_string());
    }
    if !instance.models_preset.trim().is_empty() {
        references
            .preset_files
            .insert(instance.models_preset.trim().to_string());
    }

    let mut custom_tokens = Vec::new();
    for row in &instance.custom_args {
        let parsed = crate::commands::server::split_args_checked(row.trim()).map_err(|error| {
            format!(
                "实例 {} 的自定义参数无法解析，已拒绝在引用状态不明时删除模型: {error}",
                instance.name
            )
        })?;
        custom_tokens.extend(parsed);
    }
    collect_model_argument_references(
        &mut references,
        &custom_tokens,
        &format!("实例 {} 的自定义参数", instance.name),
    )?;

    if !instance.manual_command.trim().is_empty() {
        let manual = crate::commands::server::split_args_checked(instance.manual_command.trim())
            .map_err(|error| {
                format!(
                    "实例 {} 的手动命令无法解析，已拒绝在引用状态不明时删除模型: {error}",
                    instance.name
                )
            })?;
        collect_model_argument_references(
            &mut references,
            &manual,
            &format!("实例 {} 的手动命令", instance.name),
        )?;
    }
    collect_model_preset_references(&mut references)?;
    Ok(references)
}

fn model_reference_matches_target(references: &ModelArtifactReferences, target: &Path) -> bool {
    let target = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let target_identity = engine_path_identity(&target);
    references
        .exact_paths
        .iter()
        .any(|candidate| engine_path_identity(Path::new(candidate)) == target_identity)
        || references.model_directories.iter().any(|directory| {
            let directory = Path::new(directory);
            let directory =
                std::fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
            path_is_within(&target, &directory)
        })
}

fn instances_referencing_models(
    instances: &HashMap<String, InstanceConfig>,
    running: &HashMap<String, RunningInstance>,
    targets: &[PathBuf],
) -> Result<Vec<String>, String> {
    let mut names = std::collections::BTreeSet::new();
    for instance in instances.values() {
        let references = model_artifact_references(instance)?;
        if targets
            .iter()
            .any(|target| model_reference_matches_target(&references, target))
        {
            names.insert(instance.name.clone());
        }
    }
    for (instance_id, running_instance) in running {
        let launch_config = running_instance.launch_config.as_ref().ok_or_else(|| {
            format!("运行中实例 {instance_id} 缺少启动配置快照，已拒绝在模型引用状态不明时删除模型")
        })?;
        let references = model_artifact_references(launch_config)?;
        if targets
            .iter()
            .any(|target| model_reference_matches_target(&references, target))
        {
            names.insert(if launch_config.name.trim().is_empty() {
                instance_id.clone()
            } else {
                launch_config.name.clone()
            });
        }
    }
    Ok(names.into_iter().collect())
}

#[cfg(test)]
fn instances_referencing_model(
    instances: &HashMap<String, InstanceConfig>,
    running: &HashMap<String, RunningInstance>,
    target: &Path,
) -> Result<Vec<String>, String> {
    instances_referencing_models(instances, running, &[target.to_path_buf()])
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelDeletionPreview {
    pub logical_path: String,
    pub artifact_paths: Vec<String>,
    pub artifact_count: u32,
    pub total_bytes: u64,
    pub is_sharded: bool,
    pub referenced_by: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelDeletionResult {
    pub artifact_paths: Vec<String>,
    pub artifact_count: u32,
    pub removed_bytes: u64,
}

fn resolve_model_deletion_artifacts(selected: &Path) -> Result<Vec<PathBuf>, String> {
    let canonical = std::fs::canonicalize(selected)
        .map_err(|error| format!("无法解析模型文件 {}: {error}", selected.display()))?;
    let canonical_parent = canonical
        .parent()
        .ok_or_else(|| "模型文件没有可验证的父目录".to_string())?;
    let artifacts = crate::model_artifacts::resolve_model_artifacts(&canonical).map_err(
        |error| match error {
            crate::model_artifacts::ModelArtifactError::Unavailable => {
                "模型文件不可用，无法建立删除清单".to_string()
            }
            crate::model_artifacts::ModelArtifactError::Incomplete => {
                "检测到不完整或冲突的模型分片组；为避免留下更多孤立分片，已拒绝删除".to_string()
            }
        },
    )?;
    let mut resolved = Vec::with_capacity(artifacts.len());
    let mut identities = HashSet::with_capacity(artifacts.len());
    for artifact in artifacts {
        let metadata = std::fs::symlink_metadata(&artifact)
            .map_err(|error| format!("无法检查模型分片 {}: {error}", artifact.display()))?;
        if crate::artifact_maintenance::metadata_is_link_like(&metadata) || !metadata.is_file() {
            return Err(format!(
                "拒绝删除链接、重解析点或非普通模型文件: {}",
                artifact.display()
            ));
        }
        let artifact = std::fs::canonicalize(&artifact)
            .map_err(|error| format!("无法解析模型分片 {}: {error}", artifact.display()))?;
        if artifact.parent() != Some(canonical_parent) {
            return Err(format!(
                "模型分片解析到了选定目录之外: {}",
                artifact.display()
            ));
        }
        if artifact
            .extension()
            .and_then(OsStr::to_str)
            .map(|extension| extension.eq_ignore_ascii_case("gguf"))
            != Some(true)
        {
            return Err(format!("模型分片不是 GGUF 文件: {}", artifact.display()));
        }
        if !identities.insert(path_identity_key(&artifact)) {
            return Err("模型分片清单包含重复的物理文件，已拒绝删除".to_string());
        }
        resolved.push(artifact);
    }
    Ok(resolved)
}

fn prepare_model_deletion(
    path: &str,
    state: &AppState,
) -> Result<(Vec<PathBuf>, ModelDeletionPreview), String> {
    let selected = Path::new(path);
    let selected_metadata =
        std::fs::symlink_metadata(selected).map_err(|error| format!("路径无效: {error}"))?;
    if crate::artifact_maintenance::metadata_is_link_like(&selected_metadata) {
        return Err("Cannot delete symlinked or reparse-point model files".to_string());
    }
    if selected
        .extension()
        .and_then(OsStr::to_str)
        .map(|extension| extension.eq_ignore_ascii_case("gguf"))
        != Some(true)
    {
        return Err("只能删除 .gguf 文件".to_string());
    }
    let canonical = crate::security::require_authorized_model_path(selected)?;
    let artifacts = resolve_model_deletion_artifacts(&canonical)?;
    let known_models = state.models.lock().unwrap();
    let is_known = known_models
        .iter()
        .filter_map(|model| std::fs::canonicalize(&model.path).ok())
        .any(|model_path| paths_equal(&model_path, &canonical));
    drop(known_models);
    if !is_known {
        return Err("文件不在已扫描的模型列表中".to_string());
    }

    let instances = state.instances.lock().unwrap().clone();
    let running = state.running.lock().unwrap().clone();
    let referenced_by = instances_referencing_models(&instances, &running, &artifacts)?;
    let mut total_bytes = 0_u64;
    for artifact in &artifacts {
        total_bytes = total_bytes.saturating_add(
            std::fs::metadata(artifact)
                .map_err(|error| format!("无法读取模型分片大小 {}: {error}", artifact.display()))?
                .len(),
        );
    }
    let artifact_paths = artifacts
        .iter()
        .map(|artifact| artifact.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let preview = ModelDeletionPreview {
        logical_path: canonical.to_string_lossy().to_string(),
        artifact_count: artifact_paths.len().try_into().unwrap_or(u32::MAX),
        total_bytes,
        is_sharded: artifact_paths.len() > 1,
        artifact_paths,
        referenced_by,
    };
    Ok((artifacts, preview))
}

#[derive(Debug)]
struct ModelArtifactRemovalFailure {
    removed: Vec<PathBuf>,
    failed_path: PathBuf,
    error: std::io::Error,
}

fn remove_model_artifact_files(
    artifacts: &[PathBuf],
) -> Result<Vec<PathBuf>, ModelArtifactRemovalFailure> {
    let mut removed = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        if let Err(error) = std::fs::remove_file(artifact) {
            return Err(ModelArtifactRemovalFailure {
                removed,
                failed_path: artifact.clone(),
                error,
            });
        }
        removed.push(artifact.clone());
    }
    Ok(removed)
}

fn instances_referencing_engine(
    instances: &HashMap<String, InstanceConfig>,
    engine_id: &str,
) -> Vec<String> {
    instances
        .values()
        .filter(|instance| paths_equal(Path::new(&instance.engine_id), Path::new(engine_id)))
        .map(|instance| instance.name.clone())
        .collect()
}

#[derive(Debug, Clone)]
struct DirectoryEntryFingerprint {
    path: PathBuf,
    name: String,
    is_dir: bool,
    is_file: bool,
    is_symlink: bool,
    size: u64,
    mtime: u64,
}

#[derive(Debug, Clone)]
struct DirectoryFingerprint {
    signature: String,
    entries: Vec<DirectoryEntryFingerprint>,
}

fn stable_hash(parts: &[String]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn read_directory_fingerprint(
    path: &Path,
    warnings: &mut Vec<String>,
) -> Result<DirectoryFingerprint, String> {
    let mut entries = Vec::new();
    for entry in
        std::fs::read_dir(path).map_err(|e| format!("failed to read {}: {}", path.display(), e))?
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!(
                    "failed to read an entry in {} and skipped it: {}",
                    path.display(),
                    error
                ));
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                warnings.push(format!(
                    "failed to read {} file type and skipped it: {}",
                    entry.path().display(),
                    error
                ));
                continue;
            }
        };
        let metadata = entry.metadata().ok();
        let modified = metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok());
        entries.push(DirectoryEntryFingerprint {
            path: entry.path(),
            name: entry.file_name().to_string_lossy().to_string(),
            is_dir: file_type.is_dir(),
            is_file: file_type.is_file(),
            is_symlink: file_type.is_symlink(),
            size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
            mtime: modified
                .map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64)
                .unwrap_or(0),
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    let parts = entries
        .iter()
        .map(|entry| {
            format!(
                "{}|{}|{}|{}|{}|{}",
                entry.name,
                if entry.is_dir {
                    "d"
                } else if entry.is_file {
                    "f"
                } else {
                    "o"
                },
                if entry.is_symlink { "l" } else { "-" },
                entry.size,
                entry.mtime,
                entry.path.extension().and_then(OsStr::to_str).unwrap_or("")
            )
        })
        .collect::<Vec<_>>();
    Ok(DirectoryFingerprint {
        signature: stable_hash(&parts),
        entries,
    })
}

#[cfg(test)]
fn path_is_under_directory(path: &Path, directory: &Path) -> bool {
    !paths_equal(path, directory) && path_is_within(path, directory)
}

fn saved_engine_name<'a>(names: &'a HashMap<String, String>, id: &str) -> Option<&'a String> {
    names.get(id).or_else(|| {
        names
            .iter()
            .find(|(saved_id, _)| paths_equal(Path::new(saved_id), Path::new(id)))
            .map(|(_, name)| name)
    })
}

const MAX_MODEL_SCAN_DEPTH: usize = 32;

#[derive(Debug)]
struct ModelDirectoryInspection {
    key: String,
    signature: String,
    entries: Vec<DirectoryEntryFingerprint>,
    children: Vec<ModelDirectoryInspection>,
}

fn inspect_model_tree_inner(
    path: &Path,
    canonical_root: &Path,
    depth: usize,
    max_depth: usize,
    warnings: &mut Vec<String>,
    visited: &mut HashSet<String>,
) -> Result<ModelDirectoryInspection, String> {
    let canonical_path = std::fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve {}: {}", path.display(), error))?;
    if !paths_equal(&canonical_path, canonical_root)
        && !path_is_within(&canonical_path, canonical_root)
    {
        return Err(format!(
            "{} resolves outside the authorized model root {}",
            path.display(),
            canonical_root.display()
        ));
    }

    let fingerprint = read_directory_fingerprint(path, warnings)?;
    let key = path_identity_key(&canonical_path);
    let mut children = Vec::new();

    for entry in &fingerprint.entries {
        if entry.is_symlink {
            warnings.push(format!(
                "{} is a symlink and was skipped",
                entry.path.display()
            ));
            continue;
        }
        if !entry.is_dir {
            continue;
        }
        if depth >= max_depth {
            warnings.push(format!(
                "{} exceeded the model scan depth limit of {} and was skipped",
                entry.path.display(),
                max_depth
            ));
            continue;
        }
        let canonical_child = match std::fs::canonicalize(&entry.path) {
            Ok(path) => path,
            Err(error) => {
                warnings.push(format!(
                    "{} could not be resolved and was skipped: {}",
                    entry.path.display(),
                    error
                ));
                continue;
            }
        };
        if !path_is_within(&canonical_child, canonical_root) {
            warnings.push(format!(
                "{} resolves outside the authorized model root and was skipped",
                entry.path.display()
            ));
            continue;
        }
        let child_key = path_identity_key(&canonical_child);
        if !visited.insert(child_key) {
            warnings.push(format!(
                "{} resolves to an already visited directory and was skipped",
                entry.path.display()
            ));
            continue;
        }
        match inspect_model_tree_inner(
            &entry.path,
            canonical_root,
            depth + 1,
            max_depth,
            warnings,
            visited,
        ) {
            Ok(child) => children.push(child),
            Err(error) => warnings.push(format!(
                "{} could not be scanned and was skipped: {}",
                entry.path.display(),
                error
            )),
        }
    }

    let mut signature_parts = vec![
        "model-tree-signature-v2".to_string(),
        format!("dir|{key}|{}", fingerprint.signature),
    ];
    signature_parts.extend(
        children
            .iter()
            .map(|child| format!("child|{}|{}", child.key, child.signature)),
    );
    signature_parts.sort();

    Ok(ModelDirectoryInspection {
        key,
        signature: stable_hash(&signature_parts),
        entries: fingerprint.entries,
        children,
    })
}

fn inspect_model_tree(
    path: &Path,
    depth: usize,
    max_depth: usize,
    warnings: &mut Vec<String>,
) -> Result<ModelDirectoryInspection, String> {
    let canonical_root = std::fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve {}: {}", path.display(), error))?;
    let mut visited = HashSet::from([path_identity_key(&canonical_root)]);
    inspect_model_tree_inner(
        path,
        &canonical_root,
        depth,
        max_depth,
        warnings,
        &mut visited,
    )
}

#[cfg(test)]
fn read_directory_tree_signature(path: &Path, max_depth: usize) -> Result<String, String> {
    inspect_model_tree(path, 0, max_depth, &mut Vec::new()).map(|tree| tree.signature)
}

fn index_cached_models_by_parent(
    inventory: &HashMap<String, InventoryModelRecord>,
) -> HashMap<String, Vec<InventoryModelRecord>> {
    let mut by_parent: HashMap<String, Vec<InventoryModelRecord>> = HashMap::new();
    for record in inventory.values() {
        let Some(parent) = Path::new(&record.path).parent() else {
            continue;
        };
        by_parent
            .entry(path_identity_key(parent))
            .or_default()
            .push(record.clone());
    }
    by_parent
}

#[allow(clippy::too_many_arguments)]
fn reuse_cached_models_for_tree(
    tree: &ModelDirectoryInspection,
    scan_root_key: &str,
    cached_by_parent: &HashMap<String, Vec<InventoryModelRecord>>,
    models: &mut Vec<ModelInfo>,
    seen_display_paths: &mut HashSet<String>,
    seen_inventory_paths: &mut HashSet<String>,
    seen_directory_keys: &mut HashSet<String>,
    inventory_meta: &mut HashMap<usize, (String, String, u64)>,
    directory_records: &mut Vec<InventoryDirectoryRecord>,
) -> usize {
    seen_directory_keys.insert(tree.key.clone());
    directory_records.push(InventoryDirectoryRecord::new(
        "model",
        tree.key.clone(),
        scan_root_key.to_string(),
        tree.signature.clone(),
    ));

    let mut reused = 0;
    if let Some(records) = cached_by_parent.get(&tree.key) {
        for record in records {
            let mut model = record.to_model_info();
            if !seen_display_paths.insert(path_identity_key(Path::new(&model.path))) {
                continue;
            }
            model.is_shard = false;
            let idx = models.len();
            seen_inventory_paths.insert(record.path.clone());
            models.push(model);
            inventory_meta.insert(
                idx,
                (record.path.clone(), scan_root_key.to_string(), record.mtime),
            );
            reused += 1;
        }
    }
    for child in &tree.children {
        reused += reuse_cached_models_for_tree(
            child,
            scan_root_key,
            cached_by_parent,
            models,
            seen_display_paths,
            seen_inventory_paths,
            seen_directory_keys,
            inventory_meta,
            directory_records,
        );
    }
    reused
}

fn reuse_cached_engines_for_root(
    scan_root_key: &str,
    inventory: &HashMap<String, InventoryEngineRecord>,
    seen_ids: &mut HashSet<String>,
    engines: &mut Vec<EngineInfo>,
    engine_records: &mut Vec<InventoryEngineRecord>,
) -> usize {
    let mut reused = 0;
    for record in inventory
        .values()
        .filter(|record| record.scan_root == scan_root_key)
    {
        if !seen_ids.insert(record.id.clone()) {
            continue;
        }
        let info = record.to_engine_info();
        engine_records.push(InventoryEngineRecord::from_engine(
            &info,
            record.exe_mtime,
            scan_root_key.to_string(),
        ));
        engines.push(info);
        reused += 1;
    }
    reused
}

#[allow(clippy::too_many_arguments)]
fn try_reuse_engine_root(
    scan_root_key: &str,
    signature: &str,
    inventory: &HashMap<String, InventoryEngineRecord>,
    directory_inventory: &HashMap<String, InventoryDirectoryRecord>,
    seen_directory_keys: &mut HashSet<String>,
    directory_records: &mut Vec<InventoryDirectoryRecord>,
    seen_inventory_ids: &mut HashSet<String>,
    engines: &mut Vec<EngineInfo>,
    engine_records: &mut Vec<InventoryEngineRecord>,
) -> Result<bool, String> {
    seen_directory_keys.insert(scan_root_key.to_string());
    directory_records.push(InventoryDirectoryRecord::new(
        "engine",
        scan_root_key.to_string(),
        scan_root_key.to_string(),
        signature.to_string(),
    ));

    let reusable = directory_inventory
        .get(scan_root_key)
        .map(|record| record.signature == signature)
        .unwrap_or(false);
    if reusable {
        reuse_cached_engines_for_root(
            scan_root_key,
            inventory,
            seen_inventory_ids,
            engines,
            engine_records,
        );
    }
    Ok(reusable)
}

fn push_indexed_engine(
    dir: &Path,
    exe: &Path,
    scan_root_key: &str,
    inventory: &HashMap<String, InventoryEngineRecord>,
    seen_inventory_ids: &mut HashSet<String>,
    engines: &mut Vec<EngineInfo>,
    engine_records: &mut Vec<InventoryEngineRecord>,
) {
    let cache_key = canonical_key(dir);
    let exe_mtime = file_mtime(exe);
    seen_inventory_ids.insert(cache_key.clone());

    if let Some(record) = inventory.get(&cache_key) {
        if record.exe_mtime == exe_mtime {
            let info = record.to_engine_info();
            engine_records.push(InventoryEngineRecord::from_engine(
                &info,
                exe_mtime,
                scan_root_key.to_string(),
            ));
            engines.push(info);
            return;
        }
    }

    if let Some(info) = build_engine_info(dir, exe, "") {
        engine_records.push(InventoryEngineRecord::from_engine(
            &info,
            exe_mtime,
            scan_root_key.to_string(),
        ));
        engines.push(info);
    }
}

fn merge_scanned_engine_capabilities(scanned: &mut [EngineInfo], current: &[EngineInfo]) {
    for engine in scanned {
        if !engine.capabilities.executable_fingerprint.is_empty()
            && !capabilities_match_executable(&engine.exe, &engine.capabilities)
        {
            engine.version.clear();
            engine.capabilities = crate::models::EngineCapabilities {
                error: Some("engine executable changed; compatibility probe required".to_string()),
                ..crate::models::EngineCapabilities::default()
            };
        }

        let Some(active) = current.iter().find(|candidate| {
            paths_equal(Path::new(&candidate.id), Path::new(&engine.id))
                && paths_equal(Path::new(&candidate.exe), Path::new(&engine.exe))
        }) else {
            continue;
        };
        if capabilities_match_executable(&engine.exe, &active.capabilities)
            && active.capabilities.probed_at.unwrap_or(0)
                >= engine.capabilities.probed_at.unwrap_or(0)
        {
            engine.version = active.version.clone();
            engine.capabilities = active.capabilities.clone();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_model_directory_incremental(
    tree: &ModelDirectoryInspection,
    scan_root_key: &str,
    inventory: &HashMap<String, InventoryModelRecord>,
    cached_by_parent: &HashMap<String, Vec<InventoryModelRecord>>,
    directory_inventory: &HashMap<String, InventoryDirectoryRecord>,
    models: &mut Vec<ModelInfo>,
    seen_display_paths: &mut HashSet<String>,
    seen_inventory_paths: &mut HashSet<String>,
    seen_directory_keys: &mut HashSet<String>,
    inventory_meta: &mut HashMap<usize, (String, String, u64)>,
    fresh_files: &mut Vec<(usize, PathBuf)>,
    directory_records: &mut Vec<InventoryDirectoryRecord>,
) -> usize {
    if directory_inventory
        .get(&tree.key)
        .map(|record| record.signature == tree.signature)
        .unwrap_or(false)
    {
        return reuse_cached_models_for_tree(
            tree,
            scan_root_key,
            cached_by_parent,
            models,
            seen_display_paths,
            seen_inventory_paths,
            seen_directory_keys,
            inventory_meta,
            directory_records,
        );
    }

    seen_directory_keys.insert(tree.key.clone());
    let mut file_count = 0;
    for entry in &tree.entries {
        if entry.is_symlink || entry.is_dir || !entry.is_file {
            continue;
        }

        let ext = entry
            .path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext != "gguf" || entry.name.starts_with('.') {
            continue;
        }

        let file_path = entry.path.to_string_lossy().to_string();
        if !seen_display_paths.insert(path_identity_key(Path::new(&file_path))) {
            continue;
        }

        let cache_key = canonical_key(&entry.path);
        seen_inventory_paths.insert(cache_key.clone());
        file_count += 1;

        if let Some(record) = inventory.get(&cache_key) {
            if record.mtime == entry.mtime && record.size == entry.size {
                let idx = models.len();
                let mut model = record.to_model_info();
                model.name = entry.name.clone();
                model.path = file_path;
                model.size = entry.size;
                model.is_shard = false;
                models.push(model);
                inventory_meta.insert(idx, (cache_key, scan_root_key.to_string(), entry.mtime));
                continue;
            }
        }

        let idx = models.len();
        models.push(ModelInfo {
            id: uuid::Uuid::new_v4().to_string(),
            name: entry.name.clone(),
            path: file_path,
            size: entry.size,
            architecture: None,
            context_length: None,
            quant_type: None,
            has_mtp_head: false,
            capabilities: ModelCapabilities::default(),
            file_type: utils::classify_gguf_file(&entry.path).to_string(),
            is_shard: false,
        });
        inventory_meta.insert(idx, (cache_key, scan_root_key.to_string(), entry.mtime));
        fresh_files.push((idx, entry.path.clone()));
    }

    for child in &tree.children {
        file_count += scan_model_directory_incremental(
            child,
            scan_root_key,
            inventory,
            cached_by_parent,
            directory_inventory,
            models,
            seen_display_paths,
            seen_inventory_paths,
            seen_directory_keys,
            inventory_meta,
            fresh_files,
            directory_records,
        );
    }

    directory_records.push(InventoryDirectoryRecord::new(
        "model",
        tree.key.clone(),
        scan_root_key.to_string(),
        tree.signature.clone(),
    ));
    file_count
}

fn mark_sharded_models(models: &mut [ModelInfo]) {
    type ShardGroupKey = (String, String);
    type ShardGroupEntry = (u32, u32, usize);
    let mut groups: HashMap<ShardGroupKey, Vec<ShardGroupEntry>> = HashMap::new();
    for (model_idx, model) in models.iter_mut().enumerate() {
        model.is_shard = false;
        let Some(descriptor) = crate::model_artifacts::parse_model_shard_name(&model.name) else {
            continue;
        };
        model.is_shard = descriptor.index != 1;
        let parent = Path::new(&model.path)
            .parent()
            .map(path_identity_key)
            .unwrap_or_default();
        groups.entry((parent, descriptor.base)).or_default().push((
            descriptor.index,
            descriptor.total,
            model_idx,
        ));
    }

    for entries in groups.values() {
        let Some(expected_total) = entries.first().map(|entry| entry.1) else {
            continue;
        };
        let mut indices = HashSet::new();
        let complete = entries.len() == expected_total as usize
            && entries.iter().all(|entry| entry.1 == expected_total)
            && entries
                .iter()
                .all(|entry| indices.insert(entry.0) && entry.0 <= expected_total)
            && (1..=expected_total).all(|index| indices.contains(&index));
        if !complete {
            continue;
        }

        let lead_idx = entries
            .iter()
            .find(|entry| entry.0 == 1)
            .map(|entry| entry.2)
            .unwrap();
        let architecture = entries
            .iter()
            .find_map(|entry| models[entry.2].architecture.clone());
        let context_length = entries
            .iter()
            .find_map(|entry| models[entry.2].context_length);
        let quant_type = entries
            .iter()
            .find_map(|entry| models[entry.2].quant_type.clone());
        let capabilities = entries
            .iter()
            .map(|entry| &models[entry.2])
            .find(|model| model.capabilities.metadata_complete)
            .map(|model| (model.has_mtp_head, model.capabilities.clone()));
        let lead = &mut models[lead_idx];
        if lead.architecture.is_none() {
            lead.architecture = architecture;
        }
        if lead.context_length.is_none() {
            lead.context_length = context_length;
        }
        if lead.quant_type.is_none() {
            lead.quant_type = quant_type;
        }
        if !lead.capabilities.metadata_complete {
            if let Some((has_mtp_head, capabilities)) = capabilities {
                lead.has_mtp_head = has_mtp_head;
                lead.capabilities = capabilities;
            }
        }
    }
}

// Cross-platform executable names.

// Cross-platform executable names.
#[cfg(target_os = "windows")]
const ENGINE_EXE_NAME: &str = "llama-server.exe";
#[cfg(not(target_os = "windows"))]
const ENGINE_EXE_NAME: &str = "llama-server";

// Engine packages commonly add vendor/backend/bin directories below the selected root.
// Keep traversal bounded so accidentally selecting a broad directory cannot recurse forever.
const MAX_ENGINE_SCAN_DEPTH: usize = 8;

#[derive(Debug)]
struct EngineTreeInspection {
    signature: String,
    executables: Vec<(PathBuf, PathBuf)>,
}

fn inspect_engine_tree(root: &Path, max_depth: usize) -> Result<EngineTreeInspection, String> {
    // The marker guarantees that inventory created by the former two-level algorithm is not
    // reused after upgrading to recursive engine discovery.
    let mut signature_parts = vec!["engine-tree-signature-v2".to_string()];
    let mut executables = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut visited = HashSet::new();
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());

    while let Some((dir, depth)) = pending.pop() {
        let canonical_dir = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !paths_equal(&canonical_dir, &canonical_root)
            && !path_is_within(&canonical_dir, &canonical_root)
        {
            continue;
        }
        let identity = path_identity_key(&canonical_dir);
        if !visited.insert(identity.clone()) {
            continue;
        }
        signature_parts.push(format!("dir|{identity}"));

        let exe = dir.join(ENGINE_EXE_NAME);
        if let Ok(metadata) = exe.metadata() {
            if metadata.is_file() {
                let canonical_exe = std::fs::canonicalize(&exe).unwrap_or_else(|_| exe.clone());
                if path_is_within(&canonical_exe, &canonical_root) {
                    let modified_ns = metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|duration| duration.as_nanos())
                        .unwrap_or(0);
                    signature_parts.push(format!(
                        "exe|{}|{}|{}",
                        canonical_key(&canonical_exe),
                        metadata.len(),
                        modified_ns
                    ));
                    executables.push((dir.clone(), exe));
                }
            }
        }

        if depth >= max_depth {
            continue;
        }

        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if depth == 0 => {
                return Err(format!("failed to read {}: {}", dir.display(), error));
            }
            Err(_) => {
                signature_parts.push(format!("unreadable|{identity}"));
                continue;
            }
        };
        let mut children = Vec::new();
        for entry in entries {
            let Ok(entry) = entry else {
                signature_parts.push(format!("entry-error|{identity}"));
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                signature_parts.push(format!("type-error|{}", path_identity_key(&entry.path())));
                continue;
            };
            if file_type.is_dir() && !file_type.is_symlink() {
                let child = entry.path();
                signature_parts.push(format!("child|{}", engine_path_identity(&child)));
                children.push(child);
            }
        }
        children.sort();
        pending.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
    }

    signature_parts.sort();
    Ok(EngineTreeInspection {
        signature: stable_hash(&signature_parts),
        executables,
    })
}

// Engine info construction.
pub fn build_engine_info(dir: &Path, exe: &Path, _source: &str) -> Option<EngineInfo> {
    let name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("llama-server")
        .to_string();
    let version = name.clone();
    let backend = utils::detect_backend(dir);
    // #10: Use the canonical directory path as a stable ID so moved directories do not break references.
    let id = std::fs::canonicalize(dir)
        .unwrap_or_else(|_| dir.to_path_buf())
        .to_string_lossy()
        .to_string();
    Some(EngineInfo {
        id,
        name: format!("{} ({})", name, backend),
        dir: dir.to_string_lossy().to_string(),
        exe: exe.to_string_lossy().to_string(),
        version,
        backend,
        custom_name: None,
        capabilities: Default::default(),
    })
}

// Model scanning.
struct ModelScanWork {
    models: Vec<ModelInfo>,
    records: Vec<InventoryModelRecord>,
    directory_records: Vec<InventoryDirectoryRecord>,
    scan_root_keys: HashSet<String>,
    seen_inventory_paths: HashSet<String>,
    seen_directory_keys: HashSet<String>,
    warnings: Vec<String>,
}

pub async fn scan_models(
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
    _app: tauri::AppHandle,
) -> Result<Vec<ModelInfo>, String> {
    let _scan_guard = MODEL_SCAN_LOCK.lock().await;
    let generation = state.model_scan_generation.fetch_add(1, Ordering::AcqRel) + 1;
    let app_dir = utils::get_data_dir();
    let default_path = utils::get_default_models_dir();
    let default_path_for_check = default_path.clone();

    let scan_paths: Vec<PathBuf> = if paths.is_empty() {
        vec![default_path]
    } else {
        paths
            .iter()
            .map(|p| {
                let pb = PathBuf::from(p);
                if pb.is_relative() {
                    app_dir.join(p)
                } else {
                    pb
                }
            })
            .collect()
    };
    let scan_paths = scan_paths
        .into_iter()
        .map(|path| {
            if paths_equal(&path, &default_path_for_check) && !path.exists() {
                Ok(path)
            } else {
                crate::security::require_authorized_model_root(&path)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut seen_scan_roots = HashSet::new();
    let scan_paths = scan_paths
        .into_iter()
        .filter(|path| seen_scan_roots.insert(path_identity_key(path)))
        .collect::<Vec<_>>();

    let result = tokio::task::spawn_blocking(move || -> Result<ModelScanWork, Vec<String>> {
        let (inventory, directory_inventory) =
            model_inventory::load_model_scan_indexes().map_err(|err| vec![err])?;
        let cached_by_parent = index_cached_models_by_parent(&inventory);
        let mut models: Vec<ModelInfo> = Vec::new();
        let mut seen_display_paths = HashSet::new();
        let mut seen_inventory_paths = HashSet::new();
        let mut seen_directory_keys = HashSet::new();
        let mut scan_root_keys = HashSet::new();
        let mut inventory_meta: HashMap<usize, (String, String, u64)> = HashMap::new();
        let mut fatal_errors = Vec::new();
        let mut warnings = Vec::new();
        let mut fresh_files: Vec<(usize, PathBuf)> = Vec::new();
        let mut directory_records: Vec<InventoryDirectoryRecord> = Vec::new();

        for scan_root in &scan_paths {
            let root_str = scan_root.display().to_string();
            if !scan_root.exists() {
                if paths_equal(scan_root, &default_path_for_check) {
                    continue;
                }
                fatal_errors.push(format!("{} does not exist", root_str));
                continue;
            }
            if !scan_root.is_dir() {
                fatal_errors.push(format!("{} is not a directory", root_str));
                continue;
            }

            let scan_root_key = canonical_key(scan_root);
            scan_root_keys.insert(scan_root_key.clone());
            let tree = match inspect_model_tree(scan_root, 0, MAX_MODEL_SCAN_DEPTH, &mut warnings) {
                Ok(tree) => tree,
                Err(error) => {
                    fatal_errors.push(error);
                    continue;
                }
            };
            let file_count = scan_model_directory_incremental(
                &tree,
                &scan_root_key,
                &inventory,
                &cached_by_parent,
                &directory_inventory,
                &mut models,
                &mut seen_display_paths,
                &mut seen_inventory_paths,
                &mut seen_directory_keys,
                &mut inventory_meta,
                &mut fresh_files,
                &mut directory_records,
            );

            if file_count == 0 {
                warnings.push(format!("{} contains no .gguf model files", root_str));
            }
        }

        if !fatal_errors.is_empty() {
            return Err(fatal_errors);
        }

        if !fresh_files.is_empty() {
            const MAX_GGUF_PARSE_WORKERS: usize = 4;
            let worker_count = std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(MAX_GGUF_PARSE_WORKERS)
                .clamp(1, MAX_GGUF_PARSE_WORKERS)
                .min(fresh_files.len());
            let chunk_size = fresh_files.len().div_ceil(worker_count);
            type MetadataParseResult = (
                usize,
                PathBuf,
                Result<crate::models::GgufMetadataSummary, String>,
            );
            std::thread::scope(|scope| {
                let (sender, receiver) =
                    std::sync::mpsc::sync_channel::<MetadataParseResult>(worker_count);
                for chunk in fresh_files.chunks(chunk_size) {
                    let chunk: Vec<_> = chunk.to_vec();
                    let sender = sender.clone();
                    scope.spawn(move || {
                        for (model_idx, path) in chunk {
                            if sender
                                .send((model_idx, path.clone(), utils::parse_gguf_metadata(&path)))
                                .is_err()
                            {
                                break;
                            }
                        }
                    });
                }
                drop(sender);

                for (model_idx, path, summary_result) in receiver {
                    if model_idx >= models.len() {
                        continue;
                    }
                    let summary = match summary_result {
                        Ok(summary) => summary,
                        Err(err) => {
                            warnings.push(format!(
                                "{} metadata parse failed: {}",
                                path.display(),
                                err
                            ));
                            continue;
                        }
                    };
                    let model = &mut models[model_idx];
                    model.architecture = summary.architecture;
                    model.context_length = summary.context_length;
                    model.quant_type = summary.quant_type;
                    model.has_mtp_head = summary.capabilities.has_builtin_mtp;
                    model.capabilities = summary.capabilities;
                }
            });
        }

        mark_sharded_models(&mut models);

        let records = models
            .iter()
            .enumerate()
            .filter_map(|(idx, model)| {
                inventory_meta
                    .get(&idx)
                    .map(|(cache_key, scan_root, mtime)| {
                        InventoryModelRecord::from_model(
                            model,
                            cache_key.clone(),
                            scan_root.clone(),
                            *mtime,
                        )
                    })
            })
            .collect::<Vec<_>>();
        if models.is_empty() && !warnings.is_empty() {
            Err(warnings)
        } else {
            Ok(ModelScanWork {
                models,
                records,
                directory_records,
                scan_root_keys,
                seen_inventory_paths,
                seen_directory_keys,
                warnings,
            })
        }
    })
    .await
    .map_err(|e| format!("scan thread failed: {:?}", e))?;

    let work = match result {
        Ok(work) => work,
        Err(errors) => return Err(errors.join("; ")),
    };

    if state.model_scan_generation.load(Ordering::Acquire) != generation {
        return Ok(state.models.lock().unwrap().clone());
    }
    let ModelScanWork {
        models,
        records,
        directory_records,
        scan_root_keys,
        seen_inventory_paths,
        seen_directory_keys,
        warnings,
    } = work;
    let models = tokio::task::spawn_blocking(move || {
        model_inventory::apply_model_scan(
            &records,
            &directory_records,
            &scan_root_keys,
            &seen_inventory_paths,
            &seen_directory_keys,
        )?;
        Ok::<_, String>(models)
    })
    .await
    .map_err(|error| format!("model inventory commit worker failed: {error}"))??;
    for warning in warnings {
        eprintln!("model scan warning: {warning}");
    }
    let mut state_models = state.models.lock().unwrap();
    *state_models = models.clone();
    Ok(models)
}

// Batch load: scan and restore downloads in one IPC call.
type AppDataSnapshot = (
    Vec<ModelInfo>,
    Vec<EngineInfo>,
    Vec<crate::models::PersistedQueueEntry>,
);
type CachedScan = (Vec<ModelInfo>, Vec<EngineInfo>);

pub async fn load_app_data(
    paths: Vec<String>,
    engine_paths: Vec<String>,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<AppDataSnapshot, String> {
    let (models_result, engines_result) = tokio::join!(
        scan_models(paths, state.clone(), app.clone()),
        scan_engines(engine_paths, state.clone())
    );
    let models = models_result.unwrap_or_else(|_| Vec::new());
    let engines = engines_result.unwrap_or_else(|_| Vec::new());
    let queue = crate::commands::download::restore_runtime_queue_from_disk(&state, &app);
    Ok((models, engines, queue))
}

/// Reads cached scan results from disk so startup can show data before a full scan finishes.
pub async fn get_cached_scan(
    state: tauri::State<'_, AppState>,
) -> Result<Option<CachedScan>, String> {
    let mut models = model_inventory::list_cached_models()?;
    mark_sharded_models(&mut models);

    let mut engines = model_inventory::list_cached_engines()?;
    {
        let saved_names = state.engine_names.lock().unwrap();
        for engine in &mut engines {
            if let Some(cn) = saved_engine_name(&saved_names, &engine.id) {
                engine.custom_name = Some(cn.clone());
                engine.name = cn.clone();
            }
        }
    }

    if models.is_empty() && engines.is_empty() {
        return Ok(None);
    }

    // Write into state so other components can use it immediately.
    {
        let mut state_models = state.models.lock().unwrap();
        *state_models = models.clone();
        let mut state_engines = state.engines.lock().unwrap();
        *state_engines = engines.clone();
    }

    Ok(Some((models, engines)))
}

pub async fn get_models(state: tauri::State<'_, AppState>) -> Result<Vec<ModelInfo>, String> {
    Ok(state.models.lock().unwrap().clone())
}

pub async fn preview_model_deletion(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<ModelDeletionPreview, String> {
    prepare_model_deletion(&path, &state).map(|(_, preview)| preview)
}

pub async fn delete_model_file(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<ModelDeletionResult, String> {
    // Rebuild the complete artifact set at execution time. The preview is only
    // explanatory and never grants authority to a stale set of paths.
    let (artifacts, preview) = prepare_model_deletion(&path, &state)?;
    if !preview.referenced_by.is_empty() {
        return Err(format!(
            "模型文件正被实例引用，无法删除: {}",
            preview.referenced_by.join(", ")
        ));
    }

    let removed =
        match remove_model_artifact_files(&artifacts) {
            Ok(removed) => removed,
            Err(failure) => {
                for removed_artifact in &failure.removed {
                    let _ = model_inventory::delete_model(&removed_artifact.to_string_lossy());
                }
                let removed_keys = failure
                    .removed
                    .iter()
                    .map(|removed_artifact| path_identity_key(removed_artifact))
                    .collect::<HashSet<_>>();
                state.models.lock().unwrap().retain(|model| {
                    !removed_keys.contains(&path_identity_key(Path::new(&model.path)))
                });
                return Err(format!(
                    "删除模型分片失败: {}: {}。已删除 {}/{} 个文件，请重新扫描模型目录后重试。",
                    failure.failed_path.display(),
                    failure.error,
                    failure.removed.len(),
                    artifacts.len()
                ));
            }
        };

    let removed_keys = removed
        .iter()
        .map(|artifact| path_identity_key(artifact))
        .collect::<HashSet<_>>();
    for artifact in &removed {
        let _ = model_inventory::delete_model(&artifact.to_string_lossy());
    }
    state
        .models
        .lock()
        .unwrap()
        .retain(|model| !removed_keys.contains(&path_identity_key(Path::new(&model.path))));
    Ok(ModelDeletionResult {
        artifact_paths: preview.artifact_paths,
        artifact_count: preview.artifact_count,
        removed_bytes: preview.total_bytes,
    })
}

pub async fn open_model_folder(path: String) -> Result<(), String> {
    let canonical = crate::security::require_authorized_model_path(Path::new(&path))?;
    let parent = canonical.parent().unwrap_or(Path::new("."));
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(parent)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(parent)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    Ok(())
}

pub async fn read_gguf_metadata(
    path: String,
) -> Result<crate::models::GgufMetadataSummary, String> {
    let canonical = crate::security::require_authorized_model_path(Path::new(&path))?;
    tokio::task::spawn_blocking(move || utils::parse_gguf_metadata(&canonical))
        .await
        .map_err(|error| format!("GGUF metadata worker failed: {error}"))?
}

// Engine scanning.
struct EngineScanWork {
    engines: Vec<EngineInfo>,
    records: Vec<InventoryEngineRecord>,
    directory_records: Vec<InventoryDirectoryRecord>,
    scan_root_keys: HashSet<String>,
    seen_inventory_ids: HashSet<String>,
    seen_directory_keys: HashSet<String>,
}

pub async fn scan_engines(
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<EngineInfo>, String> {
    let _scan_guard = ENGINE_SCAN_LOCK.lock().await;
    let generation = state.engine_scan_generation.fetch_add(1, Ordering::AcqRel) + 1;
    let paths = paths
        .into_iter()
        .map(|path| {
            crate::security::require_authorized_engine_root(Path::new(&path))
                .map(|canonical| canonical.to_string_lossy().to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut seen_scan_roots = HashSet::new();
    let paths = paths
        .into_iter()
        .filter(|path| seen_scan_roots.insert(path_identity_key(Path::new(path))))
        .collect::<Vec<_>>();
    let work = tokio::task::spawn_blocking(move || -> Result<EngineScanWork, String> {
        let (inventory, directory_inventory) = model_inventory::load_engine_scan_indexes()?;
        let mut engines: Vec<EngineInfo> = Vec::new();
        let mut engine_records: Vec<InventoryEngineRecord> = Vec::new();
        let mut directory_records: Vec<InventoryDirectoryRecord> = Vec::new();
        let mut seen = HashSet::new();
        let mut seen_inventory_ids = HashSet::new();
        let mut seen_directory_keys = HashSet::new();
        let mut scan_root_keys = HashSet::new();
        let app_dir = utils::get_data_dir();

        let engines_dir = app_dir.join("engines");
        if engines_dir.exists() {
            let scan_root_key = canonical_key(&engines_dir);
            scan_root_keys.insert(scan_root_key.clone());
            let inspection = inspect_engine_tree(&engines_dir, MAX_ENGINE_SCAN_DEPTH)?;
            if try_reuse_engine_root(
                &scan_root_key,
                &inspection.signature,
                &inventory,
                &directory_inventory,
                &mut seen_directory_keys,
                &mut directory_records,
                &mut seen_inventory_ids,
                &mut engines,
                &mut engine_records,
            )? {
                // Cached entries for this unchanged root have already been appended.
            } else {
                for (dir, exe) in inspection.executables {
                    let norm = engine_path_identity(&dir);
                    if seen.insert(norm) {
                        push_indexed_engine(
                            &dir,
                            &exe,
                            &scan_root_key,
                            &inventory,
                            &mut seen_inventory_ids,
                            &mut engines,
                            &mut engine_records,
                        );
                    }
                }
            }
        }

        for p in &paths {
            let root = PathBuf::from(p);
            if !root.exists() || !root.is_dir() {
                continue;
            }
            let scan_root_key = canonical_key(&root);
            scan_root_keys.insert(scan_root_key.clone());
            let inspection = inspect_engine_tree(&root, MAX_ENGINE_SCAN_DEPTH)?;
            if try_reuse_engine_root(
                &scan_root_key,
                &inspection.signature,
                &inventory,
                &directory_inventory,
                &mut seen_directory_keys,
                &mut directory_records,
                &mut seen_inventory_ids,
                &mut engines,
                &mut engine_records,
            )? {
                continue;
            }
            for (dir, exe) in inspection.executables {
                let norm = engine_path_identity(&dir);
                if seen.insert(norm) {
                    push_indexed_engine(
                        &dir,
                        &exe,
                        &scan_root_key,
                        &inventory,
                        &mut seen_inventory_ids,
                        &mut engines,
                        &mut engine_records,
                    );
                }
            }
        }

        Ok(EngineScanWork {
            engines,
            records: engine_records,
            directory_records,
            scan_root_keys,
            seen_inventory_ids,
            seen_directory_keys,
        })
    })
    .await
    .map_err(|e| format!("scan thread failed: {}", e))??;

    if state.engine_scan_generation.load(Ordering::Acquire) != generation {
        return Ok(state.engines.lock().unwrap().clone());
    }
    let EngineScanWork {
        mut engines,
        records,
        directory_records,
        scan_root_keys,
        seen_inventory_ids,
        seen_directory_keys,
    } = work;
    tokio::task::spawn_blocking(move || {
        model_inventory::apply_engine_scan(
            &records,
            &directory_records,
            &scan_root_keys,
            &seen_inventory_ids,
            &seen_directory_keys,
        )
    })
    .await
    .map_err(|error| format!("engine inventory commit worker failed: {error}"))??;

    // Preserve custom engine names; state access stays outside spawn_blocking.
    {
        let saved_names = state.engine_names.lock().unwrap();
        for engine in &mut engines {
            if let Some(cn) = saved_engine_name(&saved_names, &engine.id) {
                engine.custom_name = Some(cn.clone());
                engine.name = cn.clone();
            }
        }
    }

    let finalized = {
        let mut state_engines = state.engines.lock().unwrap();
        merge_scanned_engine_capabilities(&mut engines, &state_engines);
        *state_engines = engines.clone();
        // Keep the state lock through persistence. A concurrent probe uses the same lock for its
        // state and cache update, so an older scan snapshot cannot be written after a newer probe.
        for engine in state_engines.iter() {
            let _ = model_inventory::update_engine_probe(engine);
        }
        state_engines.clone()
    };
    Ok(finalized)
}

pub async fn get_engines(state: tauri::State<'_, AppState>) -> Result<Vec<EngineInfo>, String> {
    Ok(state.engines.lock().unwrap().clone())
}

#[cfg(test)]
mod incremental_scan_tests {
    use super::*;

    fn scanned_model(name: &str, parent: &str) -> ModelInfo {
        ModelInfo {
            id: name.to_string(),
            name: name.to_string(),
            path: Path::new(parent).join(name).to_string_lossy().to_string(),
            size: 1,
            architecture: None,
            context_length: None,
            quant_type: None,
            has_mtp_head: false,
            capabilities: ModelCapabilities::default(),
            file_type: "model".into(),
            is_shard: false,
        }
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lsm-incremental-{}-{}", name, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn directory_fingerprint_changes_when_model_file_is_added() {
        let dir = temp_test_dir("fingerprint");
        let initial = read_directory_fingerprint(&dir, &mut Vec::new()).unwrap();

        std::fs::write(dir.join("model.gguf"), b"test").unwrap();

        let updated = read_directory_fingerprint(&dir, &mut Vec::new()).unwrap();
        assert_ne!(initial.signature, updated.signature);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn complete_shards_have_one_logical_entry_and_propagate_metadata() {
        let mut models = vec![
            scanned_model("Qwen-00001-of-00003.gguf", "models-a"),
            scanned_model("Qwen-00002-of-00003.gguf", "models-a"),
            scanned_model("Qwen-00003-of-00003.gguf", "models-a"),
        ];
        models[0].architecture = Some("qwen4exp".into());
        models[1].context_length = Some(262_144);
        models[2].quant_type = Some("IQ4_XS".into());
        models[1].capabilities.metadata_complete = true;

        mark_sharded_models(&mut models);

        assert!(!models[0].is_shard);
        assert!(models[1].is_shard);
        assert!(models[2].is_shard);
        assert_eq!(models[0].architecture.as_deref(), Some("qwen4exp"));
        assert_eq!(models[0].context_length, Some(262_144));
        assert_eq!(models[0].quant_type.as_deref(), Some("IQ4_XS"));
        assert!(models[0].capabilities.metadata_complete);
    }

    #[test]
    fn shard_grouping_never_crosses_parent_directories() {
        let mut models = vec![
            scanned_model("Same-00001-of-00002.gguf", "models-a"),
            scanned_model("Same-00002-of-00002.gguf", "models-b"),
        ];
        models[1].architecture = Some("wrong-parent".into());

        mark_sharded_models(&mut models);

        assert!(!models[0].is_shard);
        assert!(models[1].is_shard);
        assert!(models[0].architecture.is_none());
    }

    #[test]
    fn deletion_preview_resolves_every_physical_model_shard() {
        let dir = temp_test_dir("delete-shards");
        let first = dir.join("Qwen-00001-of-00003.gguf");
        let second = dir.join("Qwen-00002-of-00003.gguf");
        let third = dir.join("Qwen-00003-of-00003.gguf");
        for path in [&first, &second, &third] {
            std::fs::write(path, b"shard").unwrap();
        }

        assert_eq!(
            resolve_model_deletion_artifacts(&first).unwrap(),
            vec![
                std::fs::canonicalize(&first).unwrap(),
                std::fs::canonicalize(&second).unwrap(),
                std::fs::canonicalize(&third).unwrap(),
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deletion_preview_rejects_an_incomplete_model_shard_set() {
        let dir = temp_test_dir("delete-incomplete-shards");
        let first = dir.join("Qwen-00001-of-00003.gguf");
        std::fs::write(&first, b"shard").unwrap();
        std::fs::write(dir.join("Qwen-00003-of-00003.gguf"), b"shard").unwrap();

        let error = resolve_model_deletion_artifacts(&first).unwrap_err();
        assert!(error.contains("不完整"));
        assert!(first.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn artifact_set_deletion_removes_all_shards_and_preserves_siblings() {
        let dir = temp_test_dir("delete-shard-set");
        let first = dir.join("Qwen-00001-of-00002.gguf");
        let second = dir.join("Qwen-00002-of-00002.gguf");
        let sibling = dir.join("Other.gguf");
        for path in [&first, &second, &sibling] {
            std::fs::write(path, b"model").unwrap();
        }
        let artifacts = resolve_model_deletion_artifacts(&first).unwrap();

        assert_eq!(remove_model_artifact_files(&artifacts).unwrap().len(), 2);
        assert!(!first.exists());
        assert!(!second.exists());
        assert!(sibling.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn references_to_any_physical_shard_block_artifact_set_deletion() {
        let instances = HashMap::from([(
            "draft".to_string(),
            InstanceConfig {
                name: "Draft".into(),
                draft_model_path: "/models/Qwen-00002-of-00003.gguf".into(),
                ..InstanceConfig::default()
            },
        )]);

        assert_eq!(
            instances_referencing_model(
                &instances,
                &HashMap::new(),
                Path::new("/models/Qwen-00002-of-00003.gguf")
            )
            .unwrap(),
            vec!["Draft"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn model_tree_does_not_follow_junctions_outside_the_scan_root() {
        let root = temp_test_dir("model-root");
        let outside = temp_test_dir("model-outside");
        std::fs::write(outside.join("external.gguf"), b"outside").unwrap();
        let junction = root.join("linked");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .status()
            .unwrap();
        assert!(status.success());

        let mut warnings = Vec::new();
        let tree = inspect_model_tree(&root, 0, MAX_MODEL_SCAN_DEPTH, &mut warnings).unwrap();
        assert!(tree.children.is_empty());
        assert!(warnings.iter().any(|warning| {
            warning.contains("outside the authorized model root")
                || warning.contains("symlink and was skipped")
        }));

        std::fs::remove_dir(&junction).unwrap();
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn path_under_directory_uses_component_boundaries() {
        let parent = PathBuf::from("models").join("foo");
        let child = parent.join("bar").join("model.gguf");
        let sibling = PathBuf::from("models").join("foo2").join("model.gguf");

        assert!(path_is_under_directory(&child, &parent));
        assert!(!path_is_under_directory(&sibling, &parent));
    }

    #[test]
    fn scan_merge_preserves_a_newer_probe_and_invalidates_changed_executables() {
        let dir = temp_test_dir("engine-capability-merge");
        let exe = dir.join("llama-server-test");
        std::fs::write(&exe, vec![b'a'; 128 * 1024]).unwrap();
        let fingerprint =
            crate::commands::engine_capabilities::executable_fingerprint(&exe.to_string_lossy());
        let mut current = EngineInfo {
            id: "engine-1".to_string(),
            name: "engine".to_string(),
            dir: dir.to_string_lossy().to_string(),
            exe: exe.to_string_lossy().to_string(),
            version: "version: 100".to_string(),
            backend: "CPU".to_string(),
            custom_name: None,
            capabilities: crate::models::EngineCapabilities {
                status: "detected".to_string(),
                version_status: "detected".to_string(),
                executable_fingerprint: fingerprint,
                probed_at: Some(100),
                ..crate::models::EngineCapabilities::default()
            },
        };
        let mut scanned = vec![EngineInfo {
            capabilities: crate::models::EngineCapabilities::default(),
            version: String::new(),
            ..current.clone()
        }];
        merge_scanned_engine_capabilities(&mut scanned, &[current.clone()]);
        assert_eq!(scanned[0].version, "version: 100");
        assert_eq!(scanned[0].capabilities.status, "detected");

        std::fs::write(&exe, vec![b'b'; 128 * 1024]).unwrap();
        current.capabilities.probed_at = Some(200);
        merge_scanned_engine_capabilities(&mut scanned, &[current]);
        assert_eq!(scanned[0].capabilities.status, "unprobed");
        assert!(scanned[0].version.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tree_signature_changes_for_nested_engine_file() {
        let dir = temp_test_dir("engine-tree");
        let nested = dir.join("vendor").join("backend").join("bin");
        std::fs::create_dir_all(&nested).unwrap();
        let initial = inspect_engine_tree(&dir, MAX_ENGINE_SCAN_DEPTH)
            .unwrap()
            .signature;

        std::fs::write(nested.join(ENGINE_EXE_NAME), b"exe").unwrap();

        let updated = inspect_engine_tree(&dir, MAX_ENGINE_SCAN_DEPTH)
            .unwrap()
            .signature;
        assert_ne!(initial, updated);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn engine_scan_discovers_nested_vendor_backend_layouts() {
        let dir = temp_test_dir("engine-depth");
        let atomic = dir.join("atomic");
        let hip = dir.join("poolside").join("hip-rocm714").join("bin");
        let vulkan = dir.join("poolside").join("vulkan").join("bin");
        for engine_dir in [&atomic, &hip, &vulkan] {
            std::fs::create_dir_all(engine_dir).unwrap();
            std::fs::write(engine_dir.join(ENGINE_EXE_NAME), b"exe").unwrap();
        }

        let discovered = inspect_engine_tree(&dir, MAX_ENGINE_SCAN_DEPTH)
            .unwrap()
            .executables;
        let discovered_dirs = discovered
            .iter()
            .map(|(engine_dir, _)| engine_path_identity(engine_dir))
            .collect::<HashSet<_>>();

        assert_eq!(discovered.len(), 3);
        assert!(discovered_dirs.contains(&engine_path_identity(&atomic)));
        assert!(discovered_dirs.contains(&engine_path_identity(&hip)));
        assert!(discovered_dirs.contains(&engine_path_identity(&vulkan)));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unchanged_engine_root_reuses_cached_inventory() {
        let root = temp_test_dir("engine-cache");
        let engine_dir = root.join("vendor").join("bin");
        std::fs::create_dir_all(&engine_dir).unwrap();
        std::fs::write(engine_dir.join(ENGINE_EXE_NAME), b"exe").unwrap();

        let scan_root_key = canonical_key(&root);
        let inspection = inspect_engine_tree(&root, MAX_ENGINE_SCAN_DEPTH).unwrap();
        let signature = inspection.signature.clone();
        let mut engines = Vec::new();
        let mut engine_records = Vec::new();
        let mut directory_records = Vec::new();
        let mut seen_inventory_ids = HashSet::new();
        let mut seen_directory_keys = HashSet::new();

        assert!(!try_reuse_engine_root(
            &scan_root_key,
            &signature,
            &HashMap::new(),
            &HashMap::new(),
            &mut seen_directory_keys,
            &mut directory_records,
            &mut seen_inventory_ids,
            &mut engines,
            &mut engine_records,
        )
        .unwrap());
        for (dir, exe) in inspection.executables {
            push_indexed_engine(
                &dir,
                &exe,
                &scan_root_key,
                &HashMap::new(),
                &mut seen_inventory_ids,
                &mut engines,
                &mut engine_records,
            );
        }
        assert_eq!(engines.len(), 1);

        let inventory = engine_records
            .iter()
            .cloned()
            .map(|record| (record.id.clone(), record))
            .collect::<HashMap<_, _>>();
        let directory_inventory = directory_records
            .iter()
            .cloned()
            .map(|record| (record.path.clone(), record))
            .collect::<HashMap<_, _>>();
        engines.clear();
        engine_records.clear();
        directory_records.clear();
        seen_inventory_ids.clear();
        seen_directory_keys.clear();

        assert!(try_reuse_engine_root(
            &scan_root_key,
            &signature,
            &inventory,
            &directory_inventory,
            &mut seen_directory_keys,
            &mut directory_records,
            &mut seen_inventory_ids,
            &mut engines,
            &mut engine_records,
        )
        .unwrap());
        assert_eq!(engines.len(), 1);
        assert_eq!(engine_records.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn engine_scan_stops_at_the_configured_depth_limit() {
        let dir = temp_test_dir("engine-depth-limit");
        let mut too_deep = dir.clone();
        for depth in 0..=MAX_ENGINE_SCAN_DEPTH {
            too_deep = too_deep.join(format!("level-{depth}"));
        }
        std::fs::create_dir_all(&too_deep).unwrap();
        std::fs::write(too_deep.join(ENGINE_EXE_NAME), b"exe").unwrap();

        assert!(inspect_engine_tree(&dir, MAX_ENGINE_SCAN_DEPTH)
            .unwrap()
            .executables
            .is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tree_signature_changes_when_a_deep_model_is_rewritten() {
        let dir = temp_test_dir("model-tree");
        let nested = dir.join("vendor").join("family").join("quant");
        std::fs::create_dir_all(&nested).unwrap();
        let model = nested.join("model.gguf");
        std::fs::write(&model, b"first-model-payload").unwrap();
        let initial = read_directory_tree_signature(&dir, MAX_MODEL_SCAN_DEPTH).unwrap();

        // Use a different size so the assertion stays valid on filesystems whose
        // modification timestamps are coarser than the test's execution time.
        std::fs::write(&model, b"other-longer-model-payload").unwrap();

        let updated = read_directory_tree_signature(&dir, MAX_MODEL_SCAN_DEPTH).unwrap();
        assert_ne!(initial, updated);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unchanged_failed_metadata_record_is_reused_without_reparsing() {
        let dir = temp_test_dir("malformed-model-cache");
        std::fs::write(dir.join("malformed.gguf"), b"not-a-gguf").unwrap();
        let scan_root_key = canonical_key(&dir);
        let mut models = Vec::new();
        let mut seen_display_paths = HashSet::new();
        let mut seen_inventory_paths = HashSet::new();
        let mut seen_directory_keys = HashSet::new();
        let mut inventory_meta = HashMap::new();
        let mut fresh_files = Vec::new();
        let mut directory_records = Vec::new();
        let tree = inspect_model_tree(&dir, 0, MAX_MODEL_SCAN_DEPTH, &mut Vec::new()).unwrap();

        scan_model_directory_incremental(
            &tree,
            &scan_root_key,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &mut models,
            &mut seen_display_paths,
            &mut seen_inventory_paths,
            &mut seen_directory_keys,
            &mut inventory_meta,
            &mut fresh_files,
            &mut directory_records,
        );
        assert_eq!(fresh_files.len(), 1);
        let (cache_key, stored_root, mtime) = inventory_meta.get(&0).unwrap().clone();
        let cached =
            InventoryModelRecord::from_model(&models[0], cache_key.clone(), stored_root, mtime);
        let inventory = HashMap::from([(cache_key, cached)]);
        let directory_inventory = directory_records
            .iter()
            .cloned()
            .map(|record| (record.path.clone(), record))
            .collect::<HashMap<_, _>>();

        models.clear();
        seen_display_paths.clear();
        seen_inventory_paths.clear();
        seen_directory_keys.clear();
        inventory_meta.clear();
        fresh_files.clear();
        directory_records.clear();
        let cached_by_parent = index_cached_models_by_parent(&inventory);
        let reused_count = scan_model_directory_incremental(
            &tree,
            &scan_root_key,
            &inventory,
            &cached_by_parent,
            &directory_inventory,
            &mut models,
            &mut seen_display_paths,
            &mut seen_inventory_paths,
            &mut seen_directory_keys,
            &mut inventory_meta,
            &mut fresh_files,
            &mut directory_records,
        );

        assert_eq!(reused_count, 1);
        assert_eq!(models.len(), 1);
        assert!(fresh_files.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn engine_path_identity_respects_platform_case_rules() {
        let upper = Path::new("/opt/llama/CUDA");
        let lower = Path::new("/opt/llama/cuda");
        #[cfg(target_os = "windows")]
        assert_eq!(engine_path_identity(upper), engine_path_identity(lower));
        #[cfg(not(target_os = "windows"))]
        assert_ne!(engine_path_identity(upper), engine_path_identity(lower));
    }

    #[test]
    fn referenced_models_and_engines_are_identified_before_deletion() {
        let mut instances = HashMap::new();
        instances.insert(
            "primary".into(),
            InstanceConfig {
                name: "Primary".into(),
                model_path: "/models/chat.gguf".into(),
                engine_id: "engine-1".into(),
                ..InstanceConfig::default()
            },
        );

        assert_eq!(
            instances_referencing_model(
                &instances,
                &HashMap::new(),
                Path::new("/models/chat.gguf")
            )
            .unwrap(),
            vec!["Primary"]
        );
        assert_eq!(
            instances_referencing_engine(&instances, "engine-1"),
            vec!["Primary"]
        );
        assert!(instances_referencing_engine(&instances, "engine-2").is_empty());
    }

    #[test]
    fn model_deletion_references_cover_custom_manual_lora_and_models_directory() {
        let models_dir = temp_test_dir("delete-reference-sources");
        let target = models_dir.join("target model.gguf");
        std::fs::write(&target, b"model").unwrap();
        let target_text = target.to_string_lossy().to_string();
        let directory_text = models_dir.to_string_lossy().to_string();
        let instances = HashMap::from([
            (
                "custom".to_string(),
                InstanceConfig {
                    name: "Custom".into(),
                    custom_args: vec![format!("--model=\"{target_text}\"")],
                    ..InstanceConfig::default()
                },
            ),
            (
                "directory".to_string(),
                InstanceConfig {
                    name: "Directory".into(),
                    models_dir: directory_text,
                    ..InstanceConfig::default()
                },
            ),
            (
                "lora".to_string(),
                InstanceConfig {
                    name: "LoRA".into(),
                    lora_path: target_text.clone(),
                    ..InstanceConfig::default()
                },
            ),
            (
                "scaled".to_string(),
                InstanceConfig {
                    name: "Scaled".into(),
                    lora_scaled: format!("\"{target_text}\" 0.5"),
                    ..InstanceConfig::default()
                },
            ),
            (
                "manual".to_string(),
                InstanceConfig {
                    name: "Manual".into(),
                    launch_mode: "manual".into(),
                    manual_command: format!(
                        "llama-server --spec-draft-model=\"{target_text}\" --port 8080"
                    ),
                    ..InstanceConfig::default()
                },
            ),
            (
                "future".to_string(),
                InstanceConfig {
                    name: "FutureArg".into(),
                    custom_args: vec![format!(
                        "--future-model-list=\"{target_text}\":0.5,other.gguf:0.5"
                    )],
                    ..InstanceConfig::default()
                },
            ),
        ]);

        assert_eq!(
            instances_referencing_model(&instances, &HashMap::new(), &target).unwrap(),
            vec![
                "Custom",
                "Directory",
                "FutureArg",
                "LoRA",
                "Manual",
                "Scaled"
            ]
        );
        let _ = std::fs::remove_dir_all(models_dir);
    }

    #[test]
    fn model_deletion_references_include_router_preset_files() {
        let models_dir = temp_test_dir("delete-preset-references");
        let target = models_dir.join("preset target.gguf");
        let nested_preset = models_dir.join("nested.ini");
        let root_preset = models_dir.join("root.ini");
        std::fs::write(&target, b"model").unwrap();
        std::fs::write(
            &nested_preset,
            format!(
                "version = 1\n\n[preset-model]\nmodel = {} ; retained model\n",
                target.display()
            ),
        )
        .unwrap();
        std::fs::write(
            &root_preset,
            format!(
                "[*]\nmodels-preset = {}\nLLAMA_ARG_MODELS_DIR = {}\n",
                nested_preset.display(),
                models_dir.display()
            ),
        )
        .unwrap();
        let preset_text = root_preset.to_string_lossy().to_string();
        let instances = HashMap::from([
            (
                "configured".to_string(),
                InstanceConfig {
                    name: "ConfiguredPreset".into(),
                    models_preset: preset_text.clone(),
                    ..InstanceConfig::default()
                },
            ),
            (
                "custom".to_string(),
                InstanceConfig {
                    name: "CustomPreset".into(),
                    custom_args: vec![format!("--models-preset=\"{preset_text}\"")],
                    ..InstanceConfig::default()
                },
            ),
            (
                "manual".to_string(),
                InstanceConfig {
                    name: "ManualPreset".into(),
                    launch_mode: "manual".into(),
                    manual_command: format!(
                        "llama-server --models-preset \"{preset_text}\" --port 8080"
                    ),
                    ..InstanceConfig::default()
                },
            ),
        ]);

        assert_eq!(
            instances_referencing_model(&instances, &HashMap::new(), &target).unwrap(),
            vec!["ConfiguredPreset", "CustomPreset", "ManualPreset"]
        );
        let _ = std::fs::remove_dir_all(models_dir);
    }

    #[test]
    fn missing_or_malformed_router_presets_fail_model_deletion_closed() {
        let models_dir = temp_test_dir("delete-invalid-preset");
        let missing = models_dir.join("missing.ini");
        let missing_instances = HashMap::from([(
            "missing".to_string(),
            InstanceConfig {
                name: "MissingPreset".into(),
                models_preset: missing.to_string_lossy().to_string(),
                ..InstanceConfig::default()
            },
        )]);
        let missing_error = instances_referencing_model(
            &missing_instances,
            &HashMap::new(),
            &models_dir.join("target.gguf"),
        )
        .unwrap_err();
        assert!(missing_error.contains("拒绝"));
        assert!(missing_error.contains("missing.ini"));

        let malformed = models_dir.join("malformed.ini");
        std::fs::write(&malformed, "[unterminated\nmodel = target.gguf\n").unwrap();
        let malformed_instances = HashMap::from([(
            "malformed".to_string(),
            InstanceConfig {
                name: "MalformedPreset".into(),
                models_preset: malformed.to_string_lossy().to_string(),
                ..InstanceConfig::default()
            },
        )]);
        let malformed_error = instances_referencing_model(
            &malformed_instances,
            &HashMap::new(),
            &models_dir.join("target.gguf"),
        )
        .unwrap_err();
        assert!(malformed_error.contains("未闭合"));
        let _ = std::fs::remove_dir_all(models_dir);
    }

    #[test]
    fn malformed_launch_escape_hatches_fail_model_deletion_closed() {
        let instances = HashMap::from([(
            "broken".to_string(),
            InstanceConfig {
                name: "Broken".into(),
                custom_args: vec!["--model \"unterminated".into()],
                ..InstanceConfig::default()
            },
        )]);

        let error = instances_referencing_model(
            &instances,
            &HashMap::new(),
            Path::new("/models/target.gguf"),
        )
        .unwrap_err();
        assert!(error.contains("拒绝"));
        assert!(error.contains("Broken"));
    }

    #[test]
    fn running_launch_snapshot_blocks_deleting_its_original_model() {
        let saved = HashMap::from([(
            "primary".to_string(),
            InstanceConfig {
                name: "Primary".into(),
                model_path: "/models/reconfigured.gguf".into(),
                ..InstanceConfig::default()
            },
        )]);
        let running = HashMap::from([(
            "primary".to_string(),
            RunningInstance {
                instance_id: "primary".into(),
                pid: 42,
                port: 8080,
                host: "127.0.0.1".into(),
                start_time: 0,
                executable_path: String::new(),
                telemetry_session_id: None,
                workload: String::new(),
                launch_config: Some(InstanceConfig {
                    name: "Primary".into(),
                    model_path: "/models/original.gguf".into(),
                    ..InstanceConfig::default()
                }),
            },
        )]);

        assert_eq!(
            instances_referencing_model(&saved, &running, Path::new("/models/original.gguf"))
                .unwrap(),
            vec!["Primary"]
        );
    }

    #[test]
    fn running_instance_without_launch_snapshot_fails_model_deletion_closed() {
        let running = HashMap::from([(
            "legacy-running".to_string(),
            RunningInstance {
                instance_id: "legacy-running".into(),
                pid: 42,
                port: 8080,
                host: "127.0.0.1".into(),
                start_time: 0,
                executable_path: String::new(),
                telemetry_session_id: None,
                workload: String::new(),
                launch_config: None,
            },
        )]);

        let error = instances_referencing_model(
            &HashMap::new(),
            &running,
            Path::new("/models/target.gguf"),
        )
        .unwrap_err();
        assert!(error.contains("legacy-running"));
        assert!(error.contains("拒绝"));
    }

    #[cfg(windows)]
    #[test]
    fn engine_references_and_names_accept_windows_namespace_aliases() {
        let stored = r"\\?\C:\Engines\llama-server";
        let configured = r"c:\engines\llama-server\";
        let mut instances = HashMap::new();
        instances.insert(
            "primary".into(),
            InstanceConfig {
                name: "Primary".into(),
                engine_id: configured.into(),
                ..InstanceConfig::default()
            },
        );
        assert_eq!(
            instances_referencing_engine(&instances, stored),
            vec!["Primary"]
        );

        let names = HashMap::from([(configured.to_string(), "Custom".to_string())]);
        assert_eq!(
            saved_engine_name(&names, stored).map(String::as_str),
            Some("Custom")
        );
    }
}

pub async fn delete_engine(id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let referenced_by = instances_referencing_engine(&state.instances.lock().unwrap(), &id);
    if !referenced_by.is_empty() {
        return Err(format!(
            "引擎正被实例引用，无法删除: {}",
            referenced_by.join(", ")
        ));
    }
    let mut engines = state.engines.lock().unwrap();
    let stored_id = engines
        .iter()
        .find(|engine| paths_equal(Path::new(&engine.id), Path::new(&id)))
        .map(|engine| engine.id.clone())
        .unwrap_or_else(|| id.clone());
    engines.retain(|engine| !paths_equal(Path::new(&engine.id), Path::new(&id)));
    let _ = model_inventory::delete_engine(&stored_id);
    Ok(())
}

pub async fn rename_engine(
    id: String,
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut engines = state.engines.lock().unwrap();
    let Some(engine_index) = engines
        .iter()
        .position(|engine| paths_equal(Path::new(&engine.id), Path::new(&id)))
    else {
        return Err("未找到要重命名的引擎".to_string());
    };
    let stored_id = engines[engine_index].id.clone();
    let mut next_engine_names = state.engine_names.lock().unwrap().clone();
    next_engine_names
        .retain(|saved_id, _| !paths_equal(Path::new(saved_id), Path::new(&stored_id)));
    next_engine_names.insert(stored_id, name.clone());

    // Do not publish either in-memory name until the atomic config write succeeds.
    crate::commands::config::replace_engine_names_and_persist(&state, next_engine_names)?;
    let engine = engines
        .get_mut(engine_index)
        .ok_or_else(|| "未找到要重命名的引擎".to_string())?;
    engine.custom_name = Some(name.clone());
    engine.name = name;
    Ok(())
}

pub async fn open_engine_folder(dir: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    Ok(())
}

// IPC compatibility boundary: legacy command internals keep their existing error flow,
// while every registered command serializes a stable AppError object.
#[allow(dead_code, unused_imports, unused_mut)] // Tauri references adapters through generated macros.
pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn scan_models(
        paths: Vec<String>,
        state: tauri::State<'_, AppState>,
        _app: tauri::AppHandle,
    ) -> crate::error::AppResult<Vec<ModelInfo>> {
        super::scan_models(paths, state, _app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn load_app_data(
        paths: Vec<String>,
        engine_paths: Vec<String>,
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<AppDataSnapshot> {
        super::load_app_data(paths, engine_paths, state, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_cached_scan(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Option<CachedScan>> {
        super::get_cached_scan(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_models(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<ModelInfo>> {
        super::get_models(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn preview_model_deletion(
        path: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<ModelDeletionPreview> {
        super::preview_model_deletion(path, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn delete_model_file(
        path: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<ModelDeletionResult> {
        super::delete_model_file(path, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn open_model_folder(path: String) -> crate::error::AppResult<()> {
        super::open_model_folder(path)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn read_gguf_metadata(
        path: String,
    ) -> crate::error::AppResult<crate::models::GgufMetadataSummary> {
        super::read_gguf_metadata(path)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn scan_engines(
        paths: Vec<String>,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<EngineInfo>> {
        super::scan_engines(paths, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_engines(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<EngineInfo>> {
        super::get_engines(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn delete_engine(
        id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::delete_engine(id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn rename_engine(
        id: String,
        name: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::rename_engine(id, name, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn open_engine_folder(dir: String) -> crate::error::AppResult<()> {
        super::open_engine_folder(dir)
            .await
            .map_err(crate::error::AppError::from)
    }
}
