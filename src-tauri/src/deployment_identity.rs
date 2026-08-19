use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const ARTIFACT_IDENTITY_SCHEMA_VERSION: u8 = 1;
pub const DEPLOYMENT_IDENTITY_SCHEMA_VERSION: u8 = 1;
const SAMPLE_SIZE: u64 = 64 * 1024;
const MAX_SAMPLE_COUNT: usize = 5;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactIdentity {
    #[serde(default)]
    pub schema_version: u8,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub artifact_id: String,
    #[serde(default)]
    pub algorithm: String,
    #[serde(default)]
    pub file_size: u64,
    #[serde(default)]
    pub sample_size: u64,
    #[serde(default)]
    pub sample_count: u8,
}

impl ArtifactIdentity {
    pub fn is_verified(&self) -> bool {
        self.schema_version == ARTIFACT_IDENTITY_SCHEMA_VERSION
            && matches!(self.kind.as_str(), "engine" | "model")
            && self.algorithm == "sha256-sampled-v1"
            && self
                .artifact_id
                .starts_with(&format!("urn:lsm:{}:v1:sha256:", self.kind))
            && self.sample_count > 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentIdentity {
    #[serde(default)]
    pub schema_version: u8,
    #[serde(default)]
    pub deployment_id: String,
    #[serde(default)]
    pub engine_artifact_id: String,
    #[serde(default)]
    pub model_artifact_id: String,
    #[serde(default)]
    pub config_revision_id: String,
    #[serde(default)]
    pub configuration_id: String,
    #[serde(default)]
    pub qualification_evidence_id: String,
}

impl DeploymentIdentity {
    pub fn new(
        engine_artifact_id: String,
        model_artifact_id: String,
        config_revision_id: String,
        configuration_id: String,
        qualification_evidence_id: String,
    ) -> Result<Self, String> {
        if [
            engine_artifact_id.as_str(),
            model_artifact_id.as_str(),
            config_revision_id.as_str(),
            configuration_id.as_str(),
            qualification_evidence_id.as_str(),
        ]
        .iter()
        .any(|value| value.trim().is_empty())
        {
            return Err("deployment identity requires every component identity".to_string());
        }
        let mut identity = Self {
            schema_version: DEPLOYMENT_IDENTITY_SCHEMA_VERSION,
            deployment_id: String::new(),
            engine_artifact_id,
            model_artifact_id,
            config_revision_id,
            configuration_id,
            qualification_evidence_id,
        };
        identity.deployment_id = identity.expected_id()?;
        if !identity.is_valid() {
            return Err(
                "deployment identity contains an unsupported component identity".to_string(),
            );
        }
        Ok(identity)
    }

    pub fn expected_id(&self) -> Result<String, String> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Material<'a> {
            schema_version: u8,
            engine_artifact_id: &'a str,
            model_artifact_id: &'a str,
            config_revision_id: &'a str,
            configuration_id: &'a str,
            qualification_evidence_id: &'a str,
        }
        let bytes = serde_json::to_vec(&Material {
            schema_version: self.schema_version,
            engine_artifact_id: &self.engine_artifact_id,
            model_artifact_id: &self.model_artifact_id,
            config_revision_id: &self.config_revision_id,
            configuration_id: &self.configuration_id,
            qualification_evidence_id: &self.qualification_evidence_id,
        })
        .map_err(|error| format!("failed to serialize deployment identity: {error}"))?;
        Ok(format!(
            "urn:lsm:deployment:v1:sha256:{:x}",
            Sha256::digest(bytes)
        ))
    }

    pub fn is_valid(&self) -> bool {
        self.schema_version == DEPLOYMENT_IDENTITY_SCHEMA_VERSION
            && self
                .engine_artifact_id
                .starts_with("urn:lsm:engine:v1:sha256:")
            && self
                .model_artifact_id
                .starts_with("urn:lsm:model:v1:sha256:")
            && !self.config_revision_id.trim().is_empty()
            && self
                .configuration_id
                .starts_with("urn:lsm:configuration:v1:sha256:")
            && self
                .qualification_evidence_id
                .starts_with("urn:lsm:qualification:v2:sha256:")
            && self
                .expected_id()
                .is_ok_and(|expected| expected == self.deployment_id)
    }
}

fn sample_offsets(file_size: u64) -> Vec<u64> {
    if file_size <= SAMPLE_SIZE {
        return vec![0];
    }
    let last = file_size.saturating_sub(SAMPLE_SIZE);
    let mut offsets = vec![0, last / 4, last / 2, last.saturating_mul(3) / 4, last];
    offsets.sort_unstable();
    offsets.dedup();
    offsets.truncate(MAX_SAMPLE_COUNT);
    offsets
}

pub fn artifact_identity_for_path(kind: &str, path: &Path) -> Result<ArtifactIdentity, String> {
    if !matches!(kind, "engine" | "model") {
        return Err(format!("unsupported artifact identity kind: {kind}"));
    }
    let mut file =
        File::open(path).map_err(|error| format!("failed to open {} artifact: {error}", kind))?;
    let file_size = file
        .metadata()
        .map_err(|error| format!("failed to inspect {} artifact: {error}", kind))?
        .len();
    let offsets = sample_offsets(file_size);
    let mut digest = Sha256::new();
    digest.update(b"llama-server-manager:sampled-artifact:v1\0");
    digest.update(kind.as_bytes());
    digest.update(file_size.to_le_bytes());
    digest.update(SAMPLE_SIZE.to_le_bytes());
    for offset in &offsets {
        file.seek(SeekFrom::Start(*offset))
            .map_err(|error| format!("failed to seek {} artifact: {error}", kind))?;
        let read_len = SAMPLE_SIZE.min(file_size.saturating_sub(*offset)) as usize;
        let mut sample = vec![0_u8; read_len];
        file.read_exact(&mut sample)
            .map_err(|error| format!("failed to sample {} artifact: {error}", kind))?;
        digest.update(offset.to_le_bytes());
        digest.update((read_len as u64).to_le_bytes());
        digest.update(sample);
    }
    Ok(ArtifactIdentity {
        schema_version: ARTIFACT_IDENTITY_SCHEMA_VERSION,
        kind: kind.to_string(),
        artifact_id: format!("urn:lsm:{kind}:v1:sha256:{:x}", digest.finalize()),
        algorithm: "sha256-sampled-v1".to_string(),
        file_size,
        sample_size: SAMPLE_SIZE,
        sample_count: offsets.len() as u8,
    })
}

pub fn qualification_evidence_id(
    report: &crate::models::EngineQualificationReport,
) -> Result<String, String> {
    let mut material = report.clone();
    material.evidence_id.clear();
    let bytes = serde_json::to_vec(&material)
        .map_err(|error| format!("failed to serialize qualification evidence: {error}"))?;
    Ok(format!(
        "urn:lsm:qualification:v2:sha256:{:x}",
        Sha256::digest(bytes)
    ))
}

pub fn seal_qualification_report(
    report: &mut crate::models::EngineQualificationReport,
) -> Result<(), String> {
    report.schema_version = 2;
    report.evidence_id.clear();
    report.evidence_id = qualification_evidence_id(report)?;
    Ok(())
}

pub fn qualification_evidence_valid(report: &crate::models::EngineQualificationReport) -> bool {
    report.schema_version == 2
        && !report.engine_artifact_id.is_empty()
        && !report.model_artifact_id.is_empty()
        && qualification_evidence_id(report).is_ok_and(|expected| expected == report.evidence_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn sampled_identity_survives_move_and_changes_with_sampled_content() {
        let dir = std::env::temp_dir().join(format!("lsm-identity-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let original = dir.join("model.gguf");
        let moved = dir.join("renamed.gguf");
        let mut bytes = vec![7_u8; (SAMPLE_SIZE * 6) as usize];
        fs::write(&original, &bytes).unwrap();
        let before = artifact_identity_for_path("model", &original).unwrap();
        fs::rename(&original, &moved).unwrap();
        let after_move = artifact_identity_for_path("model", &moved).unwrap();
        assert_eq!(before, after_move);
        bytes[0] = 8;
        fs::write(&moved, &bytes).unwrap();
        let after_change = artifact_identity_for_path("model", &moved).unwrap();
        assert_ne!(before.artifact_id, after_change.artifact_id);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deployment_id_is_deterministic_and_tamper_evident() {
        let identity = DeploymentIdentity::new(
            "urn:lsm:engine:v1:sha256:engine".into(),
            "urn:lsm:model:v1:sha256:model".into(),
            "revision".into(),
            "urn:lsm:configuration:v1:sha256:configuration".into(),
            "urn:lsm:qualification:v2:sha256:qualification".into(),
        )
        .unwrap();
        assert!(identity.is_valid());
        let same = DeploymentIdentity::new(
            "urn:lsm:engine:v1:sha256:engine".into(),
            "urn:lsm:model:v1:sha256:model".into(),
            "revision".into(),
            "urn:lsm:configuration:v1:sha256:configuration".into(),
            "urn:lsm:qualification:v2:sha256:qualification".into(),
        )
        .unwrap();
        assert_eq!(identity.deployment_id, same.deployment_id);
        let mut tampered = identity;
        tampered.model_artifact_id = "other-model".into();
        assert!(!tampered.is_valid());
    }
}
