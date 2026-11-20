use crate::commands::config::read_config_from_disk;
use crate::error::AppError;
use crate::gufo::EngineKind;
use crate::models::{AppState, EngineInfo};
use crate::path_utils::paths_equal;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SelectionError {
    ConfiguredMissing,
    KindMismatch,
    NoneAvailable,
}

impl From<SelectionError> for AppError {
    fn from(error: SelectionError) -> Self {
        match error {
            SelectionError::ConfiguredMissing => AppError::new(
                "CONFIGURED_ENGINE_NOT_FOUND",
                "实例配置引用的 llama-server 引擎已不存在，请重新选择引擎。",
                false,
            ),
            SelectionError::KindMismatch => AppError::new(
                "ENGINE_KIND_MISMATCH",
                "Instance and engine types differ",
                false,
            ),
            SelectionError::NoneAvailable => AppError::new(
                "CONFIGURED_ENGINE_REQUIRED",
                "实例必须引用已扫描并确认的 llama-server 引擎。",
                false,
            ),
        }
    }
}

fn select<'a>(
    engines: &'a [EngineInfo],
    kind: EngineKind,
    configured: &str,
    default_id: &str,
) -> Result<&'a EngineInfo, SelectionError> {
    let by_id = |engine: &EngineInfo, id: &str| paths_equal(Path::new(&engine.id), Path::new(id));
    if !configured.is_empty() {
        let engine = engines
            .iter()
            .find(|e| by_id(e, configured))
            .ok_or(SelectionError::ConfiguredMissing)?;
        return if engine.kind == kind {
            Ok(engine)
        } else {
            Err(SelectionError::KindMismatch)
        };
    }
    let mut candidates = engines.iter().filter(|e| e.kind == kind);
    candidates
        .clone()
        .find(|e| by_id(e, default_id))
        .or_else(|| candidates.next())
        .ok_or(SelectionError::NoneAvailable)
}

/// Read the persisted default only for an unbound instance. Callers copy just the
/// inventory fields they need before releasing the lock (no file I/O inside it).
pub(crate) fn with_instance_engine<T>(
    state: &AppState,
    kind: EngineKind,
    configured: &str,
    read: impl FnOnce(&EngineInfo) -> T,
) -> Result<T, SelectionError> {
    let default_id = if configured.is_empty() {
        let dir = state.config_dir.lock().unwrap().clone();
        read_config_from_disk(&dir).default_engine_id
    } else {
        String::new()
    };
    select(
        &state.engines.lock().unwrap(),
        kind,
        configured,
        &default_id,
    )
    .map(read)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(id: &str, kind: EngineKind) -> EngineInfo {
        serde_json::from_value(serde_json::json!({
            "id": id, "kind": kind, "name": id, "exe": id, "dir": ".", "version":"", "backend":""
        }))
        .unwrap()
    }

    #[test]
    fn explicit_engines_never_fall_back_and_defaults_are_scoped_to_kind() {
        use EngineKind::{Gufo, LlamaCpp};
        let engines = [
            engine("llama-A", LlamaCpp),
            engine("gufo", Gufo),
            engine("llama-B", LlamaCpp),
        ];
        assert_eq!(
            select(&engines, LlamaCpp, "", "llama-B").unwrap().id,
            "llama-B"
        );
        assert_eq!(
            select(&engines, LlamaCpp, "", "gufo").unwrap().id,
            "llama-A"
        );
        assert_eq!(select(&engines, Gufo, "", "missing").unwrap().id, "gufo");
        assert_eq!(
            select(&engines, LlamaCpp, "missing", "llama-B").unwrap_err(),
            SelectionError::ConfiguredMissing
        );
        assert_eq!(
            select(&engines, Gufo, "llama-A", "gufo").unwrap_err(),
            SelectionError::KindMismatch
        );
        assert_eq!(
            select(&[], LlamaCpp, "", "").unwrap_err(),
            SelectionError::NoneAvailable
        );
    }
}
