use crate::deployment_identity::DeploymentIdentity;
use crate::models::{GlobalConfig, InstanceConfig, ProxyConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEPLOYMENT_SCHEMA_VERSION: u32 = 1;
pub const DEPLOYMENT_REVISION_SCHEMA_VERSION: u8 = 1;
const DEPLOYMENT_HISTORY_LIMIT: usize = 32;

pub const fn default_deployment_schema_version() -> u32 {
    DEPLOYMENT_SCHEMA_VERSION
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRuntimePolicy {
    pub auto_start: bool,
    pub restart_policy: String,
}

impl DeploymentRuntimePolicy {
    fn from_config(config: &InstanceConfig) -> Self {
        Self {
            auto_start: config.auto_start,
            restart_policy: if config
                .restart_policy
                .trim()
                .eq_ignore_ascii_case("on-failure")
            {
                "on-failure".into()
            } else {
                "never".into()
            },
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRouteSnapshot {
    pub id: String,
    pub enabled: bool,
    pub model_alias: String,
    pub priority: i32,
    pub weight: u32,
    pub max_concurrent_requests: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRoutingSnapshot {
    pub proxy_enabled: bool,
    pub default_target: bool,
    pub routing_strategy: String,
    pub routes: Vec<DeploymentRouteSnapshot>,
}

impl DeploymentRoutingSnapshot {
    pub fn from_proxy(instance_id: &str, proxy: &ProxyConfig) -> Self {
        let mut routes = proxy
            .routes
            .iter()
            .filter(|route| route.target_instance_id == instance_id)
            .map(|route| DeploymentRouteSnapshot {
                id: route.id.trim().to_string(),
                enabled: route.enabled,
                model_alias: route.model_alias.trim().to_string(),
                priority: route.priority,
                weight: route.weight,
                max_concurrent_requests: route.max_concurrent_requests,
            })
            .collect::<Vec<_>>();
        routes.sort();
        Self {
            proxy_enabled: proxy.enabled,
            default_target: proxy.default_instance_id == instance_id,
            routing_strategy: proxy.routing_strategy.trim().to_string(),
            routes,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRevision {
    #[serde(default)]
    pub schema_version: u8,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub deployment_id: String,
    #[serde(default)]
    pub deployment_identity: DeploymentIdentity,
    #[serde(default)]
    pub runtime_policy: DeploymentRuntimePolicy,
    #[serde(default)]
    pub routing: DeploymentRoutingSnapshot,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    integrity: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRecord {
    #[serde(default)]
    pub schema_version: u8,
    #[serde(default)]
    pub deployment_id: String,
    #[serde(default)]
    pub instance_id: String,
    #[serde(default)]
    pub current_revision_id: Option<String>,
    #[serde(default)]
    pub rollback_target_revision_id: Option<String>,
    #[serde(default)]
    pub revisions: Vec<DeploymentRevision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentState {
    Unmaterialized,
    Ready,
    Stale,
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRevisionSummary {
    pub id: String,
    pub deployment_identity: DeploymentIdentity,
    pub runtime_policy: DeploymentRuntimePolicy,
    pub routing: DeploymentRoutingSnapshot,
    pub created_at: u64,
    pub current: bool,
    pub rollback_target: bool,
    pub integrity_valid: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentInspection {
    pub instance_id: String,
    pub deployment_id: String,
    pub state: DeploymentState,
    pub message: Option<String>,
    pub current_revision_id: Option<String>,
    pub rollback_target_revision_id: Option<String>,
    pub running_revision_id: Option<String>,
    pub revisions: Vec<DeploymentRevisionSummary>,
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn digest_id(kind: &str, material: &impl Serialize) -> Result<String, String> {
    let bytes = serde_json::to_vec(material)
        .map_err(|error| format!("failed to serialize {kind} identity: {error}"))?;
    Ok(format!(
        "urn:lsm:{kind}:v1:sha256:{:x}",
        Sha256::digest(bytes)
    ))
}

pub fn deployment_id_for_instance(instance_id: &str) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Material<'a> {
        schema_version: u8,
        instance_id: &'a str,
    }
    if instance_id.trim().is_empty() {
        return Err("deployment requires a non-empty instance identity".into());
    }
    digest_id(
        "managed-deployment",
        &Material {
            schema_version: DEPLOYMENT_REVISION_SCHEMA_VERSION,
            instance_id,
        },
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionMaterial<'a> {
    schema_version: u8,
    deployment_id: &'a str,
    deployment_identity: &'a DeploymentIdentity,
    runtime_policy: &'a DeploymentRuntimePolicy,
    routing: &'a DeploymentRoutingSnapshot,
}

fn expected_revision_id(revision: &DeploymentRevision) -> Result<String, String> {
    digest_id(
        "deployment-revision",
        &RevisionMaterial {
            schema_version: revision.schema_version,
            deployment_id: &revision.deployment_id,
            deployment_identity: &revision.deployment_identity,
            runtime_policy: &revision.runtime_policy,
            routing: &revision.routing,
        },
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionIntegrityMaterial<'a> {
    id: &'a str,
    created_at: u64,
    material: RevisionMaterial<'a>,
}

fn expected_revision_integrity(revision: &DeploymentRevision) -> Result<String, String> {
    let material = RevisionIntegrityMaterial {
        id: &revision.id,
        created_at: revision.created_at,
        material: RevisionMaterial {
            schema_version: revision.schema_version,
            deployment_id: &revision.deployment_id,
            deployment_identity: &revision.deployment_identity,
            runtime_policy: &revision.runtime_policy,
            routing: &revision.routing,
        },
    };
    let bytes = serde_json::to_vec(&material)
        .map_err(|error| format!("failed to serialize deployment revision integrity: {error}"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn revision_integrity_valid(revision: &DeploymentRevision) -> bool {
    revision.schema_version == DEPLOYMENT_REVISION_SCHEMA_VERSION
        && revision.deployment_identity.is_valid()
        && expected_revision_id(revision).is_ok_and(|expected| expected == revision.id)
        && expected_revision_integrity(revision)
            .is_ok_and(|expected| expected == revision.integrity)
}

fn new_record(instance_id: &str) -> Result<DeploymentRecord, String> {
    Ok(DeploymentRecord {
        schema_version: DEPLOYMENT_REVISION_SCHEMA_VERSION,
        deployment_id: deployment_id_for_instance(instance_id)?,
        instance_id: instance_id.to_string(),
        current_revision_id: None,
        rollback_target_revision_id: None,
        revisions: Vec::new(),
    })
}

fn validate_record(instance_id: &str, record: &DeploymentRecord) -> Result<(), String> {
    if record.schema_version != DEPLOYMENT_REVISION_SCHEMA_VERSION
        || record.instance_id != instance_id
        || record.deployment_id != deployment_id_for_instance(instance_id)?
    {
        return Err(format!(
            "deployment catalog identity is invalid for instance {instance_id}"
        ));
    }
    let mut ids = BTreeSet::new();
    for revision in &record.revisions {
        if revision.deployment_id != record.deployment_id || !revision_integrity_valid(revision) {
            return Err(format!(
                "deployment catalog contains an invalid revision for instance {instance_id}"
            ));
        }
        if !ids.insert(revision.id.as_str()) {
            return Err(format!(
                "deployment catalog contains duplicate revisions for instance {instance_id}"
            ));
        }
    }
    for (label, pointer) in [
        ("current", &record.current_revision_id),
        ("rollback", &record.rollback_target_revision_id),
    ] {
        if pointer
            .as_ref()
            .is_some_and(|id| !record.revisions.iter().any(|revision| &revision.id == id))
        {
            return Err(format!(
                "deployment catalog {label} pointer is invalid for instance {instance_id}"
            ));
        }
    }
    Ok(())
}

/// Returns the integrity-checked current revision for an instance. Operational
/// workflows use this binding instead of reaching into the deployment catalog
/// or accepting a merely matching identifier from the runtime process.
pub fn validated_current_revision(
    global: &GlobalConfig,
    instance_id: &str,
) -> Result<DeploymentRevision, String> {
    let record = global
        .deployments
        .get(instance_id)
        .ok_or_else(|| format!("deployment record {instance_id} does not exist"))?;
    validate_record(instance_id, record)?;
    let revision_id = record
        .current_revision_id
        .as_deref()
        .ok_or_else(|| format!("deployment {instance_id} has not been materialized"))?;
    record
        .revisions
        .iter()
        .find(|revision| revision.id == revision_id)
        .cloned()
        .ok_or_else(|| format!("deployment {instance_id} current revision is missing"))
}

pub fn ensure_deployments(global: &mut GlobalConfig) -> Result<bool, String> {
    if global.deployment_schema_version > DEPLOYMENT_SCHEMA_VERSION {
        return Err(format!(
            "deployment schema {} is newer than supported schema {}",
            global.deployment_schema_version, DEPLOYMENT_SCHEMA_VERSION
        ));
    }
    let mut changed = global.deployment_schema_version < DEPLOYMENT_SCHEMA_VERSION;
    global.deployment_schema_version = DEPLOYMENT_SCHEMA_VERSION;

    let stale = global
        .deployments
        .keys()
        .filter(|instance_id| !global.instances.contains_key(*instance_id))
        .cloned()
        .collect::<Vec<_>>();
    for instance_id in stale {
        global.deployments.remove(&instance_id);
        changed = true;
    }
    for instance_id in global.instances.keys() {
        if !global.deployments.contains_key(instance_id) {
            global
                .deployments
                .insert(instance_id.clone(), new_record(instance_id)?);
            changed = true;
        }
    }
    for record in global.deployments.values() {
        if record.schema_version > DEPLOYMENT_REVISION_SCHEMA_VERSION {
            return Err(format!(
                "deployment record schema {} is newer than supported schema {}",
                record.schema_version, DEPLOYMENT_REVISION_SCHEMA_VERSION
            ));
        }
    }
    Ok(changed)
}

fn build_revision(
    deployment_id: &str,
    instance_id: &str,
    identity: &DeploymentIdentity,
    config: &InstanceConfig,
    proxy: &ProxyConfig,
    created_at: u64,
) -> Result<DeploymentRevision, String> {
    if !identity.is_valid() {
        return Err("deployment revision requires a valid deployment identity".into());
    }
    let mut revision = DeploymentRevision {
        schema_version: DEPLOYMENT_REVISION_SCHEMA_VERSION,
        id: String::new(),
        deployment_id: deployment_id.to_string(),
        deployment_identity: identity.clone(),
        runtime_policy: DeploymentRuntimePolicy::from_config(config),
        routing: DeploymentRoutingSnapshot::from_proxy(instance_id, proxy),
        created_at,
        integrity: String::new(),
    };
    revision.id = expected_revision_id(&revision)?;
    revision.integrity = expected_revision_integrity(&revision)?;
    Ok(revision)
}

fn prune_history(record: &mut DeploymentRecord) {
    while record.revisions.len() > DEPLOYMENT_HISTORY_LIMIT {
        let protected = [
            record.current_revision_id.as_deref(),
            record.rollback_target_revision_id.as_deref(),
        ];
        let Some(index) = record
            .revisions
            .iter()
            .position(|revision| !protected.contains(&Some(revision.id.as_str())))
        else {
            break;
        };
        record.revisions.remove(index);
    }
}

pub fn materialize_revision(
    global: &mut GlobalConfig,
    instance_id: &str,
    identity: &DeploymentIdentity,
) -> Result<DeploymentRevision, String> {
    ensure_deployments(global)?;
    let config = global
        .instances
        .get(instance_id)
        .cloned()
        .ok_or_else(|| format!("deployment instance {instance_id} does not exist"))?;
    let proxy = global.proxy_config.clone();
    let record = global
        .deployments
        .get_mut(instance_id)
        .ok_or_else(|| format!("deployment record {instance_id} does not exist"))?;
    validate_record(instance_id, record)?;
    let candidate = build_revision(
        &record.deployment_id,
        instance_id,
        identity,
        &config,
        &proxy,
        now_epoch_seconds(),
    )?;
    if record.current_revision_id.as_deref() == Some(candidate.id.as_str()) {
        return record
            .revisions
            .iter()
            .find(|revision| revision.id == candidate.id)
            .cloned()
            .ok_or_else(|| "deployment current revision is missing".to_string());
    }
    let previous = record.current_revision_id.clone();
    let revision = match record
        .revisions
        .iter()
        .find(|revision| revision.id == candidate.id)
        .cloned()
    {
        Some(existing) => existing,
        None => {
            record.revisions.push(candidate.clone());
            candidate
        }
    };
    record.current_revision_id = Some(revision.id.clone());
    record.rollback_target_revision_id = previous.filter(|id| id != &revision.id);
    prune_history(record);
    validate_record(instance_id, record)?;
    Ok(revision)
}

pub fn validate_runtime_revision(
    revision: &DeploymentRevision,
    instance_id: &str,
    config: &InstanceConfig,
    identity: &DeploymentIdentity,
    proxy: &ProxyConfig,
) -> Result<(), String> {
    if !revision_integrity_valid(revision)
        || revision.deployment_id != deployment_id_for_instance(instance_id)?
    {
        return Err("DEPLOYMENT_REVISION_INVALID: runtime deployment revision is invalid".into());
    }
    if &revision.deployment_identity != identity {
        return Err(
            "DEPLOYMENT_REVISION_IDENTITY_STALE: runtime deployment identity changed".into(),
        );
    }
    if revision.runtime_policy != DeploymentRuntimePolicy::from_config(config) {
        return Err("DEPLOYMENT_REVISION_POLICY_STALE: runtime recovery policy changed".into());
    }
    if revision.routing != DeploymentRoutingSnapshot::from_proxy(instance_id, proxy) {
        return Err("DEPLOYMENT_REVISION_ROUTING_STALE: runtime routing state changed".into());
    }
    Ok(())
}

pub fn routing_changed_instance_ids(
    current: &ProxyConfig,
    next: &ProxyConfig,
    instances: impl Iterator<Item = String>,
) -> Vec<String> {
    instances
        .filter(|instance_id| {
            DeploymentRoutingSnapshot::from_proxy(instance_id, current)
                != DeploymentRoutingSnapshot::from_proxy(instance_id, next)
        })
        .collect()
}

pub fn inspect(
    global: &mut GlobalConfig,
    instance_id: &str,
    expected_identity: Option<&DeploymentIdentity>,
    preflight_error: Option<String>,
    running_revision_id: Option<String>,
) -> DeploymentInspection {
    let default_id = deployment_id_for_instance(instance_id).unwrap_or_default();
    let migration_error = ensure_deployments(global).err();
    let Some(record) = global.deployments.get(instance_id) else {
        return DeploymentInspection {
            instance_id: instance_id.into(),
            deployment_id: default_id,
            state: DeploymentState::Invalid,
            message: migration_error.or_else(|| Some("deployment record is missing".into())),
            current_revision_id: None,
            rollback_target_revision_id: None,
            running_revision_id,
            revisions: Vec::new(),
        };
    };
    let validation_error = migration_error.or_else(|| validate_record(instance_id, record).err());
    let revisions = record
        .revisions
        .iter()
        .rev()
        .map(|revision| DeploymentRevisionSummary {
            id: revision.id.clone(),
            deployment_identity: revision.deployment_identity.clone(),
            runtime_policy: revision.runtime_policy.clone(),
            routing: revision.routing.clone(),
            created_at: revision.created_at,
            current: record.current_revision_id.as_deref() == Some(revision.id.as_str()),
            rollback_target: record.rollback_target_revision_id.as_deref()
                == Some(revision.id.as_str()),
            integrity_valid: revision_integrity_valid(revision),
        })
        .collect::<Vec<_>>();
    let (state, message) = if let Some(error) = validation_error {
        (DeploymentState::Invalid, Some(error))
    } else if record.current_revision_id.is_none() {
        (
            DeploymentState::Unmaterialized,
            preflight_error.or_else(|| {
                Some("deployment will be materialized on the next qualified start".into())
            }),
        )
    } else if let Some(error) = preflight_error {
        (DeploymentState::Stale, Some(error))
    } else if let (Some(identity), Some(config)) =
        (expected_identity, global.instances.get(instance_id))
    {
        let expected = build_revision(
            &record.deployment_id,
            instance_id,
            identity,
            config,
            &global.proxy_config,
            0,
        );
        let current_matches = expected.is_ok_and(|revision| {
            record.current_revision_id.as_deref() == Some(revision.id.as_str())
        });
        let running_matches = running_revision_id
            .as_deref()
            .map_or(true, |id| record.current_revision_id.as_deref() == Some(id));
        if current_matches && running_matches {
            (DeploymentState::Ready, None)
        } else {
            (
                DeploymentState::Stale,
                Some(
                    "deployment inputs changed; start the instance to materialize a new revision"
                        .into(),
                ),
            )
        }
    } else {
        (
            DeploymentState::Stale,
            Some("deployment identity is not currently available".into()),
        )
    };
    DeploymentInspection {
        instance_id: instance_id.into(),
        deployment_id: record.deployment_id.clone(),
        state,
        message,
        current_revision_id: record.current_revision_id.clone(),
        rollback_target_revision_id: record.rollback_target_revision_id.clone(),
        running_revision_id,
        revisions,
    }
}

#[cfg(test)]
pub(crate) fn test_revision(
    instance_id: &str,
    config: &InstanceConfig,
    identity: &DeploymentIdentity,
    proxy: &ProxyConfig,
) -> DeploymentRevision {
    build_revision(
        &deployment_id_for_instance(instance_id).unwrap(),
        instance_id,
        identity,
        config,
        proxy,
        1,
    )
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::config::default_global_config;
    use crate::models::ProxyRoute;

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

    fn global() -> GlobalConfig {
        let mut global = default_global_config();
        let config = InstanceConfig {
            id: "one".into(),
            name: "One".into(),
            ..InstanceConfig::default()
        };
        global.instances.insert("one".into(), config);
        global
    }

    #[test]
    fn migration_creates_an_unmaterialized_stable_deployment() {
        let mut global = global();
        global.deployment_schema_version = 0;
        assert!(ensure_deployments(&mut global).unwrap());
        let record = &global.deployments["one"];
        assert_eq!(
            record.deployment_id,
            deployment_id_for_instance("one").unwrap()
        );
        assert!(record.current_revision_id.is_none());
        assert!(!ensure_deployments(&mut global).unwrap());
    }

    #[test]
    fn materialization_is_deterministic_and_sets_an_explicit_rollback_target() {
        let mut global = global();
        let first = materialize_revision(&mut global, "one", &identity("a")).unwrap();
        let same = materialize_revision(&mut global, "one", &identity("a")).unwrap();
        assert_eq!(first.id, same.id);
        assert_eq!(global.deployments["one"].revisions.len(), 1);

        let second = materialize_revision(&mut global, "one", &identity("b")).unwrap();
        let record = &global.deployments["one"];
        assert_eq!(
            record.current_revision_id.as_deref(),
            Some(second.id.as_str())
        );
        assert_eq!(
            record.rollback_target_revision_id.as_deref(),
            Some(first.id.as_str())
        );
    }

    #[test]
    fn history_is_bounded_without_losing_current_or_rollback_pointers() {
        let mut global = global();
        for index in 0..40 {
            materialize_revision(&mut global, "one", &identity(&index.to_string())).unwrap();
        }
        let record = &global.deployments["one"];
        assert_eq!(record.revisions.len(), DEPLOYMENT_HISTORY_LIMIT);
        assert!(record.revisions.iter().any(|revision| {
            record.current_revision_id.as_deref() == Some(revision.id.as_str())
        }));
        assert!(record.revisions.iter().any(|revision| {
            record.rollback_target_revision_id.as_deref() == Some(revision.id.as_str())
        }));
    }

    #[test]
    fn routing_snapshot_is_ordered_and_excludes_unrelated_routes() {
        let mut global = global();
        global.proxy_config.enabled = true;
        global.proxy_config.routes = vec![
            ProxyRoute {
                id: "z".into(),
                target_instance_id: "one".into(),
                model_alias: " beta ".into(),
                ..ProxyRoute::default()
            },
            ProxyRoute {
                id: "a".into(),
                target_instance_id: "other".into(),
                ..ProxyRoute::default()
            },
            ProxyRoute {
                id: "b".into(),
                target_instance_id: "one".into(),
                model_alias: "alpha".into(),
                ..ProxyRoute::default()
            },
        ];
        let revision = materialize_revision(&mut global, "one", &identity("a")).unwrap();
        assert_eq!(
            revision
                .routing
                .routes
                .iter()
                .map(|route| route.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "z"]
        );
    }

    #[test]
    fn tampering_and_future_schemas_fail_closed() {
        let mut tampered = global();
        materialize_revision(&mut tampered, "one", &identity("a")).unwrap();
        tampered.deployments.get_mut("one").unwrap().revisions[0]
            .runtime_policy
            .auto_start = true;
        assert!(ensure_deployments(&mut tampered).is_ok());
        assert!(materialize_revision(&mut tampered, "one", &identity("b")).is_err());
        assert_eq!(
            inspect(&mut tampered, "one", Some(&identity("a")), None, None).state,
            DeploymentState::Invalid
        );

        let mut future = global();
        future.deployment_schema_version = DEPLOYMENT_SCHEMA_VERSION + 1;
        assert!(ensure_deployments(&mut future).is_err());
    }

    #[test]
    fn runtime_validation_rejects_policy_and_routing_drift() {
        let mut global = global();
        let revision = materialize_revision(&mut global, "one", &identity("a")).unwrap();
        let config = global.instances["one"].clone();
        validate_runtime_revision(
            &revision,
            "one",
            &config,
            &identity("a"),
            &global.proxy_config,
        )
        .unwrap();

        let mut changed = config;
        changed.restart_policy = "on-failure".into();
        assert!(validate_runtime_revision(
            &revision,
            "one",
            &changed,
            &identity("a"),
            &global.proxy_config,
        )
        .unwrap_err()
        .starts_with("DEPLOYMENT_REVISION_POLICY_STALE"));
    }

    #[test]
    fn inspection_distinguishes_unmaterialized_ready_stale_and_invalid() {
        let mut global = global();
        let expected = identity("a");
        assert_eq!(
            inspect(&mut global, "one", Some(&expected), None, None).state,
            DeploymentState::Unmaterialized
        );
        let revision = materialize_revision(&mut global, "one", &expected).unwrap();
        assert_eq!(
            inspect(&mut global, "one", Some(&expected), None, None).state,
            DeploymentState::Ready
        );
        assert_eq!(
            inspect(
                &mut global,
                "one",
                Some(&expected),
                None,
                Some(String::new()),
            )
            .state,
            DeploymentState::Stale
        );
        assert_eq!(
            inspect(
                &mut global,
                "one",
                Some(&expected),
                None,
                Some(revision.id.clone()),
            )
            .state,
            DeploymentState::Ready
        );

        global.proxy_config.default_instance_id = "one".into();
        assert_eq!(
            inspect(&mut global, "one", Some(&expected), None, None).state,
            DeploymentState::Stale
        );
        global.deployments.get_mut("one").unwrap().revisions[0]
            .deployment_identity
            .model_artifact_id = "tampered".into();
        assert_eq!(
            inspect(&mut global, "one", Some(&expected), None, None).state,
            DeploymentState::Invalid
        );
    }

    #[test]
    fn unrelated_route_edits_do_not_invalidate_another_deployment() {
        let current = ProxyConfig {
            routes: vec![ProxyRoute {
                id: "other".into(),
                target_instance_id: "two".into(),
                model_alias: "before".into(),
                ..ProxyRoute::default()
            }],
            ..ProxyConfig::default()
        };
        let mut next = current.clone();
        next.routes[0].model_alias = "after".into();
        assert_eq!(
            routing_changed_instance_ids(&current, &next, ["one".into(), "two".into()].into_iter()),
            vec!["two"]
        );
    }
}
