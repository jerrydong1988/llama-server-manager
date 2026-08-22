use crate::commands::config::{
    load_global_config_for_update_unlocked, lock_global_config_for_update,
    persist_global_config_unlocked,
};
use crate::error::{AppError, AppResult};
use crate::models::{AppState, GlobalConfig, InstanceConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const CONFIG_REVISION_SCHEMA_VERSION: u32 = 2;
pub const CONFIG_REVISION_HISTORY_LIMIT: usize = 50;
pub const CONFIG_REVISION_AUDIT_LIMIT: usize = 200;
const CONFIG_REVISION_DIFF_LIMIT: usize = 128;
const CONFIG_REVISION_VALUE_LIMIT: usize = 192;

pub const fn default_config_revision_schema_version() -> u32 {
    CONFIG_REVISION_SCHEMA_VERSION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigRevisionReason {
    Migration,
    Created,
    Save,
    System,
    Rollback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigRevisionRecord {
    pub id: String,
    pub fingerprint: String,
    #[serde(default = "default_configuration_identity_schema_version")]
    pub identity_schema_version: u8,
    #[serde(default)]
    pub configuration_id: String,
    pub parent_revision_id: Option<String>,
    pub created_at: u64,
    pub reason: ConfigRevisionReason,
    pub rollback_of: Option<String>,
    #[serde(default)]
    event_integrity: String,
    snapshot: InstanceConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigRevisionAuditAction {
    KnownGoodSet,
    KnownGoodInvalidated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigRevisionAuditEvent {
    pub id: String,
    pub instance_id: String,
    pub created_at: u64,
    pub action: ConfigRevisionAuditAction,
    pub revision_id: Option<String>,
    pub previous_revision_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigValueSummaryState {
    Empty,
    Value,
    Set,
    ItemCount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigValueSummary {
    pub state: ConfigValueSummaryState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFieldChangeSummary {
    pub field: String,
    pub before: ConfigValueSummary,
    pub after: ConfigValueSummary,
    pub redacted: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRevisionSummary {
    pub id: String,
    pub fingerprint: String,
    pub identity_schema_version: u8,
    pub configuration_id: String,
    pub parent_revision_id: Option<String>,
    pub created_at: u64,
    pub reason: ConfigRevisionReason,
    pub rollback_of: Option<String>,
    pub current: bool,
    pub known_good: bool,
    pub integrity_valid: bool,
    pub diff_truncated: bool,
    pub changes: Vec<ConfigFieldChangeSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRevisionAuditSummary {
    pub id: String,
    pub created_at: u64,
    pub action: ConfigRevisionAuditAction,
    pub revision_id: Option<String>,
    pub previous_revision_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRevisionHistoryResponse {
    pub instance_id: String,
    pub current_fingerprint: String,
    pub current_revision_id: String,
    pub current_configuration_id: String,
    pub known_good_revision_id: Option<String>,
    pub revisions: Vec<ConfigRevisionSummary>,
    pub audit: Vec<ConfigRevisionAuditSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRevisionRollbackResponse {
    pub config: InstanceConfig,
    pub history: ConfigRevisionHistoryResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRevisionIdentity {
    pub revision_id: String,
    pub configuration_id: String,
    pub fingerprint: String,
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn canonical_deployment_config(config: &InstanceConfig) -> InstanceConfig {
    let mut canonical = config.clone();
    canonical.id.clear();
    canonical.name.clear();
    if let Some(overrides) = canonical.explicit_overrides.as_mut() {
        overrides
            .iter_mut()
            .for_each(|value| *value = value.trim().to_string());
        overrides.retain(|value| !value.is_empty());
        overrides.sort();
        overrides.dedup();
    }
    canonical
}

pub fn deployment_config_fingerprint(config: &InstanceConfig) -> Result<String, String> {
    let canonical = canonical_deployment_config(config);
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| format!("failed to serialize deployment configuration: {error}"))?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

const fn default_configuration_identity_schema_version() -> u8 {
    1
}

pub fn configuration_id_from_fingerprint(fingerprint: &str) -> Result<String, String> {
    let digest = fingerprint
        .strip_prefix("sha256:")
        .filter(|digest| {
            digest.len() == 64 && digest.chars().all(|value| value.is_ascii_hexdigit())
        })
        .ok_or_else(|| "configuration fingerprint is not a sha256 digest".to_string())?;
    Ok(format!(
        "urn:lsm:configuration:v1:sha256:{}",
        digest.to_ascii_lowercase()
    ))
}

#[derive(Serialize)]
struct ConfigRevisionIntegrityMaterial<'a> {
    id: &'a str,
    fingerprint: &'a str,
    identity_schema_version: u8,
    configuration_id: &'a str,
    parent_revision_id: &'a Option<String>,
    created_at: u64,
    reason: ConfigRevisionReason,
    rollback_of: &'a Option<String>,
}

fn event_integrity_fingerprint(
    material: &ConfigRevisionIntegrityMaterial<'_>,
) -> Result<String, String> {
    let bytes = serde_json::to_vec(material)
        .map_err(|error| format!("failed to serialize revision event identity: {error}"))?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

#[derive(Serialize)]
struct LegacyConfigRevisionIntegrityMaterial<'a> {
    id: &'a str,
    fingerprint: &'a str,
    parent_revision_id: &'a Option<String>,
    created_at: u64,
    reason: ConfigRevisionReason,
    rollback_of: &'a Option<String>,
}

fn legacy_event_integrity_fingerprint(record: &ConfigRevisionRecord) -> Result<String, String> {
    let bytes = serde_json::to_vec(&LegacyConfigRevisionIntegrityMaterial {
        id: &record.id,
        fingerprint: &record.fingerprint,
        parent_revision_id: &record.parent_revision_id,
        created_at: record.created_at,
        reason: record.reason,
        rollback_of: &record.rollback_of,
    })
    .map_err(|error| format!("failed to serialize legacy revision event identity: {error}"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn record_integrity_valid(record: &ConfigRevisionRecord) -> bool {
    let snapshot_valid = deployment_config_fingerprint(&record.snapshot)
        .is_ok_and(|fingerprint| fingerprint == record.fingerprint);
    let event_valid = if record.configuration_id.is_empty() {
        legacy_event_integrity_fingerprint(record)
            .is_ok_and(|integrity| integrity == record.event_integrity)
    } else {
        configuration_id_from_fingerprint(&record.fingerprint)
            .is_ok_and(|identity| identity == record.configuration_id)
            && event_integrity_fingerprint(&ConfigRevisionIntegrityMaterial {
                id: &record.id,
                fingerprint: &record.fingerprint,
                identity_schema_version: record.identity_schema_version,
                configuration_id: &record.configuration_id,
                parent_revision_id: &record.parent_revision_id,
                created_at: record.created_at,
                reason: record.reason,
                rollback_of: &record.rollback_of,
            })
            .is_ok_and(|integrity| integrity == record.event_integrity)
    };
    snapshot_valid && event_valid
}

fn append_audit_event(
    global: &mut GlobalConfig,
    instance_id: &str,
    action: ConfigRevisionAuditAction,
    revision_id: Option<String>,
    previous_revision_id: Option<String>,
    created_at: u64,
) {
    global.config_revision_audit.push(ConfigRevisionAuditEvent {
        id: Uuid::new_v4().to_string(),
        instance_id: instance_id.to_string(),
        created_at,
        action,
        revision_id,
        previous_revision_id,
    });
    if global.config_revision_audit.len() > CONFIG_REVISION_AUDIT_LIMIT {
        let remove = global.config_revision_audit.len() - CONFIG_REVISION_AUDIT_LIMIT;
        global.config_revision_audit.drain(0..remove);
    }
}

fn prune_history(global: &mut GlobalConfig, instance_id: &str) {
    let known_good = global.known_good_config_revisions.get(instance_id).cloned();
    let Some(history) = global.config_revisions.get_mut(instance_id) else {
        return;
    };
    while history.len() > CONFIG_REVISION_HISTORY_LIMIT {
        let latest_id = history.last().map(|revision| revision.id.as_str());
        let removable = history.iter().position(|revision| {
            Some(revision.id.as_str()) != latest_id
                && known_good.as_deref() != Some(revision.id.as_str())
        });
        match removable {
            Some(index) => {
                history.remove(index);
            }
            None => break,
        }
    }
}

fn append_revision_at(
    global: &mut GlobalConfig,
    instance_id: &str,
    config: &InstanceConfig,
    reason: ConfigRevisionReason,
    rollback_of: Option<String>,
    created_at: u64,
) -> Result<String, String> {
    let fingerprint = deployment_config_fingerprint(config)?;
    let history = global
        .config_revisions
        .entry(instance_id.to_string())
        .or_default();
    let parent_revision_id = history.last().map(|revision| revision.id.clone());
    let id = Uuid::new_v4().to_string();
    let identity_schema_version = default_configuration_identity_schema_version();
    let configuration_id = configuration_id_from_fingerprint(&fingerprint)?;
    let event_integrity = event_integrity_fingerprint(&ConfigRevisionIntegrityMaterial {
        id: &id,
        fingerprint: &fingerprint,
        identity_schema_version,
        configuration_id: &configuration_id,
        parent_revision_id: &parent_revision_id,
        created_at,
        reason,
        rollback_of: &rollback_of,
    })?;
    history.push(ConfigRevisionRecord {
        id: id.clone(),
        fingerprint,
        identity_schema_version,
        configuration_id,
        parent_revision_id,
        created_at,
        reason,
        rollback_of,
        event_integrity,
        snapshot: config.clone(),
    });
    prune_history(global, instance_id);
    Ok(id)
}

fn active_instance_ids(global: &GlobalConfig) -> HashSet<String> {
    global.instances.keys().cloned().collect()
}

pub fn ensure_current_config_revisions(global: &mut GlobalConfig) -> Result<bool, String> {
    ensure_current_config_revisions_at(global, now_epoch_seconds())
}

fn ensure_current_config_revisions_at(
    global: &mut GlobalConfig,
    created_at: u64,
) -> Result<bool, String> {
    if global.config_revision_schema_version > CONFIG_REVISION_SCHEMA_VERSION {
        return Err(format!(
            "configuration revision schema {} is newer than supported schema {}",
            global.config_revision_schema_version, CONFIG_REVISION_SCHEMA_VERSION
        ));
    }
    let mut changed = global.config_revision_schema_version < CONFIG_REVISION_SCHEMA_VERSION;
    for history in global.config_revisions.values_mut() {
        for record in history {
            if !record.configuration_id.is_empty() || !record_integrity_valid(record) {
                continue;
            }
            record.configuration_id = configuration_id_from_fingerprint(&record.fingerprint)?;
            record.identity_schema_version = default_configuration_identity_schema_version();
            record.event_integrity =
                event_integrity_fingerprint(&ConfigRevisionIntegrityMaterial {
                    id: &record.id,
                    fingerprint: &record.fingerprint,
                    identity_schema_version: record.identity_schema_version,
                    configuration_id: &record.configuration_id,
                    parent_revision_id: &record.parent_revision_id,
                    created_at: record.created_at,
                    reason: record.reason,
                    rollback_of: &record.rollback_of,
                })?;
            changed = true;
        }
    }
    global.config_revision_schema_version = CONFIG_REVISION_SCHEMA_VERSION;

    let active_ids = active_instance_ids(global);
    let stale_history_ids: Vec<String> = global
        .config_revisions
        .keys()
        .filter(|instance_id| !active_ids.contains(*instance_id))
        .cloned()
        .collect();
    for instance_id in stale_history_ids {
        global.config_revisions.remove(&instance_id);
        global.known_good_config_revisions.remove(&instance_id);
        changed = true;
    }

    let instances: Vec<(String, InstanceConfig)> = global
        .instances
        .iter()
        .map(|(instance_id, config)| (instance_id.clone(), config.clone()))
        .collect();
    for (instance_id, config) in instances {
        let fingerprint = deployment_config_fingerprint(&config)?;
        let current_is_recorded = global
            .config_revisions
            .get(&instance_id)
            .and_then(|history| history.last())
            .is_some_and(|record| {
                record.fingerprint == fingerprint && record_integrity_valid(record)
            });
        if !current_is_recorded {
            append_revision_at(
                global,
                &instance_id,
                &config,
                ConfigRevisionReason::Migration,
                None,
                created_at,
            )?;
            changed = true;
        }
    }

    let known_good_entries: Vec<(String, String)> = global
        .known_good_config_revisions
        .iter()
        .map(|(instance_id, revision_id)| (instance_id.clone(), revision_id.clone()))
        .collect();
    for (instance_id, revision_id) in known_good_entries {
        let valid = global
            .config_revisions
            .get(&instance_id)
            .is_some_and(|history| {
                history
                    .iter()
                    .any(|record| record.id == revision_id && record_integrity_valid(record))
            });
        if !valid {
            global.known_good_config_revisions.remove(&instance_id);
            append_audit_event(
                global,
                &instance_id,
                ConfigRevisionAuditAction::KnownGoodInvalidated,
                None,
                Some(revision_id),
                created_at,
            );
            changed = true;
        }
    }

    let ids: Vec<String> = global.config_revisions.keys().cloned().collect();
    for instance_id in ids {
        let before = global
            .config_revisions
            .get(&instance_id)
            .map_or(0, Vec::len);
        prune_history(global, &instance_id);
        changed |= global
            .config_revisions
            .get(&instance_id)
            .map_or(0, Vec::len)
            != before;
    }
    Ok(changed)
}

pub fn changed_deployment_instance_ids(
    previous: &HashMap<String, InstanceConfig>,
    next: &HashMap<String, InstanceConfig>,
) -> Result<Vec<String>, String> {
    let ids: BTreeSet<&String> = previous.keys().chain(next.keys()).collect();
    let mut changed = Vec::new();
    for instance_id in ids {
        let is_changed = match (previous.get(instance_id), next.get(instance_id)) {
            (Some(before), Some(after)) => {
                deployment_config_fingerprint(before)? != deployment_config_fingerprint(after)?
            }
            _ => true,
        };
        if is_changed {
            changed.push(instance_id.clone());
        }
    }
    Ok(changed)
}

pub fn first_reserved_deployment_change(
    changed_instance_ids: &[String],
    reserved_instance_ids: &HashSet<String>,
) -> Option<String> {
    changed_instance_ids
        .iter()
        .find(|instance_id| reserved_instance_ids.contains(instance_id.as_str()))
        .cloned()
}

pub fn record_saved_config_revisions(
    global: &mut GlobalConfig,
    previous: &HashMap<String, InstanceConfig>,
) -> Result<(), String> {
    let active_ids = active_instance_ids(global);
    let removed: Vec<String> = previous
        .keys()
        .filter(|instance_id| !active_ids.contains(*instance_id))
        .cloned()
        .collect();
    for instance_id in removed {
        global.config_revisions.remove(&instance_id);
        global.known_good_config_revisions.remove(&instance_id);
    }

    let current: Vec<(String, InstanceConfig)> = global
        .instances
        .iter()
        .map(|(instance_id, config)| (instance_id.clone(), config.clone()))
        .collect();
    let created_at = now_epoch_seconds();
    for (instance_id, config) in current {
        let reason = match previous.get(&instance_id) {
            None => Some(ConfigRevisionReason::Created),
            Some(before)
                if deployment_config_fingerprint(before)?
                    != deployment_config_fingerprint(&config)? =>
            {
                Some(ConfigRevisionReason::Save)
            }
            Some(_) => None,
        };
        if let Some(reason) = reason {
            append_revision_at(global, &instance_id, &config, reason, None, created_at)?;
        }
    }
    Ok(())
}

fn bounded_text(value: &str) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(CONFIG_REVISION_VALUE_LIMIT).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn presence_summary(value: &serde_json::Value) -> ConfigValueSummary {
    let set = match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.is_empty(),
        serde_json::Value::Array(value) => !value.is_empty(),
        serde_json::Value::Object(value) => !value.is_empty(),
        _ => true,
    };
    ConfigValueSummary {
        state: if set {
            ConfigValueSummaryState::Set
        } else {
            ConfigValueSummaryState::Empty
        },
        value: None,
        item_count: None,
    }
}

fn item_count_summary(value: &serde_json::Value) -> ConfigValueSummary {
    let count = value.as_array().map_or(0, Vec::len);
    ConfigValueSummary {
        state: ConfigValueSummaryState::ItemCount,
        value: None,
        item_count: Some(count),
    }
}

fn public_value_summary(value: &serde_json::Value) -> ConfigValueSummary {
    if value.is_null() || value.as_str().is_some_and(str::is_empty) {
        return ConfigValueSummary {
            state: ConfigValueSummaryState::Empty,
            value: None,
            item_count: None,
        };
    }
    let display = match value {
        serde_json::Value::String(value) => bounded_text(value),
        _ => bounded_text(&serde_json::to_string(value).unwrap_or_else(|_| "<unavailable>".into())),
    };
    ConfigValueSummary {
        state: ConfigValueSummaryState::Value,
        value: Some(display),
        item_count: None,
    }
}

fn sensitive_field(field: &str) -> bool {
    matches!(
        field,
        "api_key"
            | "api_key_file"
            | "ssl_key_file"
            | "ssl_cert_file"
            | "manual_command"
            | "custom_args"
            | "mcp_servers_config"
            | "mcp_servers_json"
            | "ui_config"
            | "ui_config_file"
    )
}

fn value_summary(field: &str, value: &serde_json::Value) -> ConfigValueSummary {
    if field == "custom_args" {
        item_count_summary(value)
    } else if sensitive_field(field) {
        presence_summary(value)
    } else {
        public_value_summary(value)
    }
}

fn deployment_value_map(
    config: &InstanceConfig,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let value = serde_json::to_value(canonical_deployment_config(config))
        .map_err(|error| format!("failed to summarize deployment configuration: {error}"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| "deployment configuration did not serialize as an object".into())
}

fn summarize_diff(
    before: &InstanceConfig,
    after: &InstanceConfig,
) -> Result<(Vec<ConfigFieldChangeSummary>, bool), String> {
    let before = deployment_value_map(before)?;
    let after = deployment_value_map(after)?;
    let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let total_changes = keys
        .iter()
        .filter(|field| before.get(**field) != after.get(**field))
        .count();
    let changes = keys
        .into_iter()
        .filter(|field| before.get(*field) != after.get(*field))
        .take(CONFIG_REVISION_DIFF_LIMIT)
        .map(|field| {
            let empty = serde_json::Value::Null;
            let before_value = before.get(field).unwrap_or(&empty);
            let after_value = after.get(field).unwrap_or(&empty);
            ConfigFieldChangeSummary {
                field: field.clone(),
                before: value_summary(field, before_value),
                after: value_summary(field, after_value),
                redacted: sensitive_field(field),
            }
        })
        .collect();
    Ok((changes, total_changes > CONFIG_REVISION_DIFF_LIMIT))
}

fn config_revision_error(code: &str, message: impl Into<String>, retryable: bool) -> AppError {
    AppError::new(code, message, retryable)
}

fn validate_instance_id(instance_id: &str) -> AppResult<()> {
    if instance_id.trim().is_empty() {
        return Err(config_revision_error(
            "CONFIG_REVISION_INSTANCE_REQUIRED",
            "instance ID is required",
            false,
        ));
    }
    Ok(())
}

fn current_fingerprint(global: &GlobalConfig, instance_id: &str) -> AppResult<String> {
    let config = global.instances.get(instance_id).ok_or_else(|| {
        config_revision_error(
            "CONFIG_REVISION_INSTANCE_NOT_FOUND",
            "configuration instance was not found",
            false,
        )
        .with_context("instanceId", instance_id)
    })?;
    deployment_config_fingerprint(config).map_err(|error| {
        config_revision_error("CONFIG_REVISION_FINGERPRINT_FAILED", error, false)
            .with_context("instanceId", instance_id)
    })
}

fn revision_location(global: &GlobalConfig, revision_id: &str) -> Vec<String> {
    global
        .config_revisions
        .iter()
        .filter(|(_, history)| history.iter().any(|revision| revision.id == revision_id))
        .map(|(instance_id, _)| instance_id.clone())
        .collect()
}

fn build_history_response(
    global: &GlobalConfig,
    instance_id: &str,
) -> AppResult<ConfigRevisionHistoryResponse> {
    let current_fingerprint = current_fingerprint(global, instance_id)?;
    let history = global.config_revisions.get(instance_id).ok_or_else(|| {
        config_revision_error(
            "CONFIG_REVISION_HISTORY_NOT_FOUND",
            "configuration revision history was not found",
            true,
        )
        .with_context("instanceId", instance_id)
    })?;
    let current_revision_id = history
        .last()
        .map(|revision| revision.id.clone())
        .ok_or_else(|| {
            config_revision_error(
                "CONFIG_REVISION_HISTORY_EMPTY",
                "configuration revision history is empty",
                true,
            )
            .with_context("instanceId", instance_id)
        })?;
    let current_configuration_id = history
        .last()
        .map(|revision| revision.configuration_id.clone())
        .unwrap_or_default();
    let known_good_revision_id = global.known_good_config_revisions.get(instance_id).cloned();
    let by_id: HashMap<&str, &ConfigRevisionRecord> = history
        .iter()
        .map(|revision| (revision.id.as_str(), revision))
        .collect();
    let mut revisions = Vec::with_capacity(history.len());
    for revision in history.iter().rev() {
        let integrity_valid = record_integrity_valid(revision);
        let (changes, diff_truncated) = revision
            .parent_revision_id
            .as_deref()
            .and_then(|parent| by_id.get(parent).copied())
            .filter(|parent| integrity_valid && record_integrity_valid(parent))
            .map(|parent| summarize_diff(&parent.snapshot, &revision.snapshot))
            .transpose()
            .map_err(|error| {
                config_revision_error("CONFIG_REVISION_DIFF_FAILED", error, false)
                    .with_context("revisionId", &revision.id)
            })?
            .unwrap_or_default();
        revisions.push(ConfigRevisionSummary {
            id: revision.id.clone(),
            fingerprint: revision.fingerprint.clone(),
            identity_schema_version: revision.identity_schema_version,
            configuration_id: revision.configuration_id.clone(),
            parent_revision_id: revision.parent_revision_id.clone(),
            created_at: revision.created_at,
            reason: revision.reason,
            rollback_of: revision.rollback_of.clone(),
            current: revision.id == current_revision_id,
            known_good: known_good_revision_id.as_deref() == Some(revision.id.as_str()),
            integrity_valid,
            diff_truncated,
            changes,
        });
    }
    let audit = global
        .config_revision_audit
        .iter()
        .rev()
        .filter(|event| event.instance_id == instance_id)
        .map(|event| ConfigRevisionAuditSummary {
            id: event.id.clone(),
            created_at: event.created_at,
            action: event.action,
            revision_id: event.revision_id.clone(),
            previous_revision_id: event.previous_revision_id.clone(),
        })
        .collect();
    Ok(ConfigRevisionHistoryResponse {
        instance_id: instance_id.to_string(),
        current_fingerprint,
        current_revision_id,
        current_configuration_id,
        known_good_revision_id,
        revisions,
        audit,
    })
}

pub fn resolve_current_config_identity(
    state: &AppState,
    instance_id: &str,
    config: &InstanceConfig,
) -> AppResult<ConfigRevisionIdentity> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let _guard = lock_global_config_for_update(&config_dir).map_err(|error| {
        config_revision_error("DEPLOYMENT_CONFIG_LOCK_FAILED", error, true)
            .with_context("instanceId", instance_id)
    })?;
    let mut global = load_global_config_for_update_unlocked(&config_dir).map_err(|error| {
        config_revision_error("DEPLOYMENT_CONFIG_LOAD_FAILED", error, true)
            .with_context("instanceId", instance_id)
    })?;
    if ensure_current_config_revisions(&mut global).map_err(|error| {
        config_revision_error("DEPLOYMENT_CONFIG_MIGRATION_FAILED", error, false)
            .with_context("instanceId", instance_id)
    })? {
        persist_global_config_unlocked(&config_dir, &global).map_err(|error| {
            config_revision_error("DEPLOYMENT_CONFIG_PERSIST_FAILED", error, true)
                .with_context("instanceId", instance_id)
        })?;
    }
    let persisted = global.instances.get(instance_id).ok_or_else(|| {
        config_revision_error(
            "DEPLOYMENT_CONFIG_NOT_PERSISTED",
            "save this instance configuration before creating a deployment identity",
            true,
        )
        .with_context("instanceId", instance_id)
    })?;
    let requested_fingerprint = deployment_config_fingerprint(config).map_err(|error| {
        config_revision_error("DEPLOYMENT_CONFIG_IDENTITY_FAILED", error, false)
            .with_context("instanceId", instance_id)
    })?;
    let persisted_fingerprint = deployment_config_fingerprint(persisted).map_err(|error| {
        config_revision_error("DEPLOYMENT_CONFIG_IDENTITY_FAILED", error, false)
            .with_context("instanceId", instance_id)
    })?;
    if requested_fingerprint != persisted_fingerprint {
        return Err(config_revision_error(
            "DEPLOYMENT_CONFIG_UNSAVED",
            "the launch configuration differs from the current persisted revision",
            true,
        )
        .with_context("instanceId", instance_id));
    }
    let record = global
        .config_revisions
        .get(instance_id)
        .and_then(|history| history.last())
        .filter(|record| record.fingerprint == requested_fingerprint)
        .ok_or_else(|| {
            config_revision_error(
                "DEPLOYMENT_CONFIG_REVISION_MISSING",
                "the persisted configuration has no matching current revision",
                true,
            )
            .with_context("instanceId", instance_id)
        })?;
    if !record_integrity_valid(record) || record.configuration_id.is_empty() {
        return Err(config_revision_error(
            "DEPLOYMENT_CONFIG_REVISION_INVALID",
            "the current configuration revision failed identity verification",
            false,
        )
        .with_context("instanceId", instance_id)
        .with_context("revisionId", &record.id));
    }
    Ok(ConfigRevisionIdentity {
        revision_id: record.id.clone(),
        configuration_id: record.configuration_id.clone(),
        fingerprint: record.fingerprint.clone(),
    })
}

#[tauri::command]
pub fn list_config_revisions(
    instance_id: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<ConfigRevisionHistoryResponse> {
    validate_instance_id(&instance_id)?;
    let config_dir = state.config_dir.lock().unwrap().clone();
    let _guard = lock_global_config_for_update(&config_dir).map_err(|error| {
        config_revision_error("CONFIG_REVISION_LOCK_FAILED", error, true)
            .with_context("instanceId", &instance_id)
    })?;
    let mut global = load_global_config_for_update_unlocked(&config_dir).map_err(|error| {
        config_revision_error("CONFIG_REVISION_LOAD_FAILED", error, true)
            .with_context("instanceId", &instance_id)
    })?;
    if ensure_current_config_revisions(&mut global).map_err(|error| {
        config_revision_error("CONFIG_REVISION_MIGRATION_FAILED", error, false)
            .with_context("instanceId", &instance_id)
    })? {
        persist_global_config_unlocked(&config_dir, &global).map_err(|error| {
            config_revision_error("CONFIG_REVISION_PERSIST_FAILED", error, true)
                .with_context("instanceId", &instance_id)
        })?;
    }
    build_history_response(&global, &instance_id)
}

fn apply_known_good_to_global(
    global: &mut GlobalConfig,
    instance_id: &str,
    revision_id: &str,
    expected_current_fingerprint: &str,
    created_at: u64,
) -> AppResult<bool> {
    let mut changed = ensure_current_config_revisions_at(global, created_at).map_err(|error| {
        config_revision_error("CONFIG_REVISION_MIGRATION_FAILED", error, false)
            .with_context("instanceId", instance_id)
    })?;
    let actual_fingerprint = current_fingerprint(global, instance_id)?;
    if actual_fingerprint != expected_current_fingerprint {
        return Err(config_revision_error(
            "CONFIG_REVISION_STALE",
            "the active configuration changed; refresh revision history and retry",
            true,
        )
        .with_context("instanceId", instance_id)
        .with_context("expectedFingerprint", expected_current_fingerprint)
        .with_context("actualFingerprint", actual_fingerprint));
    }
    let target = global
        .config_revisions
        .get(instance_id)
        .and_then(|history| history.iter().find(|revision| revision.id == revision_id))
        .ok_or_else(|| {
            let locations = revision_location(global, revision_id);
            let code = if locations.is_empty() {
                "CONFIG_REVISION_NOT_FOUND"
            } else {
                "CONFIG_REVISION_CROSS_INSTANCE"
            };
            config_revision_error(
                code,
                "known-good target is unavailable for this instance",
                false,
            )
            .with_context("instanceId", instance_id)
            .with_context("revisionId", revision_id)
        })?;
    if !record_integrity_valid(target) {
        return Err(config_revision_error(
            "CONFIG_REVISION_CORRUPT",
            "known-good target failed its fingerprint integrity check",
            false,
        )
        .with_context("instanceId", instance_id)
        .with_context("revisionId", revision_id));
    }
    let previous = global
        .known_good_config_revisions
        .insert(instance_id.to_string(), revision_id.to_string());
    if previous.as_deref() != Some(revision_id) {
        append_audit_event(
            global,
            instance_id,
            ConfigRevisionAuditAction::KnownGoodSet,
            Some(revision_id.to_string()),
            previous,
            created_at,
        );
        prune_history(global, instance_id);
        changed = true;
    }
    Ok(changed)
}

#[tauri::command]
pub fn mark_config_revision_known_good(
    instance_id: String,
    revision_id: String,
    expected_current_fingerprint: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<ConfigRevisionHistoryResponse> {
    validate_instance_id(&instance_id)?;
    let config_dir = state.config_dir.lock().unwrap().clone();
    let _guard = lock_global_config_for_update(&config_dir).map_err(|error| {
        config_revision_error("CONFIG_REVISION_LOCK_FAILED", error, true)
            .with_context("instanceId", &instance_id)
    })?;
    let mut global = load_global_config_for_update_unlocked(&config_dir).map_err(|error| {
        config_revision_error("CONFIG_REVISION_LOAD_FAILED", error, true)
            .with_context("instanceId", &instance_id)
    })?;
    if apply_known_good_to_global(
        &mut global,
        &instance_id,
        &revision_id,
        &expected_current_fingerprint,
        now_epoch_seconds(),
    )? {
        persist_global_config_unlocked(&config_dir, &global).map_err(|error| {
            config_revision_error("CONFIG_REVISION_PERSIST_FAILED", error, true)
                .with_context("instanceId", &instance_id)
        })?;
    }
    build_history_response(&global, &instance_id)
}

fn apply_rollback_to_global(
    global: &mut GlobalConfig,
    instance_id: &str,
    revision_id: &str,
    expected_current_fingerprint: &str,
    created_at: u64,
) -> AppResult<InstanceConfig> {
    ensure_current_config_revisions_at(global, created_at).map_err(|error| {
        config_revision_error("CONFIG_REVISION_MIGRATION_FAILED", error, false)
            .with_context("instanceId", instance_id)
    })?;
    let actual_fingerprint = current_fingerprint(global, instance_id)?;
    if actual_fingerprint != expected_current_fingerprint {
        return Err(config_revision_error(
            "CONFIG_REVISION_STALE",
            "the active configuration changed; refresh revision history and retry",
            true,
        )
        .with_context("instanceId", instance_id)
        .with_context("expectedFingerprint", expected_current_fingerprint)
        .with_context("actualFingerprint", actual_fingerprint));
    }

    let target = global
        .config_revisions
        .get(instance_id)
        .and_then(|revisions| revisions.iter().find(|revision| revision.id == revision_id))
        .cloned()
        .ok_or_else(|| {
            let locations = revision_location(global, revision_id);
            let code = if locations.is_empty() {
                "CONFIG_REVISION_NOT_FOUND"
            } else {
                "CONFIG_REVISION_CROSS_INSTANCE"
            };
            config_revision_error(
                code,
                "rollback target is unavailable for this instance",
                false,
            )
            .with_context("instanceId", instance_id)
            .with_context("revisionId", revision_id)
        })?;
    if !record_integrity_valid(&target) {
        return Err(config_revision_error(
            "CONFIG_REVISION_CORRUPT",
            "rollback target failed its fingerprint integrity check",
            false,
        )
        .with_context("instanceId", instance_id)
        .with_context("revisionId", revision_id));
    }
    if target.fingerprint == actual_fingerprint {
        return Err(config_revision_error(
            "CONFIG_REVISION_NOOP",
            "rollback target matches the active deployment configuration",
            false,
        )
        .with_context("instanceId", instance_id)
        .with_context("revisionId", revision_id));
    }

    let current = global.instances.get(instance_id).cloned().ok_or_else(|| {
        config_revision_error(
            "CONFIG_REVISION_INSTANCE_NOT_FOUND",
            "configuration instance was not found",
            false,
        )
        .with_context("instanceId", instance_id)
    })?;
    let mut restored = target.snapshot;
    restored.id = current.id;
    restored.name = current.name;
    let restored_fingerprint = deployment_config_fingerprint(&restored).map_err(|error| {
        config_revision_error("CONFIG_REVISION_FINGERPRINT_FAILED", error, false)
            .with_context("instanceId", instance_id)
    })?;
    if restored_fingerprint != target.fingerprint {
        return Err(config_revision_error(
            "CONFIG_REVISION_CORRUPT",
            "rollback target identity does not match its stored content",
            false,
        )
        .with_context("instanceId", instance_id)
        .with_context("revisionId", revision_id));
    }

    global
        .instances
        .insert(instance_id.to_string(), restored.clone());
    append_revision_at(
        global,
        instance_id,
        &restored,
        ConfigRevisionReason::Rollback,
        Some(revision_id.to_string()),
        created_at,
    )
    .map_err(|error| {
        config_revision_error("CONFIG_REVISION_CREATE_FAILED", error, false)
            .with_context("instanceId", instance_id)
    })?;
    Ok(restored)
}

struct RollbackReservation<'a> {
    instance_id: String,
    starting: &'a Mutex<HashSet<String>>,
}

impl Drop for RollbackReservation<'_> {
    fn drop(&mut self) {
        self.starting.lock().unwrap().remove(&self.instance_id);
    }
}

fn reserve_rollback<'a>(
    state: &'a AppState,
    instance_id: &str,
) -> AppResult<RollbackReservation<'a>> {
    let running = state.running.lock().unwrap();
    let mut starting = state.starting.lock().unwrap();
    if running.contains_key(instance_id) || !starting.insert(instance_id.to_string()) {
        return Err(config_revision_error(
            "CONFIG_REVISION_LIFECYCLE_CONFLICT",
            "stop the instance and cancel any start or recovery before rollback",
            true,
        )
        .with_context("instanceId", instance_id));
    }
    Ok(RollbackReservation {
        instance_id: instance_id.to_string(),
        starting: &state.starting,
    })
}

async fn ensure_runtime_quiet(instance_id: &str) -> AppResult<()> {
    if !crate::runtime_service::manages_instances() {
        return Ok(());
    }
    let status = crate::runtime_service::runtime_status()
        .await
        .map_err(|error| {
            config_revision_error(
                "CONFIG_REVISION_RUNTIME_STATUS_UNAVAILABLE",
                format!("could not verify runtime recovery state: {error}"),
                true,
            )
            .with_context("instanceId", instance_id)
        })?;
    if status.running.contains_key(instance_id) || status.recovery.contains_key(instance_id) {
        return Err(config_revision_error(
            "CONFIG_REVISION_LIFECYCLE_CONFLICT",
            "stop the instance and cancel runtime recovery before rollback",
            true,
        )
        .with_context("instanceId", instance_id));
    }
    Ok(())
}

#[tauri::command]
pub async fn rollback_config_revision(
    instance_id: String,
    revision_id: String,
    expected_current_fingerprint: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<ConfigRevisionRollbackResponse> {
    validate_instance_id(&instance_id)?;
    let _reservation = reserve_rollback(state.inner(), &instance_id)?;
    ensure_runtime_quiet(&instance_id).await?;

    let config_dir = state.config_dir.lock().unwrap().clone();
    let (config, history) = {
        let _guard = lock_global_config_for_update(&config_dir).map_err(|error| {
            config_revision_error("CONFIG_REVISION_LOCK_FAILED", error, true)
                .with_context("instanceId", &instance_id)
        })?;
        if state.running.lock().unwrap().contains_key(&instance_id) {
            return Err(config_revision_error(
                "CONFIG_REVISION_LIFECYCLE_CONFLICT",
                "the instance started while rollback was being prepared",
                true,
            )
            .with_context("instanceId", &instance_id));
        }
        let mut global = load_global_config_for_update_unlocked(&config_dir).map_err(|error| {
            config_revision_error("CONFIG_REVISION_LOAD_FAILED", error, true)
                .with_context("instanceId", &instance_id)
        })?;
        let restored = apply_rollback_to_global(
            &mut global,
            &instance_id,
            &revision_id,
            &expected_current_fingerprint,
            now_epoch_seconds(),
        )?;
        persist_global_config_unlocked(&config_dir, &global).map_err(|error| {
            config_revision_error("CONFIG_REVISION_PERSIST_FAILED", error, true)
                .with_context("instanceId", &instance_id)
        })?;
        state
            .instances
            .lock()
            .unwrap()
            .insert(instance_id.clone(), restored.clone());
        let history = build_history_response(&global, &instance_id)?;
        (restored, history)
    };

    if crate::runtime_service::manages_instances() {
        crate::runtime_service::mark_config_sync_pending();
    }
    Ok(ConfigRevisionRollbackResponse {
        config: crate::models::redact_instance_for_frontend(&config),
        history,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::config::default_global_config;
    use std::path::PathBuf;

    fn instance(id: &str, name: &str, port: u16, api_key: &str) -> InstanceConfig {
        InstanceConfig {
            id: id.into(),
            name: name.into(),
            port,
            api_key: api_key.into(),
            explicit_overrides: Some(vec!["port".into(), "api_key".into()]),
            ..InstanceConfig::default()
        }
    }

    fn global_with(configs: Vec<InstanceConfig>) -> GlobalConfig {
        let mut global = default_global_config();
        global.instances = configs
            .into_iter()
            .map(|config| (config.id.clone(), config))
            .collect();
        global
    }

    fn temp_config_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "llama-server-manager-config-revision-{label}-{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn global_with_two_revisions() -> (GlobalConfig, String, String) {
        let original = instance("one", "Original display", 8080, "first-secret");
        let mut global = global_with(vec![original]);
        ensure_current_config_revisions_at(&mut global, 10).unwrap();
        let baseline_id = global.config_revisions["one"][0].id.clone();
        let previous = global.instances.clone();
        let current = global.instances.get_mut("one").unwrap();
        current.name = "Current display".into();
        current.port = 9090;
        current.api_key = "second-secret".into();
        record_saved_config_revisions(&mut global, &previous).unwrap();
        let current_id = global.config_revisions["one"].last().unwrap().id.clone();
        (global, baseline_id, current_id)
    }

    #[test]
    fn fingerprint_ignores_display_identity_and_canonicalizes_override_order() {
        let mut first = instance("one", "First", 8080, "secret");
        first.explicit_overrides = Some(vec!["port".into(), " api_key ".into(), "port".into()]);
        let mut second = first.clone();
        second.id = "two".into();
        second.name = "Second".into();
        second.explicit_overrides = Some(vec!["api_key".into(), "port".into()]);

        assert_eq!(
            deployment_config_fingerprint(&first).unwrap(),
            deployment_config_fingerprint(&second).unwrap()
        );
        second.port += 1;
        assert_ne!(
            deployment_config_fingerprint(&first).unwrap(),
            deployment_config_fingerprint(&second).unwrap()
        );
    }

    #[test]
    fn migration_seeds_one_baseline_without_changing_the_active_config() {
        let config = instance("one", "Display", 8080, "secret");
        let mut global = global_with(vec![config.clone()]);

        assert!(ensure_current_config_revisions_at(&mut global, 10).unwrap());
        assert_eq!(global.instances["one"], config);
        assert_eq!(global.config_revisions["one"].len(), 1);
        assert_eq!(
            global.config_revisions["one"][0].reason,
            ConfigRevisionReason::Migration
        );
        assert!(!ensure_current_config_revisions_at(&mut global, 11).unwrap());
        assert_eq!(global.config_revisions["one"].len(), 1);
    }

    #[test]
    fn schema_one_migration_preserves_revision_links_and_known_good_evidence() {
        let (mut global, baseline_id, current_id) = global_with_two_revisions();
        global.config_revision_schema_version = 1;
        global
            .known_good_config_revisions
            .insert("one".into(), baseline_id.clone());
        let audit_id = Uuid::new_v4().to_string();
        global.config_revision_audit.push(ConfigRevisionAuditEvent {
            id: audit_id.clone(),
            instance_id: "one".into(),
            created_at: 9,
            action: ConfigRevisionAuditAction::KnownGoodSet,
            revision_id: Some(baseline_id.clone()),
            previous_revision_id: None,
        });
        for record in global.config_revisions.get_mut("one").unwrap() {
            record.configuration_id.clear();
            record.event_integrity = legacy_event_integrity_fingerprint(record).unwrap();
        }

        assert!(ensure_current_config_revisions_at(&mut global, 20).unwrap());

        let history = &global.config_revisions["one"];
        assert_eq!(history[0].id, baseline_id);
        assert_eq!(history[1].id, current_id);
        assert_eq!(
            history[1].parent_revision_id.as_deref(),
            Some(history[0].id.as_str())
        );
        assert!(history.iter().all(|record| {
            !record.configuration_id.is_empty() && record_integrity_valid(record)
        }));
        assert_eq!(global.known_good_config_revisions["one"], history[0].id);
        assert!(global
            .config_revision_audit
            .iter()
            .any(|event| event.id == audit_id));
    }

    #[test]
    fn corrupt_schema_one_revision_is_retained_as_invalid_during_migration() {
        let mut global = global_with(vec![instance("one", "Display", 8080, "secret")]);
        ensure_current_config_revisions_at(&mut global, 10).unwrap();
        global.config_revision_schema_version = 1;
        let corrupt_id = global.config_revisions["one"][0].id.clone();
        {
            let record = &mut global.config_revisions.get_mut("one").unwrap()[0];
            record.configuration_id.clear();
            record.event_integrity = legacy_event_integrity_fingerprint(record).unwrap();
            record.snapshot.port += 1;
        }

        assert!(ensure_current_config_revisions_at(&mut global, 20).unwrap());

        let history = &global.config_revisions["one"];
        let corrupt = history
            .iter()
            .find(|record| record.id == corrupt_id)
            .unwrap();
        assert!(corrupt.configuration_id.is_empty());
        assert!(!record_integrity_valid(corrupt));
        assert!(record_integrity_valid(history.last().unwrap()));
    }

    #[test]
    fn future_revision_schema_is_rejected_without_downgrade() {
        let mut global = global_with(vec![instance("one", "Display", 8080, "secret")]);
        global.config_revision_schema_version = CONFIG_REVISION_SCHEMA_VERSION + 1;

        let error = ensure_current_config_revisions_at(&mut global, 10).unwrap_err();

        assert!(error.contains("newer than supported schema"));
        assert_eq!(
            global.config_revision_schema_version,
            CONFIG_REVISION_SCHEMA_VERSION + 1
        );
        assert!(global.config_revisions.is_empty());
    }

    #[test]
    fn save_revisions_skip_noops_and_display_name_only_edits() {
        let original = instance("one", "Original", 8080, "secret");
        let mut global = global_with(vec![original.clone()]);
        ensure_current_config_revisions_at(&mut global, 10).unwrap();
        let previous = global.instances.clone();

        global.instances.get_mut("one").unwrap().name = "Renamed".into();
        record_saved_config_revisions(&mut global, &previous).unwrap();
        assert_eq!(global.config_revisions["one"].len(), 1);

        let previous = global.instances.clone();
        global.instances.get_mut("one").unwrap().port = 9090;
        record_saved_config_revisions(&mut global, &previous).unwrap();
        assert_eq!(global.config_revisions["one"].len(), 2);
        assert_eq!(
            global.config_revisions["one"][1].reason,
            ConfigRevisionReason::Save
        );
    }

    #[test]
    fn diff_redacts_every_private_category_and_bounds_large_public_values() {
        let before = instance("one", "One", 8080, "");
        let mut after = before.clone();
        let private_values = [
            "api-secret-marker",
            "credential-file-marker",
            "tls-key-marker",
            "tls-cert-marker",
            "manual-command-marker",
            "custom-argument-marker",
            "mcp-config-marker",
            "mcp-json-marker",
            "ui-config-marker",
            "ui-file-marker",
        ];
        after.api_key = private_values[0].into();
        after.api_key_file = private_values[1].into();
        after.ssl_key_file = private_values[2].into();
        after.ssl_cert_file = private_values[3].into();
        after.manual_command = private_values[4].into();
        after.custom_args = vec![private_values[5].into()];
        after.mcp_servers_config = private_values[6].into();
        after.mcp_servers_json = private_values[7].into();
        after.ui_config = private_values[8].into();
        after.ui_config_file = private_values[9].into();
        after.chat_template = "x".repeat(CONFIG_REVISION_VALUE_LIMIT + 20);

        let (changes, truncated) = summarize_diff(&before, &after).unwrap();
        assert!(!truncated);
        let serialized = serde_json::to_string(&changes).unwrap();
        for private_value in private_values {
            assert!(!serialized.contains(private_value));
        }
        for field in [
            "api_key",
            "api_key_file",
            "ssl_key_file",
            "ssl_cert_file",
            "manual_command",
            "custom_args",
            "mcp_servers_config",
            "mcp_servers_json",
            "ui_config",
            "ui_config_file",
        ] {
            assert!(changes
                .iter()
                .any(|change| change.field == field && change.redacted));
        }
        assert!(serialized.contains("item_count"));
        let template = changes
            .iter()
            .find(|change| change.field == "chat_template")
            .unwrap();
        assert!(template.after.value.as_deref().unwrap().ends_with('…'));
    }

    #[test]
    fn retention_preserves_latest_and_known_good_revision() {
        let mut global = global_with(vec![instance("one", "One", 8000, "")]);
        ensure_current_config_revisions_at(&mut global, 1).unwrap();
        let known_good = global.config_revisions["one"][0].id.clone();
        global
            .known_good_config_revisions
            .insert("one".into(), known_good.clone());

        for index in 1..=(CONFIG_REVISION_HISTORY_LIMIT + 10) {
            let mut config = global.instances["one"].clone();
            config.port = 8000 + index as u16;
            global.instances.insert("one".into(), config.clone());
            append_revision_at(
                &mut global,
                "one",
                &config,
                ConfigRevisionReason::Save,
                None,
                index as u64 + 1,
            )
            .unwrap();
        }

        let history = &global.config_revisions["one"];
        assert_eq!(history.len(), CONFIG_REVISION_HISTORY_LIMIT);
        assert!(history.iter().any(|revision| revision.id == known_good));
        assert_eq!(
            history.last().unwrap().fingerprint,
            deployment_config_fingerprint(&global.instances["one"]).unwrap()
        );
    }

    #[test]
    fn invalid_known_good_pointer_is_cleared_and_audited() {
        let mut global = global_with(vec![instance("one", "One", 8080, "")]);
        global
            .known_good_config_revisions
            .insert("one".into(), "missing".into());

        ensure_current_config_revisions_at(&mut global, 42).unwrap();

        assert!(!global.known_good_config_revisions.contains_key("one"));
        assert_eq!(global.config_revision_audit.len(), 1);
        assert_eq!(
            global.config_revision_audit[0].action,
            ConfigRevisionAuditAction::KnownGoodInvalidated
        );
    }

    #[test]
    fn corrupt_known_good_pointer_is_cleared_and_audited() {
        let mut global = global_with(vec![instance("one", "One", 8080, "")]);
        ensure_current_config_revisions_at(&mut global, 1).unwrap();
        let known_good = global.config_revisions["one"][0].id.clone();
        global
            .known_good_config_revisions
            .insert("one".into(), known_good);
        global.config_revisions.get_mut("one").unwrap()[0]
            .snapshot
            .port = 9999;

        assert!(ensure_current_config_revisions_at(&mut global, 42).unwrap());

        assert!(!global.known_good_config_revisions.contains_key("one"));
        assert_eq!(global.config_revision_audit.len(), 1);
        assert_eq!(
            global.config_revision_audit[0].action,
            ConfigRevisionAuditAction::KnownGoodInvalidated
        );
        assert_eq!(global.config_revisions["one"].len(), 2);
        assert!(record_integrity_valid(
            global.config_revisions["one"].last().unwrap()
        ));
    }

    #[test]
    fn repeated_known_good_action_reports_pending_migration_for_persistence() {
        let (mut global, baseline_id, _) = global_with_two_revisions();
        global
            .known_good_config_revisions
            .insert("one".into(), baseline_id.clone());
        global.instances.get_mut("one").unwrap().port = 9191;
        let expected = current_fingerprint(&global, "one").unwrap();

        assert!(
            apply_known_good_to_global(&mut global, "one", &baseline_id, &expected, 200,).unwrap()
        );
        assert_eq!(global.config_revisions["one"].len(), 3);
        assert!(global.config_revision_audit.is_empty());
        assert!(
            !apply_known_good_to_global(&mut global, "one", &baseline_id, &expected, 201,).unwrap()
        );
    }

    #[test]
    fn corrupted_revision_is_visible_but_fails_integrity() {
        let mut global = global_with(vec![instance("one", "One", 8080, "")]);
        ensure_current_config_revisions_at(&mut global, 1).unwrap();
        global.config_revisions.get_mut("one").unwrap()[0]
            .snapshot
            .port = 9999;

        let response = build_history_response(&global, "one").unwrap();

        assert!(!response.revisions[0].integrity_valid);
        assert!(response.revisions[0].changes.is_empty());
    }

    #[test]
    fn tampered_revision_event_identity_fails_integrity() {
        let mut global = global_with(vec![instance("one", "One", 8080, "")]);
        ensure_current_config_revisions_at(&mut global, 1).unwrap();
        global.config_revisions.get_mut("one").unwrap()[0].parent_revision_id =
            Some("forged-parent".into());

        let response = build_history_response(&global, "one").unwrap();

        assert!(!response.revisions[0].integrity_valid);
        assert!(response.revisions[0].changes.is_empty());
    }

    #[test]
    fn rollback_preserves_display_identity_and_appends_an_immutable_event() {
        let (mut global, baseline_id, current_id) = global_with_two_revisions();
        let expected = current_fingerprint(&global, "one").unwrap();

        let restored =
            apply_rollback_to_global(&mut global, "one", &baseline_id, &expected, 99).unwrap();

        assert_eq!(restored.id, "one");
        assert_eq!(restored.name, "Current display");
        assert_eq!(restored.port, 8080);
        assert_eq!(restored.api_key, "first-secret");
        let rollback = global.config_revisions["one"].last().unwrap();
        assert_eq!(
            rollback.parent_revision_id.as_deref(),
            Some(current_id.as_str())
        );
        assert_eq!(rollback.rollback_of.as_deref(), Some(baseline_id.as_str()));
        assert_eq!(rollback.reason, ConfigRevisionReason::Rollback);
        assert_eq!(rollback.created_at, 99);
        assert_eq!(rollback.snapshot, restored);
        assert_eq!(
            rollback.fingerprint,
            deployment_config_fingerprint(&restored).unwrap()
        );
        assert_eq!(global.config_revisions["one"].len(), 3);
    }

    #[test]
    fn rollback_rejects_stale_noop_unknown_cross_instance_and_corrupt_targets() {
        let (global, baseline_id, _) = global_with_two_revisions();
        let expected = current_fingerprint(&global, "one").unwrap();

        let mut stale = global_with_two_revisions().0;
        let error = apply_rollback_to_global(&mut stale, "one", &baseline_id, "sha256:stale", 100)
            .unwrap_err();
        assert_eq!(error.code, "CONFIG_REVISION_STALE");

        let (mut noop, _, noop_current_id) = global_with_two_revisions();
        let noop_expected = current_fingerprint(&noop, "one").unwrap();
        let error =
            apply_rollback_to_global(&mut noop, "one", &noop_current_id, &noop_expected, 100)
                .unwrap_err();
        assert_eq!(error.code, "CONFIG_REVISION_NOOP");

        let mut missing = global_with_two_revisions().0;
        let error =
            apply_rollback_to_global(&mut missing, "one", "pruned-or-unknown", &expected, 100)
                .unwrap_err();
        assert_eq!(error.code, "CONFIG_REVISION_NOT_FOUND");

        let mut cross = global;
        let other = instance("two", "Other", 7070, "");
        cross.instances.insert("two".into(), other);
        ensure_current_config_revisions_at(&mut cross, 101).unwrap();
        let cross_id = cross.config_revisions["two"][0].id.clone();
        let error =
            apply_rollback_to_global(&mut cross, "one", &cross_id, &expected, 102).unwrap_err();
        assert_eq!(error.code, "CONFIG_REVISION_CROSS_INSTANCE");

        let (mut corrupt, corrupt_id, _) = global_with_two_revisions();
        corrupt.config_revisions.get_mut("one").unwrap()[0]
            .snapshot
            .port = 6553;
        let error =
            apply_rollback_to_global(&mut corrupt, "one", &corrupt_id, &expected, 103).unwrap_err();
        assert_eq!(error.code, "CONFIG_REVISION_CORRUPT");
    }

    #[test]
    fn rollback_reservation_blocks_concurrent_deployment_saves_but_not_display_edits() {
        let original = instance("one", "Original", 8080, "");
        let previous = HashMap::from([("one".to_string(), original.clone())]);
        let mut renamed = previous.clone();
        renamed.get_mut("one").unwrap().name = "Renamed".into();
        let mut deployment_change = renamed.clone();
        deployment_change.get_mut("one").unwrap().port = 9090;
        let reserved = HashSet::from(["one".to_string()]);

        let display_changes = changed_deployment_instance_ids(&previous, &renamed).unwrap();
        assert!(display_changes.is_empty());
        assert_eq!(
            first_reserved_deployment_change(&display_changes, &reserved),
            None
        );

        let deployment_changes =
            changed_deployment_instance_ids(&previous, &deployment_change).unwrap();
        assert_eq!(
            first_reserved_deployment_change(&deployment_changes, &reserved).as_deref(),
            Some("one")
        );
    }

    #[test]
    fn failed_rollback_persistence_keeps_the_durable_configuration_unchanged() {
        let dir = temp_config_dir("persist-failure");
        let (mut staged, baseline_id, _) = global_with_two_revisions();
        persist_global_config_unlocked(&dir, &staged).unwrap();
        let durable_before = std::fs::read(dir.join("instances.json")).unwrap();
        let expected = current_fingerprint(&staged, "one").unwrap();
        apply_rollback_to_global(&mut staged, "one", &baseline_id, &expected, 200).unwrap();

        let invalid_config_dir = dir.join("not-a-directory");
        std::fs::write(&invalid_config_dir, b"block directory creation").unwrap();
        assert!(persist_global_config_unlocked(&invalid_config_dir, &staged).is_err());

        assert_eq!(
            std::fs::read(dir.join("instances.json")).unwrap(),
            durable_before
        );
        let durable: GlobalConfig = serde_json::from_slice(&durable_before).unwrap();
        assert_eq!(durable.instances["one"].port, 9090);
        assert_eq!(durable.config_revisions["one"].len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
