use crate::models::{GlobalConfig, ProxyRoute, RunningInstance};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

pub const CANARY_SCHEMA_VERSION: u32 = 1;
const CANARY_RECORD_SCHEMA_VERSION: u8 = 1;
const CANARY_HISTORY_LIMIT: usize = 32;
const CANARY_AUDIT_LIMIT: usize = 128;
const MAX_MODEL_ALIAS_BYTES: usize = 512;

pub const fn default_canary_schema_version() -> u32 {
    CANARY_SCHEMA_VERSION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanaryRolloutState {
    Active,
    Promoted,
    Aborted,
    RolledBack,
}

impl CanaryRolloutState {
    fn owns_routes(self) -> bool {
        matches!(self, Self::Active | Self::Promoted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanaryAuditKind {
    Created,
    Observed,
    TrafficChanged,
    Promoted,
    Aborted,
    RolledBack,
    DriftDetected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanaryRequestEvidence {
    pub total: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub latest_completed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft_p95_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_wait_p95_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_reuse_basis_points: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanaryAuditEvent {
    pub sequence: u64,
    pub occurred_at: i64,
    pub kind: CanaryAuditKind,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable_evidence: Option<CanaryRequestEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_evidence: Option<CanaryRequestEvidence>,
    #[serde(default)]
    previous_integrity: String,
    #[serde(default)]
    integrity: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanaryRolloutRecord {
    #[serde(default)]
    pub schema_version: u8,
    pub id: String,
    pub model_alias: String,
    pub state: CanaryRolloutState,
    pub stable_instance_id: String,
    pub candidate_instance_id: String,
    pub stable_revision_id: String,
    pub candidate_revision_id: String,
    pub stable_route_id: String,
    pub candidate_route_id: String,
    pub candidate_weight: u32,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    expected_routes: Vec<ProxyRoute>,
    #[serde(default)]
    audit_anchor_integrity: String,
    #[serde(default)]
    next_event_sequence: u64,
    #[serde(default)]
    pub events: Vec<CanaryAuditEvent>,
    #[serde(default)]
    integrity: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanaryTargetHealth {
    pub instance_id: String,
    pub status: String,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanaryAuditEventView {
    pub sequence: u64,
    pub occurred_at: i64,
    pub kind: CanaryAuditKind,
    pub summary: String,
    pub stable_evidence: Option<CanaryRequestEvidence>,
    pub candidate_evidence: Option<CanaryRequestEvidence>,
    pub integrity_valid: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanaryRolloutInspection {
    pub id: String,
    pub model_alias: String,
    pub state: CanaryRolloutState,
    pub stable_instance_id: String,
    pub candidate_instance_id: String,
    pub stable_revision_id: String,
    pub candidate_revision_id: String,
    pub stable_weight: u32,
    pub candidate_weight: u32,
    pub created_at: i64,
    pub updated_at: i64,
    pub integrity_valid: bool,
    pub drift: Vec<String>,
    pub can_change_traffic: bool,
    pub can_promote: bool,
    pub can_abort: bool,
    pub can_rollback: bool,
    pub stable_health: CanaryTargetHealth,
    pub candidate_health: CanaryTargetHealth,
    pub stable_evidence: Option<CanaryRequestEvidence>,
    pub candidate_evidence: Option<CanaryRequestEvidence>,
    pub events: Vec<CanaryAuditEventView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventIntegrityMaterial<'a> {
    sequence: u64,
    occurred_at: i64,
    kind: CanaryAuditKind,
    summary: &'a str,
    stable_evidence: &'a Option<CanaryRequestEvidence>,
    candidate_evidence: &'a Option<CanaryRequestEvidence>,
    previous_integrity: &'a str,
}

fn expected_event_integrity(event: &CanaryAuditEvent) -> Result<String, String> {
    let bytes = serde_json::to_vec(&EventIntegrityMaterial {
        sequence: event.sequence,
        occurred_at: event.occurred_at,
        kind: event.kind,
        summary: &event.summary,
        stable_evidence: &event.stable_evidence,
        candidate_evidence: &event.candidate_evidence,
        previous_integrity: &event.previous_integrity,
    })
    .map_err(|error| format!("failed to serialize canary audit integrity: {error}"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn event_integrity_valid(event: &CanaryAuditEvent) -> bool {
    expected_event_integrity(event).is_ok_and(|expected| expected == event.integrity)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordIntegrityMaterial<'a> {
    schema_version: u8,
    id: &'a str,
    model_alias: &'a str,
    state: CanaryRolloutState,
    stable_instance_id: &'a str,
    candidate_instance_id: &'a str,
    stable_revision_id: &'a str,
    candidate_revision_id: &'a str,
    stable_route_id: &'a str,
    candidate_route_id: &'a str,
    candidate_weight: u32,
    created_at: i64,
    updated_at: i64,
    expected_routes: &'a [ProxyRoute],
    audit_anchor_integrity: &'a str,
    next_event_sequence: u64,
    events: &'a [CanaryAuditEvent],
}

fn expected_record_integrity(record: &CanaryRolloutRecord) -> Result<String, String> {
    let bytes = serde_json::to_vec(&RecordIntegrityMaterial {
        schema_version: record.schema_version,
        id: &record.id,
        model_alias: &record.model_alias,
        state: record.state,
        stable_instance_id: &record.stable_instance_id,
        candidate_instance_id: &record.candidate_instance_id,
        stable_revision_id: &record.stable_revision_id,
        candidate_revision_id: &record.candidate_revision_id,
        stable_route_id: &record.stable_route_id,
        candidate_route_id: &record.candidate_route_id,
        candidate_weight: record.candidate_weight,
        created_at: record.created_at,
        updated_at: record.updated_at,
        expected_routes: &record.expected_routes,
        audit_anchor_integrity: &record.audit_anchor_integrity,
        next_event_sequence: record.next_event_sequence,
        events: &record.events,
    })
    .map_err(|error| format!("failed to serialize canary rollout integrity: {error}"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn reseal(record: &mut CanaryRolloutRecord) -> Result<(), String> {
    record.integrity = expected_record_integrity(record)?;
    Ok(())
}

fn validate_record(record: &CanaryRolloutRecord) -> Result<(), String> {
    if record.schema_version != CANARY_RECORD_SCHEMA_VERSION {
        return Err(format!(
            "canary rollout {} has an unsupported schema",
            record.id
        ));
    }
    if record.id.trim().is_empty()
        || record.model_alias.trim().is_empty()
        || record.stable_instance_id.trim().is_empty()
        || record.candidate_instance_id.trim().is_empty()
        || record.stable_instance_id == record.candidate_instance_id
        || record.stable_revision_id.trim().is_empty()
        || record.candidate_revision_id.trim().is_empty()
        || record.stable_route_id.trim().is_empty()
        || record.candidate_route_id.trim().is_empty()
        || record.stable_route_id == record.candidate_route_id
    {
        return Err(format!(
            "canary rollout {} has invalid identity fields",
            record.id
        ));
    }
    if record.next_event_sequence == 0 || record.events.is_empty() {
        return Err(format!("canary rollout {} has no audit history", record.id));
    }
    let mut previous = record.audit_anchor_integrity.as_str();
    let mut expected_sequence = record.events[0].sequence;
    for event in &record.events {
        if event.sequence != expected_sequence
            || event.previous_integrity != previous
            || !event_integrity_valid(event)
        {
            return Err(format!(
                "canary rollout {} audit integrity is invalid",
                record.id
            ));
        }
        previous = &event.integrity;
        expected_sequence = expected_sequence.saturating_add(1);
    }
    if record.next_event_sequence != expected_sequence {
        return Err(format!(
            "canary rollout {} audit sequence is invalid",
            record.id
        ));
    }
    if record.state.owns_routes() {
        if record.expected_routes.len() != 2 {
            return Err(format!(
                "canary rollout {} route snapshot is invalid",
                record.id
            ));
        }
        let stable = record
            .expected_routes
            .iter()
            .find(|route| route.id == record.stable_route_id);
        let candidate = record
            .expected_routes
            .iter()
            .find(|route| route.id == record.candidate_route_id);
        let routes_valid = stable.is_some_and(|route| {
            route.model_alias == record.model_alias
                && route.target_instance_id == record.stable_instance_id
                && route.required_deployment_revision_id == record.stable_revision_id
                && route.priority == 0
                && route.max_concurrent_requests == 0
        }) && candidate.is_some_and(|route| {
            route.model_alias == record.model_alias
                && route.target_instance_id == record.candidate_instance_id
                && route.required_deployment_revision_id == record.candidate_revision_id
                && route.priority == 0
                && route.max_concurrent_requests == 0
        });
        let state_valid = match (record.state, stable, candidate) {
            (CanaryRolloutState::Active, Some(stable), Some(candidate)) => {
                (1..=50).contains(&record.candidate_weight)
                    && stable.enabled
                    && candidate.enabled
                    && stable.weight == 100 - record.candidate_weight
                    && candidate.weight == record.candidate_weight
            }
            (CanaryRolloutState::Promoted, Some(stable), Some(candidate)) => {
                record.candidate_weight == 100
                    && !stable.enabled
                    && candidate.enabled
                    && stable.weight == 1
                    && candidate.weight == 100
            }
            _ => false,
        };
        if !routes_valid || !state_valid {
            return Err(format!(
                "canary rollout {} route ownership is inconsistent",
                record.id
            ));
        }
    } else if !record.expected_routes.is_empty() {
        return Err(format!(
            "completed canary rollout {} still owns routes",
            record.id
        ));
    } else if record.candidate_weight != 0 {
        return Err(format!(
            "completed canary rollout {} has a non-zero traffic share",
            record.id
        ));
    }
    let expected_integrity = expected_record_integrity(record)?;
    if expected_integrity != record.integrity {
        return Err(format!("canary rollout {} integrity is invalid", record.id));
    }
    Ok(())
}

fn append_event(
    record: &mut CanaryRolloutRecord,
    kind: CanaryAuditKind,
    summary: String,
    occurred_at: i64,
    stable_evidence: Option<CanaryRequestEvidence>,
    candidate_evidence: Option<CanaryRequestEvidence>,
) -> Result<(), String> {
    let previous_integrity = record
        .events
        .last()
        .map(|event| event.integrity.clone())
        .unwrap_or_else(|| record.audit_anchor_integrity.clone());
    let mut event = CanaryAuditEvent {
        sequence: record.next_event_sequence,
        occurred_at,
        kind,
        summary,
        stable_evidence,
        candidate_evidence,
        previous_integrity,
        integrity: String::new(),
    };
    event.integrity = expected_event_integrity(&event)?;
    record.events.push(event);
    record.next_event_sequence = record.next_event_sequence.saturating_add(1);
    while record.events.len() > CANARY_AUDIT_LIMIT {
        let removed = record.events.remove(0);
        record.audit_anchor_integrity = removed.integrity;
    }
    record.updated_at = occurred_at;
    reseal(record)
}

pub fn ensure_canary_catalog(global: &mut GlobalConfig) -> Result<bool, String> {
    if global.canary_schema_version > CANARY_SCHEMA_VERSION {
        return Err(format!(
            "canary schema {} is newer than supported schema {}",
            global.canary_schema_version, CANARY_SCHEMA_VERSION
        ));
    }
    let mut changed = global.canary_schema_version < CANARY_SCHEMA_VERSION;
    global.canary_schema_version = CANARY_SCHEMA_VERSION;
    for record in &global.canary_rollouts {
        validate_record(record)?;
    }
    if global
        .canary_rollouts
        .iter()
        .filter(|record| record.state.owns_routes())
        .count()
        > 1
    {
        return Err("multiple open canary rollouts are not supported".into());
    }
    while global.canary_rollouts.len() > CANARY_HISTORY_LIMIT {
        let Some(index) = global
            .canary_rollouts
            .iter()
            .enumerate()
            .filter(|(_, record)| !record.state.owns_routes())
            .min_by_key(|(_, record)| record.updated_at)
            .map(|(index, _)| index)
        else {
            break;
        };
        global.canary_rollouts.remove(index);
        changed = true;
    }
    Ok(changed)
}

fn running_revision<'a>(
    running: &'a HashMap<String, RunningInstance>,
    instance_id: &str,
) -> Result<&'a RunningInstance, String> {
    running
        .get(instance_id)
        .ok_or_else(|| format!("instance {instance_id} is not running"))
}

fn validated_binding(
    global: &GlobalConfig,
    running: &HashMap<String, RunningInstance>,
    instance_id: &str,
) -> Result<String, String> {
    let revision = crate::deployment::validated_current_revision(global, instance_id)?;
    let live = running_revision(running, instance_id)?;
    if live.deployment_revision_id != revision.id {
        return Err(format!(
            "instance {instance_id} is not running its current deployment revision"
        ));
    }
    if live.deployment_id != revision.deployment_id {
        return Err(format!(
            "instance {instance_id} runtime deployment identity does not match"
        ));
    }
    Ok(revision.id)
}

fn matching_workload(left: &RunningInstance, right: &RunningInstance) -> bool {
    fn normalize(value: &str) -> &str {
        let value = value.trim();
        if value.is_empty() {
            "inference"
        } else {
            value
        }
    }
    normalize(&left.workload) == normalize(&right.workload)
}

fn route(id: String, alias: &str, instance_id: &str, revision_id: &str, weight: u32) -> ProxyRoute {
    ProxyRoute {
        id,
        enabled: true,
        model_alias: alias.to_string(),
        target_instance_id: instance_id.to_string(),
        required_deployment_revision_id: revision_id.to_string(),
        priority: 0,
        weight: weight.max(1),
        max_concurrent_requests: 0,
    }
}

pub struct CanaryRolloutCreate<'a> {
    pub stable_instance_id: &'a str,
    pub candidate_instance_id: &'a str,
    pub model_alias: &'a str,
    pub candidate_weight: u32,
}

pub fn create_rollout(
    global: &mut GlobalConfig,
    running: &HashMap<String, RunningInstance>,
    proxy_running: bool,
    request: CanaryRolloutCreate<'_>,
    now_ms: i64,
) -> Result<String, String> {
    ensure_canary_catalog(global)?;
    if global
        .canary_rollouts
        .iter()
        .any(|record| record.state.owns_routes())
    {
        return Err(
            "finish or roll back the current canary rollout before creating another".into(),
        );
    }
    let stable_instance_id = request.stable_instance_id.trim();
    let candidate_instance_id = request.candidate_instance_id.trim();
    let model_alias = request.model_alias.trim();
    let candidate_weight = request.candidate_weight;
    if stable_instance_id.is_empty()
        || candidate_instance_id.is_empty()
        || stable_instance_id == candidate_instance_id
    {
        return Err("canary rollout requires two distinct managed instances".into());
    }
    if model_alias.is_empty() || model_alias.len() > MAX_MODEL_ALIAS_BYTES {
        return Err("canary rollout requires a valid public model alias".into());
    }
    if !(1..=50).contains(&candidate_weight) {
        return Err("candidate traffic must be between 1 and 50 percent before promotion".into());
    }
    if !global.proxy_config.enabled || !proxy_running {
        return Err("start the routing proxy before creating a canary rollout".into());
    }
    if !global.proxy_config.canary_routes.is_empty() {
        return Err("unowned canary routing state already exists".into());
    }
    let stable_revision_id = validated_binding(global, running, stable_instance_id)?;
    let candidate_revision_id = validated_binding(global, running, candidate_instance_id)?;
    let stable_running = running_revision(running, stable_instance_id)?;
    let candidate_running = running_revision(running, candidate_instance_id)?;
    if !matching_workload(stable_running, candidate_running) {
        return Err("stable and candidate instances must expose the same workload type".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let stable_route_id = format!("canary-{id}-stable");
    let candidate_route_id = format!("canary-{id}-candidate");
    let expected_routes = vec![
        route(
            stable_route_id.clone(),
            model_alias,
            stable_instance_id,
            &stable_revision_id,
            100 - candidate_weight,
        ),
        route(
            candidate_route_id.clone(),
            model_alias,
            candidate_instance_id,
            &candidate_revision_id,
            candidate_weight,
        ),
    ];
    global.proxy_config.canary_routes = expected_routes.clone();
    let mut record = CanaryRolloutRecord {
        schema_version: CANARY_RECORD_SCHEMA_VERSION,
        id: id.clone(),
        model_alias: model_alias.to_string(),
        state: CanaryRolloutState::Active,
        stable_instance_id: stable_instance_id.to_string(),
        candidate_instance_id: candidate_instance_id.to_string(),
        stable_revision_id,
        candidate_revision_id,
        stable_route_id,
        candidate_route_id,
        candidate_weight,
        created_at: now_ms,
        updated_at: now_ms,
        expected_routes,
        audit_anchor_integrity: String::new(),
        next_event_sequence: 1,
        events: Vec::new(),
        integrity: String::new(),
    };
    append_event(
        &mut record,
        CanaryAuditKind::Created,
        format!("canary activated at {candidate_weight}% candidate traffic"),
        now_ms,
        None,
        None,
    )?;
    validate_record(&record)?;
    global.canary_rollouts.push(record);
    ensure_canary_catalog(global)?;
    Ok(id)
}

fn record_index(global: &GlobalConfig, rollout_id: &str) -> Result<usize, String> {
    global
        .canary_rollouts
        .iter()
        .position(|record| record.id == rollout_id)
        .ok_or_else(|| format!("canary rollout {rollout_id} does not exist"))
}

pub fn drift_reasons(
    record: &CanaryRolloutRecord,
    global: &GlobalConfig,
    running: &HashMap<String, RunningInstance>,
    proxy_running: bool,
) -> Vec<String> {
    if !record.state.owns_routes() {
        return Vec::new();
    }
    let mut drift = Vec::new();
    if !global.proxy_config.enabled || !proxy_running {
        drift.push("routing proxy is not running".to_string());
    }
    for (role, instance_id, revision_id) in [
        (
            "stable",
            &record.stable_instance_id,
            &record.stable_revision_id,
        ),
        (
            "candidate",
            &record.candidate_instance_id,
            &record.candidate_revision_id,
        ),
    ] {
        match crate::deployment::validated_current_revision(global, instance_id) {
            Ok(revision) if revision.id == *revision_id => {}
            Ok(_) => drift.push(format!("{role} deployment revision changed")),
            Err(error) => drift.push(format!("{role} deployment is invalid: {error}")),
        }
        match running.get(instance_id) {
            Some(live) if live.deployment_revision_id == *revision_id => {}
            Some(_) => drift.push(format!("{role} runtime revision changed")),
            None => drift.push(format!("{role} instance is not running")),
        }
    }
    if global.proxy_config.canary_routes != record.expected_routes {
        drift.push("canary routing overlay changed outside the rollout workflow".into());
    }
    drift
}

fn assert_progress_safe(
    record: &CanaryRolloutRecord,
    global: &GlobalConfig,
    running: &HashMap<String, RunningInstance>,
    proxy_running: bool,
) -> Result<(), String> {
    let drift = drift_reasons(record, global, running, proxy_running);
    if drift.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "canary rollout is blocked by drift: {}",
            drift.join("; ")
        ))
    }
}

fn assert_overlay_safe(record: &CanaryRolloutRecord, global: &GlobalConfig) -> Result<(), String> {
    if global.proxy_config.canary_routes == record.expected_routes {
        Ok(())
    } else {
        Err("canary routing overlay changed outside the rollout workflow".into())
    }
}

pub fn set_weight(
    global: &mut GlobalConfig,
    running: &HashMap<String, RunningInstance>,
    proxy_running: bool,
    rollout_id: &str,
    candidate_weight: u32,
    now_ms: i64,
) -> Result<(), String> {
    ensure_canary_catalog(global)?;
    if !(1..=50).contains(&candidate_weight) {
        return Err("candidate traffic must be between 1 and 50 percent before promotion".into());
    }
    let index = record_index(global, rollout_id)?;
    {
        let record = &global.canary_rollouts[index];
        if record.state != CanaryRolloutState::Active {
            return Err("traffic can only change while a canary rollout is active".into());
        }
        assert_progress_safe(record, global, running, proxy_running)?;
    }
    let record = &mut global.canary_rollouts[index];
    for route in &mut record.expected_routes {
        if route.id == record.stable_route_id {
            route.weight = 100 - candidate_weight;
        } else if route.id == record.candidate_route_id {
            route.weight = candidate_weight;
        }
    }
    global.proxy_config.canary_routes = record.expected_routes.clone();
    record.candidate_weight = candidate_weight;
    append_event(
        record,
        CanaryAuditKind::TrafficChanged,
        format!("candidate traffic changed to {candidate_weight}%"),
        now_ms,
        None,
        None,
    )
}

pub fn promote(
    global: &mut GlobalConfig,
    running: &HashMap<String, RunningInstance>,
    proxy_running: bool,
    candidate_ready: bool,
    rollout_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    ensure_canary_catalog(global)?;
    let index = record_index(global, rollout_id)?;
    {
        let record = &global.canary_rollouts[index];
        if record.state != CanaryRolloutState::Active {
            return Err("only an active canary rollout can be promoted".into());
        }
        assert_progress_safe(record, global, running, proxy_running)?;
    }
    if !candidate_ready {
        return Err(
            "candidate health is not ready; observe and recover it before promotion".into(),
        );
    }
    let record = &mut global.canary_rollouts[index];
    for route in &mut record.expected_routes {
        if route.id == record.stable_route_id {
            route.enabled = false;
            route.weight = 1;
        } else if route.id == record.candidate_route_id {
            route.enabled = true;
            route.weight = 100;
        }
    }
    global.proxy_config.canary_routes = record.expected_routes.clone();
    record.candidate_weight = 100;
    record.state = CanaryRolloutState::Promoted;
    append_event(
        record,
        CanaryAuditKind::Promoted,
        "candidate promoted to 100% traffic; rollback remains available".into(),
        now_ms,
        None,
        None,
    )
}

fn restore(
    global: &mut GlobalConfig,
    rollout_id: &str,
    expected_state: CanaryRolloutState,
    next_state: CanaryRolloutState,
    kind: CanaryAuditKind,
    summary: &str,
    now_ms: i64,
) -> Result<(), String> {
    ensure_canary_catalog(global)?;
    let index = record_index(global, rollout_id)?;
    {
        let record = &global.canary_rollouts[index];
        if record.state != expected_state {
            return Err(format!(
                "canary rollout is not in the required {expected_state:?} state"
            ));
        }
        assert_overlay_safe(record, global)?;
    }
    global.proxy_config.canary_routes.clear();
    let record = &mut global.canary_rollouts[index];
    record.expected_routes.clear();
    record.candidate_weight = 0;
    record.state = next_state;
    append_event(record, kind, summary.into(), now_ms, None, None)
}

pub fn abort(global: &mut GlobalConfig, rollout_id: &str, now_ms: i64) -> Result<(), String> {
    restore(
        global,
        rollout_id,
        CanaryRolloutState::Active,
        CanaryRolloutState::Aborted,
        CanaryAuditKind::Aborted,
        "canary aborted and routing overlay removed",
        now_ms,
    )
}

pub fn rollback(global: &mut GlobalConfig, rollout_id: &str, now_ms: i64) -> Result<(), String> {
    restore(
        global,
        rollout_id,
        CanaryRolloutState::Promoted,
        CanaryRolloutState::RolledBack,
        CanaryAuditKind::RolledBack,
        "promotion rolled back and routing overlay removed",
        now_ms,
    )
}

pub fn record_observation(
    global: &mut GlobalConfig,
    running: &HashMap<String, RunningInstance>,
    proxy_running: bool,
    rollout_id: &str,
    stable_evidence: CanaryRequestEvidence,
    candidate_evidence: CanaryRequestEvidence,
    now_ms: i64,
) -> Result<(), String> {
    ensure_canary_catalog(global)?;
    let index = record_index(global, rollout_id)?;
    if !global.canary_rollouts[index].state.owns_routes() {
        return Err("only an active or promoted canary rollout can be observed".into());
    }
    let drift = {
        let record = &global.canary_rollouts[index];
        drift_reasons(record, global, running, proxy_running)
    };
    let kind = if drift.is_empty() {
        CanaryAuditKind::Observed
    } else {
        CanaryAuditKind::DriftDetected
    };
    let summary = if drift.is_empty() {
        format!(
            "observation captured: stable {}/{}, candidate {}/{} successful",
            stable_evidence.succeeded,
            stable_evidence.total,
            candidate_evidence.succeeded,
            candidate_evidence.total
        )
    } else {
        format!("observation detected drift: {}", drift.join("; "))
    };
    let record = &mut global.canary_rollouts[index];
    append_event(
        record,
        kind,
        summary,
        now_ms,
        Some(stable_evidence),
        Some(candidate_evidence),
    )
}

fn latest_evidence(
    record: &CanaryRolloutRecord,
) -> (Option<CanaryRequestEvidence>, Option<CanaryRequestEvidence>) {
    record
        .events
        .iter()
        .rev()
        .find_map(|event| {
            event
                .stable_evidence
                .as_ref()
                .zip(event.candidate_evidence.as_ref())
                .map(|(stable, candidate)| (Some(stable.clone()), Some(candidate.clone())))
        })
        .unwrap_or((None, None))
}

fn event_views(record: &CanaryRolloutRecord) -> Vec<CanaryAuditEventView> {
    let mut previous = record.audit_anchor_integrity.as_str();
    let mut expected_sequence = record
        .events
        .first()
        .map(|event| event.sequence)
        .unwrap_or(1);
    let mut views = record
        .events
        .iter()
        .map(|event| {
            let integrity_valid = event.sequence == expected_sequence
                && event.previous_integrity == previous
                && event_integrity_valid(event);
            previous = &event.integrity;
            expected_sequence = expected_sequence.saturating_add(1);
            CanaryAuditEventView {
                sequence: event.sequence,
                occurred_at: event.occurred_at,
                kind: event.kind,
                summary: event.summary.clone(),
                stable_evidence: event.stable_evidence.clone(),
                candidate_evidence: event.candidate_evidence.clone(),
                integrity_valid,
            }
        })
        .collect::<Vec<_>>();
    views.reverse();
    views
}

pub fn inspect_rollout(
    record: &CanaryRolloutRecord,
    global: &GlobalConfig,
    running: &HashMap<String, RunningInstance>,
    proxy_running: bool,
    health: &HashMap<String, CanaryTargetHealth>,
) -> CanaryRolloutInspection {
    let integrity_valid = validate_record(record).is_ok();
    let drift = if integrity_valid {
        drift_reasons(record, global, running, proxy_running)
    } else {
        vec!["rollout or audit integrity is invalid".into()]
    };
    let (stable_evidence, candidate_evidence) = latest_evidence(record);
    let health_for = |instance_id: &str| {
        health
            .get(instance_id)
            .cloned()
            .unwrap_or_else(|| CanaryTargetHealth {
                instance_id: instance_id.to_string(),
                status: "unknown".into(),
                ready: false,
            })
    };
    let progress_ready = integrity_valid && drift.is_empty();
    let overlay_safe =
        integrity_valid && global.proxy_config.canary_routes == record.expected_routes;
    CanaryRolloutInspection {
        id: record.id.clone(),
        model_alias: record.model_alias.clone(),
        state: record.state,
        stable_instance_id: record.stable_instance_id.clone(),
        candidate_instance_id: record.candidate_instance_id.clone(),
        stable_revision_id: record.stable_revision_id.clone(),
        candidate_revision_id: record.candidate_revision_id.clone(),
        stable_weight: if record.state == CanaryRolloutState::Promoted {
            0
        } else if record.state == CanaryRolloutState::Active {
            100 - record.candidate_weight
        } else {
            0
        },
        candidate_weight: record.candidate_weight,
        created_at: record.created_at,
        updated_at: record.updated_at,
        integrity_valid,
        drift,
        can_change_traffic: progress_ready && record.state == CanaryRolloutState::Active,
        can_promote: progress_ready && record.state == CanaryRolloutState::Active,
        can_abort: overlay_safe && record.state == CanaryRolloutState::Active,
        can_rollback: overlay_safe && record.state == CanaryRolloutState::Promoted,
        stable_health: health_for(&record.stable_instance_id),
        candidate_health: health_for(&record.candidate_instance_id),
        stable_evidence,
        candidate_evidence,
        events: event_views(record),
    }
}

pub fn inspections(
    global: &GlobalConfig,
    running: &HashMap<String, RunningInstance>,
    proxy_running: bool,
    health: &HashMap<String, CanaryTargetHealth>,
) -> Vec<CanaryRolloutInspection> {
    let mut records = global.canary_rollouts.iter().collect::<Vec<_>>();
    records.sort_by_key(|record| std::cmp::Reverse(record.updated_at));
    records
        .into_iter()
        .map(|record| inspect_rollout(record, global, running, proxy_running, health))
        .collect()
}

pub fn active_instance_ids(global: &GlobalConfig) -> HashSet<String> {
    global
        .canary_rollouts
        .iter()
        .filter(|record| record.state.owns_routes())
        .flat_map(|record| {
            [
                record.stable_instance_id.clone(),
                record.candidate_instance_id.clone(),
            ]
        })
        .collect()
}

pub fn active_revision_for_instance<'a>(
    global: &'a GlobalConfig,
    instance_id: &str,
) -> Option<&'a str> {
    global
        .canary_rollouts
        .iter()
        .find(|record| record.state.owns_routes())
        .and_then(|record| {
            if record.stable_instance_id == instance_id {
                Some(record.stable_revision_id.as_str())
            } else if record.candidate_instance_id == instance_id {
                Some(record.candidate_revision_id.as_str())
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::config::default_global_config;
    use crate::deployment_identity::DeploymentIdentity;
    use crate::models::{InstanceConfig, ProxyConfig};

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

    fn setup() -> (GlobalConfig, HashMap<String, RunningInstance>) {
        let mut global = default_global_config();
        global.proxy_config = ProxyConfig {
            enabled: true,
            ..ProxyConfig::default()
        };
        for id in ["stable", "candidate"] {
            global.instances.insert(
                id.into(),
                InstanceConfig {
                    id: id.into(),
                    name: id.into(),
                    ..InstanceConfig::default()
                },
            );
        }
        crate::deployment::ensure_deployments(&mut global).unwrap();
        let mut running = HashMap::new();
        for id in ["stable", "candidate"] {
            let revision =
                crate::deployment::materialize_revision(&mut global, id, &identity(id)).unwrap();
            running.insert(
                id.into(),
                RunningInstance {
                    instance_id: id.into(),
                    pid: 1,
                    port: 8080,
                    host: "127.0.0.1".into(),
                    start_time: 1,
                    executable_path: String::new(),
                    telemetry_session_id: None,
                    workload: "inference".into(),
                    launch_config: global.instances.get(id).cloned(),
                    deployment_identity: identity(id),
                    deployment_id: revision.deployment_id.clone(),
                    deployment_revision_id: revision.id.clone(),
                },
            );
        }
        (global, running)
    }

    #[test]
    fn lifecycle_is_explicit_and_restores_base_routing_without_revision_drift() {
        let (mut global, running) = setup();
        let base_routes = global.proxy_config.routes.clone();
        let before = crate::deployment::validated_current_revision(&global, "stable").unwrap();
        let id = create_rollout(
            &mut global,
            &running,
            true,
            CanaryRolloutCreate {
                stable_instance_id: "stable",
                candidate_instance_id: "candidate",
                model_alias: "public",
                candidate_weight: 10,
            },
            10,
        )
        .unwrap();
        assert_eq!(global.proxy_config.routes, base_routes);
        assert_eq!(global.proxy_config.canary_routes.len(), 2);
        let after = crate::deployment::validated_current_revision(&global, "stable").unwrap();
        assert_eq!(before.id, after.id);
        crate::deployment::validate_runtime_revision(
            &before,
            "stable",
            &global.instances["stable"],
            &running["stable"].deployment_identity,
            &global.proxy_config,
        )
        .unwrap();
        set_weight(&mut global, &running, true, &id, 25, 20).unwrap();
        promote(&mut global, &running, true, true, &id, 30).unwrap();
        rollback(&mut global, &id, 40).unwrap();
        assert!(global.proxy_config.canary_routes.is_empty());
        assert_eq!(global.proxy_config.routes, base_routes);
        assert_eq!(
            global.canary_rollouts[0].state,
            CanaryRolloutState::RolledBack
        );
        assert!(record_observation(
            &mut global,
            &running,
            true,
            &id,
            CanaryRequestEvidence::default(),
            CanaryRequestEvidence::default(),
            50,
        )
        .is_err());
    }

    #[test]
    fn progress_fails_closed_on_revision_or_route_drift_but_abort_remains_safe() {
        let (mut global, running) = setup();
        let id = create_rollout(
            &mut global,
            &running,
            true,
            CanaryRolloutCreate {
                stable_instance_id: "stable",
                candidate_instance_id: "candidate",
                model_alias: "public",
                candidate_weight: 10,
            },
            10,
        )
        .unwrap();
        global.proxy_config.canary_routes[0].weight = 1;
        assert!(set_weight(&mut global, &running, true, &id, 20, 20)
            .unwrap_err()
            .contains("drift"));
        assert!(abort(&mut global, &id, 30).is_err());
        global.proxy_config.canary_routes = global.canary_rollouts[0].expected_routes.clone();
        abort(&mut global, &id, 40).unwrap();
        assert!(global.proxy_config.canary_routes.is_empty());
    }

    #[test]
    fn audit_history_is_bounded_and_integrity_protected() {
        let (mut global, running) = setup();
        let id = create_rollout(
            &mut global,
            &running,
            true,
            CanaryRolloutCreate {
                stable_instance_id: "stable",
                candidate_instance_id: "candidate",
                model_alias: "public",
                candidate_weight: 10,
            },
            10,
        )
        .unwrap();
        for index in 0..(CANARY_AUDIT_LIMIT + 20) {
            record_observation(
                &mut global,
                &running,
                true,
                &id,
                CanaryRequestEvidence {
                    total: index as u64,
                    succeeded: index as u64,
                    ..Default::default()
                },
                CanaryRequestEvidence::default(),
                20 + index as i64,
            )
            .unwrap();
        }
        assert_eq!(global.canary_rollouts[0].events.len(), CANARY_AUDIT_LIMIT);
        validate_record(&global.canary_rollouts[0]).unwrap();
        global.canary_rollouts[0].events[0]
            .summary
            .push_str(" tampered");
        assert!(validate_record(&global.canary_rollouts[0]).is_err());
    }

    #[test]
    fn candidate_share_is_bounded_until_separate_promotion() {
        let (mut global, running) = setup();
        assert!(create_rollout(
            &mut global,
            &running,
            true,
            CanaryRolloutCreate {
                stable_instance_id: "stable",
                candidate_instance_id: "candidate",
                model_alias: "public",
                candidate_weight: 51,
            },
            10
        )
        .is_err());
        let id = create_rollout(
            &mut global,
            &running,
            true,
            CanaryRolloutCreate {
                stable_instance_id: "stable",
                candidate_instance_id: "candidate",
                model_alias: "public",
                candidate_weight: 50,
            },
            10,
        )
        .unwrap();
        assert!(set_weight(&mut global, &running, true, &id, 100, 20).is_err());
        promote(&mut global, &running, true, true, &id, 30).unwrap();
        assert_eq!(global.canary_rollouts[0].candidate_weight, 100);
    }

    #[test]
    fn rollout_history_is_bounded_at_creation_time() {
        let (mut global, running) = setup();
        for index in 0..=CANARY_HISTORY_LIMIT {
            let id = create_rollout(
                &mut global,
                &running,
                true,
                CanaryRolloutCreate {
                    stable_instance_id: "stable",
                    candidate_instance_id: "candidate",
                    model_alias: "public",
                    candidate_weight: 10,
                },
                index as i64 * 2,
            )
            .unwrap();
            abort(&mut global, &id, index as i64 * 2 + 1).unwrap();
        }
        assert_eq!(global.canary_rollouts.len(), CANARY_HISTORY_LIMIT);
        ensure_canary_catalog(&mut global).unwrap();
    }
}
