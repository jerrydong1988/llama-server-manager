use crate::models::{AppState, InstanceConfig};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalArtifactReference {
    pub instance_id: String,
    pub instance_name: String,
    pub source: String,
    pub flag: String,
    pub artifact_kind: String,
    pub ownership: String,
    pub value: String,
    pub location_kind: String,
    pub exists: Option<bool>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalArtifactInventory {
    pub references: Vec<ExternalArtifactReference>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy)]
struct TrackedFlag {
    names: &'static [&'static str],
    canonical: &'static str,
    kind: &'static str,
    remote: bool,
}

#[derive(Clone, Copy)]
struct ReferenceContext<'a> {
    instance_id: &'a str,
    instance_name: &'a str,
    source: &'a str,
}

const TRACKED_CUSTOM_FLAGS: &[TrackedFlag] = &[
    TrackedFlag {
        names: &["--lookup-cache-static", "-lcs"],
        canonical: "--lookup-cache-static",
        kind: "lookup-cache-static",
        remote: false,
    },
    TrackedFlag {
        names: &["--lookup-cache-dynamic", "-lcd"],
        canonical: "--lookup-cache-dynamic",
        kind: "lookup-cache-dynamic",
        remote: false,
    },
    TrackedFlag {
        names: &["--slot-save-path"],
        canonical: "--slot-save-path",
        kind: "slot-state",
        remote: false,
    },
    TrackedFlag {
        names: &["--log-prompts-dir"],
        canonical: "--log-prompts-dir",
        kind: "prompt-log-directory",
        remote: false,
    },
    TrackedFlag {
        names: &["--log-file"],
        canonical: "--log-file",
        kind: "engine-log-file",
        remote: false,
    },
    TrackedFlag {
        names: &["--mmproj-url"],
        canonical: "--mmproj-url",
        kind: "projector-url",
        remote: true,
    },
];

fn tracked_flag(name: &str) -> Option<TrackedFlag> {
    TRACKED_CUSTOM_FLAGS
        .iter()
        .copied()
        .find(|tracked| tracked.names.contains(&name))
}

fn describe_location(value: &str, remote: bool) -> (String, Option<bool>, Option<u64>) {
    if remote || value.starts_with("http://") || value.starts_with("https://") {
        return ("remote".into(), None, None);
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        return ("relative".into(), None, None);
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ("absolute-missing".into(), Some(false), None)
        }
        Err(_) => return ("absolute-inaccessible".into(), None, None),
    };
    if crate::artifact_maintenance::metadata_is_link_like(&metadata) {
        return ("absolute-link".into(), Some(true), None);
    }
    if metadata.is_dir() {
        ("absolute-directory".into(), Some(true), None)
    } else if metadata.is_file() {
        ("absolute-file".into(), Some(true), Some(metadata.len()))
    } else {
        ("absolute-other".into(), Some(true), None)
    }
}

fn push_reference(
    inventory: &mut ExternalArtifactInventory,
    context: ReferenceContext<'_>,
    flag: &str,
    kind: &str,
    value: &str,
    remote: bool,
) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    let (location_kind, exists, size_bytes) = describe_location(value, remote);
    inventory.references.push(ExternalArtifactReference {
        instance_id: context.instance_id.to_string(),
        instance_name: context.instance_name.to_string(),
        source: context.source.to_string(),
        flag: flag.to_string(),
        artifact_kind: kind.to_string(),
        ownership: "operator".to_string(),
        value: value.to_string(),
        location_kind,
        exists,
        size_bytes,
    });
}

fn inventory_configured_fields(
    inventory: &mut ExternalArtifactInventory,
    instance_id: &str,
    config: &InstanceConfig,
) {
    for (flag, kind, value, remote) in [
        (
            "--lookup-cache-static",
            "lookup-cache-static",
            config.lookup_cache_static.as_str(),
            false,
        ),
        (
            "--lookup-cache-dynamic",
            "lookup-cache-dynamic",
            config.lookup_cache_dynamic.as_str(),
            false,
        ),
        (
            "--slot-save-path",
            "slot-state",
            config.slot_save_path.as_str(),
            false,
        ),
        (
            "--log-prompts-dir",
            "prompt-log-directory",
            config.log_prompts_dir.as_str(),
            false,
        ),
        (
            "--mmproj-url",
            "projector-url",
            config.mmproj_url.as_str(),
            true,
        ),
    ] {
        push_reference(
            inventory,
            ReferenceContext {
                instance_id,
                instance_name: &config.name,
                source: "configured-field",
            },
            flag,
            kind,
            value,
            remote,
        );
    }
}

fn inventory_argument_tokens(
    inventory: &mut ExternalArtifactInventory,
    instance_id: &str,
    config: &InstanceConfig,
    source: &str,
    tokens: &[String],
) {
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        let (name, inline_value) = token
            .split_once('=')
            .map_or((token.as_str(), None), |(name, value)| (name, Some(value)));
        let Some(tracked) = tracked_flag(name) else {
            index += 1;
            continue;
        };
        let value = if let Some(value) = inline_value {
            value
        } else if let Some(value) = tokens.get(index + 1) {
            if tracked_flag(value).is_some() {
                inventory.warnings.push(format!(
                    "实例 {} 的 {source} 参数 {name} 缺少路径值",
                    config.name,
                ));
                index += 1;
                continue;
            }
            index += 1;
            value
        } else {
            inventory.warnings.push(format!(
                "实例 {} 的 {source} 参数 {name} 缺少路径值",
                config.name,
            ));
            index += 1;
            continue;
        };
        push_reference(
            inventory,
            ReferenceContext {
                instance_id,
                instance_name: &config.name,
                source,
            },
            tracked.canonical,
            tracked.kind,
            value,
            tracked.remote,
        );
        index += 1;
    }
}

fn inventory_custom_arguments(
    inventory: &mut ExternalArtifactInventory,
    instance_id: &str,
    config: &InstanceConfig,
) {
    let mut tokens = Vec::new();
    for row in &config.custom_args {
        match crate::commands::server::split_args_checked(row.trim()) {
            Ok(parsed) => tokens.extend(parsed),
            Err(error) => inventory.warnings.push(format!(
                "实例 {} 的自定义参数无法解析，外部产物盘点可能不完整: {error}",
                config.name
            )),
        }
    }
    inventory_argument_tokens(inventory, instance_id, config, "custom-argument", &tokens);
}

fn inventory_manual_command(
    inventory: &mut ExternalArtifactInventory,
    instance_id: &str,
    config: &InstanceConfig,
) {
    if config.manual_command.trim().is_empty() {
        return;
    }
    match crate::commands::server::split_args_checked(config.manual_command.trim()) {
        Ok(tokens) => {
            inventory_argument_tokens(inventory, instance_id, config, "manual-command", &tokens)
        }
        Err(error) => inventory.warnings.push(format!(
            "实例 {} 的手动命令无法解析，外部产物盘点可能不完整: {error}",
            config.name
        )),
    }
}

pub(crate) fn inventory_external_artifacts(
    instances: &HashMap<String, InstanceConfig>,
) -> ExternalArtifactInventory {
    let mut inventory = ExternalArtifactInventory::default();
    let mut instance_ids = instances.keys().collect::<Vec<_>>();
    instance_ids.sort();
    for instance_id in instance_ids {
        let config = &instances[instance_id];
        inventory_configured_fields(&mut inventory, instance_id, config);
        inventory_custom_arguments(&mut inventory, instance_id, config);
        inventory_manual_command(&mut inventory, instance_id, config);
    }
    inventory.references.sort_by(|left, right| {
        left.instance_id
            .cmp(&right.instance_id)
            .then_with(|| left.flag.cmp(&right.flag))
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.value.cmp(&right.value))
    });
    inventory
}

#[tauri::command]
pub async fn get_external_artifact_inventory(
    state: tauri::State<'_, AppState>,
) -> crate::error::AppResult<ExternalArtifactInventory> {
    Ok(inventory_external_artifacts(
        &state.instances.lock().unwrap(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventories_configured_and_custom_external_paths_without_claiming_ownership() {
        let config = InstanceConfig {
            name: "Primary".into(),
            lookup_cache_static: "cache/static.bin".into(),
            log_prompts_dir: "prompt-logs".into(),
            mmproj_url: "https://example.invalid/mmproj.gguf".into(),
            custom_args: vec![
                "--slot-save-path".into(),
                "custom-slots".into(),
                "--lookup-cache-dynamic=cache/dynamic.bin".into(),
                "--log-file \"engine output.log\"".into(),
            ],
            manual_command:
                "llama-server --log-prompts-dir \"manual prompt logs\" --model model.gguf".into(),
            ..InstanceConfig::default()
        };
        let inventory = inventory_external_artifacts(&HashMap::from([("primary".into(), config)]));

        assert!(inventory.warnings.is_empty());
        assert_eq!(inventory.references.len(), 7);
        assert!(inventory
            .references
            .iter()
            .all(|reference| reference.ownership == "operator"));
        assert!(inventory.references.iter().any(|reference| {
            reference.flag == "--slot-save-path"
                && reference.value == "custom-slots"
                && reference.source == "custom-argument"
        }));
        assert!(inventory.references.iter().any(|reference| {
            reference.flag == "--mmproj-url" && reference.location_kind == "remote"
        }));
        assert!(inventory.references.iter().any(|reference| {
            reference.flag == "--log-prompts-dir"
                && reference.value == "manual prompt logs"
                && reference.source == "manual-command"
        }));
    }

    #[test]
    fn malformed_or_missing_custom_values_are_reported_without_guessing() {
        let config = InstanceConfig {
            name: "Broken".into(),
            custom_args: vec![
                "--slot-save-path".into(),
                "--log-file".into(),
                "\"unterminated".into(),
            ],
            ..InstanceConfig::default()
        };
        let inventory = inventory_external_artifacts(&HashMap::from([("broken".into(), config)]));

        assert!(inventory.references.is_empty());
        assert!(inventory.warnings.len() >= 2);
    }
}
