use crate::deployment::validated_current_revision;
use crate::models::{EngineInfo, GlobalConfig, RunningInstance};
use crate::resource_planner::{plan_instance_resources, CapacitySnapshot, ResourcePlan};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const RESIDENCY_SCHEMA_VERSION: u32 = 1;
const RESIDENCY_AUDIT_LIMIT: usize = 128;
const DEFAULT_DRAIN_TIMEOUT_SECONDS: u64 = 120;
// Residency performs aggregate budget enforcement itself. A deliberately large
// planner capacity prevents per-candidate live headroom from making the same
// declarative policy fluctuate as unrelated processes allocate memory, while
// still using the planner's worst-case resource estimates.
const PLANNER_ESTIMATE_CAPACITY_BYTES: u64 = 1_u64 << 50;

pub const fn default_residency_schema_version() -> u32 {
    RESIDENCY_SCHEMA_VERSION
}

fn default_drain_timeout_seconds() -> u64 {
    DEFAULT_DRAIN_TIMEOUT_SECONDS
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidencyIntent {
    pub instance_id: String,
    pub priority: i32,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidencyPolicy {
    pub enabled: bool,
    pub ram_budget_bytes: u64,
    pub vram_budget_bytes: u64,
    #[serde(default = "default_drain_timeout_seconds")]
    pub drain_timeout_seconds: u64,
    #[serde(default)]
    pub intents: Vec<ResidencyIntent>,
}

impl Default for ResidencyPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            ram_budget_bytes: 0,
            vram_budget_bytes: 0,
            drain_timeout_seconds: DEFAULT_DRAIN_TIMEOUT_SECONDS,
            intents: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidencyPlacementPhase {
    #[default]
    Evicted,
    Resident,
    Draining,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidencyPlacementRecord {
    pub instance_id: String,
    pub deployment_id: String,
    pub revision_id: String,
    pub phase: ResidencyPlacementPhase,
    pub plan_id: String,
    pub updated_at: u64,
    #[serde(default)]
    pub routing_drained: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidencyAuditEvent {
    pub id: String,
    pub recorded_at: u64,
    pub action: String,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidencyOperationKind {
    Drain,
    Evict,
    Warm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidencyOperation {
    pub sequence: u32,
    pub kind: ResidencyOperationKind,
    pub instance_id: String,
    pub deployment_id: String,
    pub revision_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidencyDecision {
    pub instance_id: String,
    pub instance_name: String,
    pub priority: i32,
    pub intent_enabled: bool,
    pub selected: bool,
    pub deployment_id: Option<String>,
    pub revision_id: Option<String>,
    pub running_revision_id: Option<String>,
    pub resource_status: String,
    pub ram_bytes: u64,
    pub vram_bytes: u64,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidencyPlan {
    pub schema_version: u32,
    pub plan_id: String,
    pub generated_at: u64,
    pub ram_budget_bytes: u64,
    pub ram_used_bytes: u64,
    pub vram_budget_bytes: u64,
    pub vram_used_bytes: u64,
    pub decisions: Vec<ResidencyDecision>,
    pub operations: Vec<ResidencyOperation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidencyInspection {
    pub policy: ResidencyPolicy,
    pub plan: ResidencyPlan,
    pub placements: Vec<ResidencyPlacementRecord>,
    pub audit: Vec<ResidencyAuditEvent>,
    pub registered_rpc_workers: usize,
    pub worker_agent_available: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanIdentity<'a> {
    schema_version: u32,
    policy: &'a ResidencyPolicy,
    ram_used_bytes: u64,
    vram_used_bytes: u64,
    decisions: &'a [ResidencyDecision],
    operations: &'a [ResidencyOperation],
}

#[allow(clippy::too_many_arguments)] // Audit linkage stays explicit at every durable transition.
fn push_audit(
    global: &mut GlobalConfig,
    action: &str,
    outcome: &str,
    instance_id: Option<&str>,
    deployment_id: Option<&str>,
    revision_id: Option<&str>,
    plan_id: Option<&str>,
    message: Option<String>,
) {
    global.residency_audit.push(ResidencyAuditEvent {
        id: uuid::Uuid::new_v4().to_string(),
        recorded_at: now_epoch_seconds(),
        action: action.to_string(),
        outcome: outcome.to_string(),
        instance_id: instance_id.map(str::to_string),
        deployment_id: deployment_id.map(str::to_string),
        revision_id: revision_id.map(str::to_string),
        plan_id: plan_id.map(str::to_string),
        message,
    });
    if global.residency_audit.len() > RESIDENCY_AUDIT_LIMIT {
        let remove = global.residency_audit.len() - RESIDENCY_AUDIT_LIMIT;
        global.residency_audit.drain(0..remove);
    }
}

pub fn ensure_residency_catalog(global: &mut GlobalConfig) -> Result<bool, String> {
    if global.residency_schema_version > RESIDENCY_SCHEMA_VERSION {
        return Err(format!(
            "residency schema {} is newer than supported schema {}",
            global.residency_schema_version, RESIDENCY_SCHEMA_VERSION
        ));
    }
    let mut changed = global.residency_schema_version < RESIDENCY_SCHEMA_VERSION;
    global.residency_schema_version = RESIDENCY_SCHEMA_VERSION;
    if global.residency_policy.drain_timeout_seconds == 0 {
        global.residency_policy.drain_timeout_seconds = DEFAULT_DRAIN_TIMEOUT_SECONDS;
        changed = true;
    }
    if global.residency_audit.len() > RESIDENCY_AUDIT_LIMIT {
        let remove = global.residency_audit.len() - RESIDENCY_AUDIT_LIMIT;
        global.residency_audit.drain(0..remove);
        changed = true;
    }
    let stale_intents = global
        .residency_policy
        .intents
        .iter()
        .filter(|intent| !global.instances.contains_key(&intent.instance_id))
        .map(|intent| intent.instance_id.clone())
        .collect::<Vec<_>>();
    if !stale_intents.is_empty() {
        global
            .residency_policy
            .intents
            .retain(|intent| global.instances.contains_key(&intent.instance_id));
        for instance_id in stale_intents {
            push_audit(
                global,
                "stale_intent_removed",
                "success",
                Some(&instance_id),
                None,
                None,
                None,
                Some("instance was removed; placement evidence was retained".into()),
            );
        }
        changed = true;
    }
    Ok(changed)
}

pub fn validate_policy(global: &GlobalConfig, policy: &ResidencyPolicy) -> Result<(), String> {
    if policy.enabled && policy.ram_budget_bytes == 0 {
        return Err("enabled residency scheduling requires a non-zero RAM budget".into());
    }
    if !(5..=3_600).contains(&policy.drain_timeout_seconds) {
        return Err("residency drain timeout must be between 5 and 3600 seconds".into());
    }
    let mut ids = BTreeSet::new();
    for intent in &policy.intents {
        let instance_id = intent.instance_id.trim();
        if instance_id.is_empty() || !global.instances.contains_key(instance_id) {
            return Err(format!(
                "residency policy references unknown instance {}",
                intent.instance_id
            ));
        }
        if !ids.insert(instance_id) {
            return Err(format!(
                "residency policy contains duplicate instance {instance_id}"
            ));
        }
    }
    Ok(())
}

pub fn set_policy(global: &mut GlobalConfig, mut policy: ResidencyPolicy) -> Result<(), String> {
    ensure_residency_catalog(global)?;
    for intent in &mut policy.intents {
        intent.instance_id = intent.instance_id.trim().to_string();
    }
    policy.intents.sort_by(|left, right| {
        (left.priority, &left.instance_id).cmp(&(right.priority, &right.instance_id))
    });
    validate_policy(global, &policy)?;
    global.residency_policy = policy;
    push_audit(
        global,
        "policy_updated",
        "success",
        None,
        None,
        None,
        None,
        None,
    );
    Ok(())
}

fn engine_for_config<'a>(
    global: &GlobalConfig,
    engines: &'a [EngineInfo],
    instance_id: &str,
) -> Option<&'a EngineInfo> {
    let config = global.instances.get(instance_id)?;
    let selected = if config.engine_id.trim().is_empty() {
        global.default_engine_id.trim()
    } else {
        config.engine_id.trim()
    };
    if selected.is_empty() {
        return None;
    }
    let selected_key = crate::path_utils::path_identity_key(Path::new(selected));
    engines
        .iter()
        .find(|engine| crate::path_utils::path_identity_key(Path::new(&engine.id)) == selected_key)
}

fn blocked_decision(
    global: &GlobalConfig,
    intent: &ResidencyIntent,
    reason: &str,
    running: &HashMap<String, RunningInstance>,
) -> ResidencyDecision {
    ResidencyDecision {
        instance_id: intent.instance_id.clone(),
        instance_name: global
            .instances
            .get(&intent.instance_id)
            .map(|config| config.name.clone())
            .unwrap_or_else(|| intent.instance_id.clone()),
        priority: intent.priority,
        intent_enabled: intent.enabled,
        selected: false,
        deployment_id: None,
        revision_id: None,
        running_revision_id: running
            .get(&intent.instance_id)
            .map(|item| item.deployment_revision_id.clone()),
        resource_status: "blocked".into(),
        ram_bytes: 0,
        vram_bytes: 0,
        reasons: vec![reason.into()],
    }
}

fn candidate_decision(
    global: &GlobalConfig,
    engines: &[EngineInfo],
    running: &HashMap<String, RunningInstance>,
    active_canary_instances: &HashSet<String>,
    intent: &ResidencyIntent,
    capacity: CapacitySnapshot,
) -> ResidencyDecision {
    if !intent.enabled {
        return blocked_decision(global, intent, "intent_disabled", running);
    }
    if active_canary_instances.contains(&intent.instance_id) {
        return blocked_decision(global, intent, "unresolved_canary_rollout", running);
    }
    let revision = match validated_current_revision(global, &intent.instance_id) {
        Ok(revision) => revision,
        Err(_) => return blocked_decision(global, intent, "current_revision_unavailable", running),
    };
    let Some(config) = global.instances.get(&intent.instance_id) else {
        return blocked_decision(global, intent, "instance_unavailable", running);
    };
    let Some(engine) = engine_for_config(global, engines, &intent.instance_id) else {
        return blocked_decision(global, intent, "engine_unavailable", running);
    };
    let resource_plan: ResourcePlan = plan_instance_resources(config, &engine.backend, capacity);
    ResidencyDecision {
        instance_id: intent.instance_id.clone(),
        instance_name: config.name.clone(),
        priority: intent.priority,
        intent_enabled: true,
        selected: false,
        deployment_id: Some(revision.deployment_id),
        revision_id: Some(revision.id),
        running_revision_id: running
            .get(&intent.instance_id)
            .map(|item| item.deployment_revision_id.clone()),
        resource_status: resource_plan.status.clone(),
        ram_bytes: resource_plan.ram.required.max_bytes,
        vram_bytes: resource_plan.vram.required.max_bytes,
        reasons: if resource_plan.status == "feasible" {
            Vec::new()
        } else {
            vec![format!("resource_plan_{}", resource_plan.status)]
        },
    }
}

fn placement_owned(global: &GlobalConfig, instance_id: &str) -> bool {
    global.residency_placements.iter().any(|placement| {
        placement.instance_id == instance_id && placement.phase != ResidencyPlacementPhase::Evicted
    })
}

fn build_operations(
    global: &GlobalConfig,
    decisions: &[ResidencyDecision],
) -> Vec<ResidencyOperation> {
    let mut drains = Vec::new();
    let mut evictions = Vec::new();
    let mut warms = Vec::new();
    for decision in decisions {
        let explicitly_unselected = decision.reasons.iter().any(|reason| {
            matches!(
                reason.as_str(),
                "intent_disabled" | "ram_budget_exhausted" | "vram_budget_exhausted"
            )
        });
        let Some(running_revision) = decision.running_revision_id.as_deref() else {
            if decision.selected {
                if let (Some(deployment_id), Some(revision_id)) =
                    (&decision.deployment_id, &decision.revision_id)
                {
                    warms.push((
                        decision.priority,
                        decision.instance_id.clone(),
                        deployment_id.clone(),
                        revision_id.clone(),
                        "selected_revision_not_running".to_string(),
                    ));
                }
            }
            continue;
        };
        let desired_revision = decision.revision_id.as_deref();
        let replacement_required = decision.selected && desired_revision != Some(running_revision);
        let eviction_required = replacement_required
            || (!decision.selected
                && explicitly_unselected
                && placement_owned(global, &decision.instance_id));
        if eviction_required {
            let deployment_id = decision
                .deployment_id
                .clone()
                .or_else(|| {
                    global
                        .residency_placements
                        .iter()
                        .find(|placement| placement.instance_id == decision.instance_id)
                        .map(|placement| placement.deployment_id.clone())
                })
                .unwrap_or_default();
            let reason = if replacement_required {
                "running_revision_is_stale"
            } else {
                "placement_no_longer_selected"
            };
            drains.push((
                decision.priority,
                decision.instance_id.clone(),
                deployment_id.clone(),
                running_revision.to_string(),
                reason.to_string(),
            ));
            evictions.push((
                decision.priority,
                decision.instance_id.clone(),
                deployment_id,
                running_revision.to_string(),
                reason.to_string(),
            ));
            if replacement_required {
                if let (Some(deployment_id), Some(revision_id)) =
                    (&decision.deployment_id, &decision.revision_id)
                {
                    warms.push((
                        decision.priority,
                        decision.instance_id.clone(),
                        deployment_id.clone(),
                        revision_id.clone(),
                        "replace_stale_revision".to_string(),
                    ));
                }
            }
        } else if decision.selected
            && !global.residency_placements.iter().any(|placement| {
                placement.instance_id == decision.instance_id
                    && placement.phase == ResidencyPlacementPhase::Resident
                    && placement.revision_id == running_revision
            })
        {
            if let (Some(deployment_id), Some(revision_id)) =
                (&decision.deployment_id, &decision.revision_id)
            {
                warms.push((
                    decision.priority,
                    decision.instance_id.clone(),
                    deployment_id.clone(),
                    revision_id.clone(),
                    "adopt_running_revision".to_string(),
                ));
            }
        }
    }
    let stable_sort = |items: &mut Vec<(i32, String, String, String, String)>| {
        items.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    };
    stable_sort(&mut drains);
    stable_sort(&mut evictions);
    stable_sort(&mut warms);
    let mut operations = Vec::new();
    for (kind, items) in [
        (ResidencyOperationKind::Drain, drains),
        (ResidencyOperationKind::Evict, evictions),
        (ResidencyOperationKind::Warm, warms),
    ] {
        for (_, instance_id, deployment_id, revision_id, reason) in items {
            operations.push(ResidencyOperation {
                sequence: operations.len() as u32 + 1,
                kind: kind.clone(),
                instance_id,
                deployment_id,
                revision_id,
                reason,
            });
        }
    }
    operations
}

pub fn build_plan(
    global: &GlobalConfig,
    engines: &[EngineInfo],
    running: &HashMap<String, RunningInstance>,
) -> Result<ResidencyPlan, String> {
    validate_policy(global, &global.residency_policy)?;
    let policy = &global.residency_policy;
    let capacity = CapacitySnapshot {
        ram_total_bytes: Some(PLANNER_ESTIMATE_CAPACITY_BYTES),
        ram_available_bytes: Some(PLANNER_ESTIMATE_CAPACITY_BYTES),
        vram_total_bytes: Some(PLANNER_ESTIMATE_CAPACITY_BYTES),
        vram_available_bytes: Some(PLANNER_ESTIMATE_CAPACITY_BYTES),
    };
    let active_canary_instances = crate::canary::active_instance_ids(global);
    let mut intents = policy.intents.clone();
    intents.sort_by(|left, right| {
        (left.priority, &left.instance_id).cmp(&(right.priority, &right.instance_id))
    });
    let mut decisions = intents
        .iter()
        .map(|intent| {
            candidate_decision(
                global,
                engines,
                running,
                &active_canary_instances,
                intent,
                capacity,
            )
        })
        .collect::<Vec<_>>();
    let mut ram_used = 0_u64;
    let mut vram_used = 0_u64;
    if policy.enabled {
        for decision in &mut decisions {
            if decision.resource_status != "feasible" || !decision.intent_enabled {
                continue;
            }
            let next_ram = ram_used.saturating_add(decision.ram_bytes);
            let next_vram = vram_used.saturating_add(decision.vram_bytes);
            if next_ram > policy.ram_budget_bytes {
                decision.reasons.push("ram_budget_exhausted".into());
                continue;
            }
            if next_vram > policy.vram_budget_bytes {
                decision.reasons.push("vram_budget_exhausted".into());
                continue;
            }
            decision.selected = true;
            decision.reasons.push("selected_within_budget".into());
            ram_used = next_ram;
            vram_used = next_vram;
        }
    } else {
        for decision in &mut decisions {
            decision.reasons.insert(0, "scheduler_disabled".into());
        }
    }
    let operations = if policy.enabled {
        build_operations(global, &decisions)
    } else {
        Vec::new()
    };
    let identity = PlanIdentity {
        schema_version: RESIDENCY_SCHEMA_VERSION,
        policy,
        ram_used_bytes: ram_used,
        vram_used_bytes: vram_used,
        decisions: &decisions,
        operations: &operations,
    };
    let bytes = serde_json::to_vec(&identity)
        .map_err(|error| format!("failed to serialize residency plan identity: {error}"))?;
    let plan_id = format!("sha256:{:x}", Sha256::digest(bytes));
    Ok(ResidencyPlan {
        schema_version: RESIDENCY_SCHEMA_VERSION,
        plan_id,
        generated_at: now_epoch_seconds(),
        ram_budget_bytes: policy.ram_budget_bytes,
        ram_used_bytes: ram_used,
        vram_budget_bytes: policy.vram_budget_bytes,
        vram_used_bytes: vram_used,
        decisions,
        operations,
    })
}

pub fn inspection(
    global: &GlobalConfig,
    engines: &[EngineInfo],
    running: &HashMap<String, RunningInstance>,
    registered_rpc_workers: usize,
) -> Result<ResidencyInspection, String> {
    let mut placements = global.residency_placements.clone();
    placements.sort_by(|left, right| left.instance_id.cmp(&right.instance_id));
    let mut audit = global.residency_audit.clone();
    audit.reverse();
    Ok(ResidencyInspection {
        policy: global.residency_policy.clone(),
        plan: build_plan(global, engines, running)?,
        placements,
        audit,
        registered_rpc_workers,
        worker_agent_available: false,
    })
}

pub fn draining_instance_ids(global: &GlobalConfig) -> HashSet<String> {
    global
        .residency_placements
        .iter()
        .filter(|placement| placement.routing_drained)
        .map(|placement| placement.instance_id.clone())
        .collect()
}

#[allow(clippy::too_many_arguments)] // Placement identity and recovery flags are one atomic update.
fn upsert_placement(
    global: &mut GlobalConfig,
    instance_id: &str,
    deployment_id: &str,
    revision_id: &str,
    plan_id: &str,
    phase: ResidencyPlacementPhase,
    routing_drained: bool,
    last_error: Option<String>,
) {
    let placement = global
        .residency_placements
        .iter_mut()
        .find(|placement| placement.instance_id == instance_id);
    let next = ResidencyPlacementRecord {
        instance_id: instance_id.to_string(),
        deployment_id: deployment_id.to_string(),
        revision_id: revision_id.to_string(),
        phase,
        plan_id: plan_id.to_string(),
        updated_at: now_epoch_seconds(),
        routing_drained,
        last_error,
    };
    if let Some(placement) = placement {
        *placement = next;
    } else {
        global.residency_placements.push(next);
    }
}

pub fn begin_drain(
    global: &mut GlobalConfig,
    operation: &ResidencyOperation,
    plan_id: &str,
) -> Result<(), String> {
    if operation.kind != ResidencyOperationKind::Drain {
        return Err("residency drain requires a drain operation".into());
    }
    upsert_placement(
        global,
        &operation.instance_id,
        &operation.deployment_id,
        &operation.revision_id,
        plan_id,
        ResidencyPlacementPhase::Draining,
        true,
        None,
    );
    push_audit(
        global,
        "drain",
        "started",
        Some(&operation.instance_id),
        Some(&operation.deployment_id),
        Some(&operation.revision_id),
        Some(plan_id),
        None,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn finish_operation(
    global: &mut GlobalConfig,
    action: &str,
    instance_id: &str,
    deployment_id: &str,
    revision_id: &str,
    plan_id: &str,
    success: bool,
    error: Option<String>,
) -> Result<bool, String> {
    let (phase, routing_drained, clear_drain) = match (action, success) {
        ("warm", true) => (ResidencyPlacementPhase::Resident, false, true),
        ("warm", false) => (ResidencyPlacementPhase::Failed, false, true),
        ("evict", true) => (ResidencyPlacementPhase::Evicted, false, true),
        ("evict", false) => (ResidencyPlacementPhase::Failed, true, false),
        _ => return Err("residency completion action must be warm or evict".into()),
    };
    upsert_placement(
        global,
        instance_id,
        deployment_id,
        revision_id,
        plan_id,
        phase,
        routing_drained,
        error.clone(),
    );
    push_audit(
        global,
        action,
        if success { "success" } else { "failed" },
        Some(instance_id),
        Some(deployment_id),
        Some(revision_id),
        Some(plan_id),
        error,
    );
    Ok(clear_drain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::config::default_global_config;
    use crate::deployment_identity::DeploymentIdentity;
    use crate::models::{EngineCapabilities, InstanceConfig};
    use std::path::PathBuf;

    struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "llama-server-manager-residency-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn identity(seed: &str) -> DeploymentIdentity {
        DeploymentIdentity::new(
            format!("urn:lsm:engine:v1:sha256:{seed}"),
            format!("urn:lsm:model:v1:sha256:{seed}"),
            format!("revision-{seed}"),
            format!("urn:lsm:configuration:v1:sha256:{seed}"),
            format!("urn:lsm:qualification:v2:sha256:{seed}"),
        )
        .unwrap()
    }

    fn fixture() -> (TempDirGuard, GlobalConfig, Vec<EngineInfo>) {
        let temp = TempDirGuard::new();
        let model = temp.path().join("model.gguf");
        std::fs::write(&model, vec![0_u8; 1024]).unwrap();
        let engine_id = temp.path().join("engine").to_string_lossy().to_string();
        let mut global = default_global_config();
        global.default_engine_id = engine_id.clone();
        for (instance_id, priority) in [("one", 10), ("two", 20)] {
            global.instances.insert(
                instance_id.into(),
                InstanceConfig {
                    id: instance_id.into(),
                    name: instance_id.into(),
                    model_path: model.to_string_lossy().to_string(),
                    engine_id: engine_id.clone(),
                    gpu_layers: 0,
                    ctx_size: 128,
                    parallel: 1,
                    ..InstanceConfig::default()
                },
            );
            crate::deployment::ensure_deployments(&mut global).unwrap();
            crate::deployment::materialize_revision(
                &mut global,
                instance_id,
                &identity(instance_id),
            )
            .unwrap();
            global.residency_policy.intents.push(ResidencyIntent {
                instance_id: instance_id.into(),
                priority,
                enabled: true,
            });
        }
        global.residency_policy.enabled = true;
        global.residency_policy.ram_budget_bytes = 64 * 1024 * 1024 * 1024;
        global.residency_policy.vram_budget_bytes = 0;
        let engines = vec![EngineInfo {
            id: engine_id.clone(),
            name: "Engine".into(),
            dir: engine_id.clone(),
            exe: engine_id,
            version: "test".into(),
            backend: "cpu".into(),
            custom_name: None,
            capabilities: EngineCapabilities::default(),
            artifact_identity: Default::default(),
        }];
        (temp, global, engines)
    }

    #[test]
    fn identical_inputs_produce_the_same_plan_identity_and_order() {
        let (_temp, global, engines) = fixture();
        let first = build_plan(&global, &engines, &HashMap::new()).unwrap();
        let second = build_plan(&global, &engines, &HashMap::new()).unwrap();
        assert_eq!(first.plan_id, second.plan_id);
        assert_eq!(
            first
                .decisions
                .iter()
                .map(|decision| decision.instance_id.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        assert!(first
            .operations
            .iter()
            .all(|operation| operation.kind == ResidencyOperationKind::Warm));
    }

    #[test]
    fn worst_case_accounting_never_exceeds_declared_budgets() {
        let (_temp, mut global, engines) = fixture();
        let baseline = build_plan(&global, &engines, &HashMap::new()).unwrap();
        let first = baseline.decisions[0].ram_bytes;
        global.residency_policy.ram_budget_bytes = first;
        let plan = build_plan(&global, &engines, &HashMap::new()).unwrap();
        assert_eq!(plan.ram_used_bytes, first);
        assert!(plan.ram_used_bytes <= plan.ram_budget_bytes);
        assert_eq!(
            plan.decisions.iter().filter(|item| item.selected).count(),
            1
        );
        assert!(plan.decisions[1]
            .reasons
            .contains(&"ram_budget_exhausted".into()));
    }

    #[test]
    fn unknown_resource_inputs_fail_closed() {
        let (_temp, mut global, engines) = fixture();
        global.instances.get_mut("one").unwrap().custom_args = vec!["--unknown".into()];
        let plan = build_plan(&global, &engines, &HashMap::new()).unwrap();
        let decision = plan
            .decisions
            .iter()
            .find(|decision| decision.instance_id == "one")
            .unwrap();
        assert!(!decision.selected);
        assert!(decision.reasons.contains(&"resource_plan_unknown".into()));
    }

    #[test]
    fn unfinished_drain_and_failed_eviction_remain_persistently_drained() {
        let (_temp, mut global, engines) = fixture();
        let plan = build_plan(&global, &engines, &HashMap::new()).unwrap();
        let warm = plan.operations.first().unwrap();
        finish_operation(
            &mut global,
            "warm",
            &warm.instance_id,
            &warm.deployment_id,
            &warm.revision_id,
            &plan.plan_id,
            true,
            None,
        )
        .unwrap();
        global.residency_policy.intents[0].enabled = false;
        let running = HashMap::from([(
            warm.instance_id.clone(),
            RunningInstance {
                instance_id: warm.instance_id.clone(),
                pid: 1,
                port: 8080,
                host: "127.0.0.1".into(),
                start_time: 1,
                executable_path: "engine".into(),
                telemetry_session_id: None,
                workload: "inference".into(),
                launch_config: None,
                deployment_identity: DeploymentIdentity::default(),
                deployment_id: warm.deployment_id.clone(),
                deployment_revision_id: warm.revision_id.clone(),
            },
        )]);
        let evict_plan = build_plan(&global, &engines, &running).unwrap();
        let drain = evict_plan
            .operations
            .iter()
            .find(|operation| operation.kind == ResidencyOperationKind::Drain)
            .unwrap();
        begin_drain(&mut global, drain, &evict_plan.plan_id).unwrap();
        assert!(draining_instance_ids(&global).contains(&drain.instance_id));
        assert!(!finish_operation(
            &mut global,
            "evict",
            &drain.instance_id,
            &drain.deployment_id,
            &drain.revision_id,
            &evict_plan.plan_id,
            false,
            Some("stop failed".into()),
        )
        .unwrap());
        assert!(draining_instance_ids(&global).contains(&drain.instance_id));
        let restored: GlobalConfig =
            serde_json::from_slice(&serde_json::to_vec(&global).unwrap()).unwrap();
        assert!(draining_instance_ids(&restored).contains(&drain.instance_id));
    }

    #[test]
    fn future_schema_and_stale_intents_fail_or_migrate_without_losing_placements() {
        let (_temp, mut global, _engines) = fixture();
        global.residency_schema_version = RESIDENCY_SCHEMA_VERSION + 1;
        assert!(ensure_residency_catalog(&mut global).is_err());

        global.residency_schema_version = RESIDENCY_SCHEMA_VERSION;
        global.residency_placements.push(ResidencyPlacementRecord {
            instance_id: "one".into(),
            deployment_id: "deployment".into(),
            revision_id: "revision".into(),
            phase: ResidencyPlacementPhase::Resident,
            plan_id: "plan".into(),
            updated_at: 1,
            routing_drained: false,
            last_error: None,
        });
        global.instances.remove("one");
        assert!(ensure_residency_catalog(&mut global).unwrap());
        assert!(!global
            .residency_policy
            .intents
            .iter()
            .any(|intent| intent.instance_id == "one"));
        assert!(global
            .residency_placements
            .iter()
            .any(|placement| placement.instance_id == "one"));
        assert!(global
            .residency_audit
            .iter()
            .any(|event| event.action == "stale_intent_removed"));
    }

    #[test]
    fn unresolved_canary_rollouts_hold_owned_placements_without_lifecycle_actions() {
        let (_temp, mut global, engines) = fixture();
        let running = ["one", "two"]
            .into_iter()
            .map(|instance_id| {
                let revision = validated_current_revision(&global, instance_id).unwrap();
                (
                    instance_id.to_string(),
                    RunningInstance {
                        instance_id: instance_id.to_string(),
                        pid: if instance_id == "one" { 1 } else { 2 },
                        port: if instance_id == "one" { 8080 } else { 8081 },
                        host: "127.0.0.1".into(),
                        start_time: 1,
                        executable_path: "engine".into(),
                        telemetry_session_id: None,
                        workload: "inference".into(),
                        launch_config: None,
                        deployment_identity: revision.deployment_identity,
                        deployment_id: revision.deployment_id,
                        deployment_revision_id: revision.id,
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        let current = validated_current_revision(&global, "one").unwrap();
        global.residency_placements.push(ResidencyPlacementRecord {
            instance_id: "one".into(),
            deployment_id: current.deployment_id,
            revision_id: current.id,
            phase: ResidencyPlacementPhase::Resident,
            plan_id: "previous".into(),
            updated_at: 1,
            routing_drained: false,
            last_error: None,
        });
        global.proxy_config.enabled = true;
        crate::canary::create_rollout(
            &mut global,
            &running,
            true,
            crate::canary::CanaryRolloutCreate {
                stable_instance_id: "one",
                candidate_instance_id: "two",
                model_alias: "public-model",
                candidate_weight: 10,
            },
            1,
        )
        .unwrap();

        let plan = build_plan(&global, &engines, &running).unwrap();
        assert!(plan.decisions.iter().all(|decision| {
            !decision.selected
                && decision
                    .reasons
                    .contains(&"unresolved_canary_rollout".into())
        }));
        assert!(plan.operations.is_empty());
    }
}
