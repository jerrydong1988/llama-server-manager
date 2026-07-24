use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use tauri_plugin_dialog::DialogExt;

const PATH_AUTHORITY_FILE: &str = "authorized-paths.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct PathAuthority {
    #[serde(default)]
    engine_roots: BTreeSet<String>,
    #[serde(default)]
    model_roots: BTreeSet<String>,
    #[serde(default)]
    download_roots: BTreeSet<String>,
}

static PATH_AUTHORITY: LazyLock<Mutex<PathAuthority>> = LazyLock::new(|| {
    let authority = std::fs::read(path_authority_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    Mutex::new(authority)
});

fn path_authority_path() -> PathBuf {
    crate::utils::get_data_dir()
        .join("configs")
        .join(PATH_AUTHORITY_FILE)
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("无法访问所选目录 {}: {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!("所选路径不是目录: {}", path.display()));
    }
    std::fs::canonicalize(path)
        .map_err(|error| format!("无法解析所选目录 {}: {error}", path.display()))
}

fn normalized_authority_key(path: &Path) -> String {
    let value = path.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        value.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        value
    }
}

fn persist_authority(authority: &PathAuthority) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(authority)
        .map_err(|error| format!("无法序列化目录授权: {error}"))?;
    crate::persistence::atomic_write(&path_authority_path(), &bytes, None)
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn path_is_within(candidate: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let candidate = normalized_authority_key(candidate);
        let mut root = normalized_authority_key(root);
        while root.ends_with(['\\', '/']) {
            root.pop();
        }
        candidate == root
            || candidate
                .strip_prefix(&root)
                .is_some_and(|rest| rest.starts_with(['\\', '/']))
    }
    #[cfg(not(windows))]
    {
        candidate.starts_with(root)
    }
}

fn is_authorized_by_roots(candidate: &Path, roots: &BTreeSet<String>) -> bool {
    roots
        .iter()
        .map(PathBuf::from)
        .any(|root| path_is_within(candidate, &root))
}

fn validate_download_root_boundary(root: &Path, app_data_root: &Path) -> Result<(), String> {
    let managed_models = app_data_root.join("models");
    if path_is_within(app_data_root, root)
        || (path_is_within(root, app_data_root) && !path_is_within(root, &managed_models))
    {
        return Err("下载目录不能包含应用数据根，也不能位于配置、运行时或引擎目录中".to_string());
    }
    Ok(())
}

fn record_authorized_root(purpose: &str, root: &Path) -> Result<(), String> {
    let root = canonical_directory(root)?;
    let key = normalized_authority_key(&root);
    let mut authority = PATH_AUTHORITY.lock().unwrap();
    match purpose {
        "engine" => {
            authority.engine_roots.insert(key);
        }
        "model" => {
            authority.model_roots.insert(key);
        }
        "download" => {
            let app_data_root = canonical_directory(&crate::utils::get_data_dir())
                .unwrap_or_else(|_| crate::utils::get_data_dir());
            validate_download_root_boundary(&root, &app_data_root)?;
            authority.download_roots.insert(key);
        }
        _ => return Err("未知的目录授权用途".to_string()),
    }
    persist_authority(&authority)
}

pub fn initialize_path_authority(
    legacy_engine_roots: &[String],
    legacy_model_roots: &[String],
) -> Result<(), String> {
    let app_data_root = crate::utils::get_data_dir();
    let mut authority = PATH_AUTHORITY.lock().unwrap();
    for root in legacy_engine_roots {
        let root = PathBuf::from(root);
        if let Ok(canonical) = canonical_directory(&root) {
            authority
                .engine_roots
                .insert(normalized_authority_key(&canonical));
        }
    }
    for root in legacy_model_roots {
        let root = PathBuf::from(root);
        let root = if root.is_relative() {
            app_data_root.join(root)
        } else {
            root
        };
        if let Ok(canonical) = canonical_directory(&root) {
            authority
                .model_roots
                .insert(normalized_authority_key(&canonical));
        }
    }
    persist_authority(&authority)
}

pub fn require_authorized_engine_root(path: &Path) -> Result<PathBuf, String> {
    let canonical = canonical_directory(path)?;
    let managed_root = crate::utils::get_data_dir().join("engines");
    if path_is_within(&canonical, &managed_root) {
        return Ok(canonical);
    }

    let authority = PATH_AUTHORITY.lock().unwrap();
    if is_authorized_by_roots(&canonical, &authority.engine_roots) {
        Ok(canonical)
    } else {
        Err("引擎目录未获授权；请使用应用内的目录选择按钮重新选择。".to_string())
    }
}

pub fn require_authorized_model_root(path: &Path) -> Result<PathBuf, String> {
    let canonical = canonical_directory(path)?;
    let managed_root = crate::utils::get_default_models_dir();
    let managed_root =
        canonical_directory(&managed_root).unwrap_or_else(|_| managed_root.to_path_buf());
    if path_is_within(&canonical, &managed_root) {
        return Ok(canonical);
    }

    let authority = PATH_AUTHORITY.lock().unwrap();
    if is_authorized_by_roots(&canonical, &authority.model_roots) {
        Ok(canonical)
    } else {
        Err("模型目录未获授权；请使用应用内的目录选择按钮重新选择。".to_string())
    }
}

pub fn require_authorized_model_path(path: &Path) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("无法解析模型路径 {}: {error}", path.display()))?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|error| format!("无法访问模型路径 {}: {error}", canonical.display()))?;
    if !metadata.is_file() {
        return Err(format!("模型路径不是文件: {}", canonical.display()));
    }

    let managed_root = crate::utils::get_default_models_dir();
    let managed_root =
        canonical_directory(&managed_root).unwrap_or_else(|_| managed_root.to_path_buf());
    if path_is_within(&canonical, &managed_root) {
        return Ok(canonical);
    }

    let authority = PATH_AUTHORITY.lock().unwrap();
    if is_authorized_by_roots(&canonical, &authority.model_roots) {
        Ok(canonical)
    } else {
        Err("模型文件不在已授权目录中；请使用应用内的目录选择按钮重新选择。".to_string())
    }
}

pub fn require_path_within_root(path: &Path, root: &Path) -> Result<PathBuf, String> {
    let canonical_path = std::fs::canonicalize(path)
        .map_err(|error| format!("无法解析受控路径 {}: {error}", path.display()))?;
    let canonical_root = canonical_directory(root)?;
    if path_is_within(&canonical_path, &canonical_root) {
        Ok(canonical_path)
    } else {
        Err(format!(
            "路径 {} 越过了已授权目录 {}",
            canonical_path.display(),
            canonical_root.display()
        ))
    }
}

pub fn resolve_authorized_download_root(
    app_data_root: &Path,
    save_dir: &str,
) -> Result<PathBuf, String> {
    let requested = PathBuf::from(save_dir.trim());
    if requested.is_relative() {
        if !is_safe_relative_path(&requested) {
            return Err("下载目录不能包含父目录跳转或平台路径前缀".to_string());
        }
        let resolved = app_data_root.join(requested);
        let managed_models = app_data_root.join("models");
        if !path_is_within(&resolved, &managed_models) {
            return Err("相对下载目录必须位于应用管理的 models 目录中".to_string());
        }
        return Ok(resolved);
    }
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("下载目录不能包含父目录跳转".to_string());
    }
    let requested = canonical_directory(&requested)?;
    let canonical_app_data_root =
        canonical_directory(app_data_root).unwrap_or_else(|_| app_data_root.to_path_buf());
    validate_download_root_boundary(&requested, &canonical_app_data_root)?;

    let authority = PATH_AUTHORITY.lock().unwrap();
    if authority
        .download_roots
        .iter()
        .map(PathBuf::from)
        .any(|root| path_is_within(&requested, &root))
    {
        Ok(requested)
    } else {
        Err("绝对下载目录未获授权；请使用应用内的目录选择按钮重新选择。".to_string())
    }
}

pub fn ensure_download_path_within_root(path: &Path, root: &Path) -> Result<(), String> {
    if path_is_within(path, root) {
        Ok(())
    } else {
        Err("下载目标越过了已授权目录边界".to_string())
    }
}

#[tauri::command]
pub async fn pick_authorized_directory(
    purpose: String,
    app: tauri::AppHandle,
) -> Result<Option<String>, String> {
    if purpose != "engine" && purpose != "model" && purpose != "download" {
        return Err("未知的目录授权用途".to_string());
    }
    let title = match purpose.as_str() {
        "engine" => "选择 llama-server 引擎目录",
        "model" => "选择模型扫描目录",
        _ => "选择下载保存目录",
    };
    let Some(selection) = app.dialog().file().set_title(title).blocking_pick_folder() else {
        return Ok(None);
    };
    let path = selection
        .into_path()
        .map_err(|error| format!("无法解析所选目录: {error}"))?;
    record_authorized_root(&purpose, &path)?;
    Ok(Some(
        canonical_directory(&path)?.to_string_lossy().to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_download_roots_reject_parent_traversal() {
        assert!(is_safe_relative_path(Path::new("models/custom")));
        assert!(!is_safe_relative_path(Path::new("../outside")));
        assert!(!is_safe_relative_path(Path::new("models/../../outside")));
        let app_data = Path::new("/app-data");
        assert!(resolve_authorized_download_root(app_data, "models/custom")
            .unwrap()
            .starts_with(app_data.join("models")));
        assert!(resolve_authorized_download_root(app_data, "engines").is_err());
        assert!(resolve_authorized_download_root(app_data, "configs").is_err());
    }

    #[test]
    fn path_containment_uses_component_boundaries() {
        let root = Path::new("downloads").join("models");
        assert!(path_is_within(&root.join("repo").join("model.gguf"), &root));
        assert!(!path_is_within(
            &Path::new("downloads").join("models-elsewhere"),
            &root
        ));
    }

    #[test]
    fn canonical_containment_rejects_a_sibling_file() {
        let base = std::env::temp_dir().join(format!(
            "llama-server-manager-path-containment-{}",
            uuid::Uuid::new_v4()
        ));
        let root = base.join("authorized");
        let sibling = base.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let inside_file = root.join("llama-server");
        let outside_file = sibling.join("llama-server");
        std::fs::write(&inside_file, b"inside").unwrap();
        std::fs::write(&outside_file, b"outside").unwrap();

        assert!(require_path_within_root(&inside_file, &root).is_ok());
        assert!(require_path_within_root(&outside_file, &root).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn model_authority_does_not_cover_a_sibling_directory() {
        let root = Path::new("authorized-models");
        let roots = BTreeSet::from([normalized_authority_key(root)]);
        assert!(is_authorized_by_roots(
            &root.join("repo").join("model.gguf"),
            &roots
        ));
        assert!(!is_authorized_by_roots(
            &Path::new("authorized-models-sibling").join("model.gguf"),
            &roots
        ));
    }
}
