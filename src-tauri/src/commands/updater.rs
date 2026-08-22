use crate::{bounded_http, error::AppResult};
use minisign_verify::{PublicKey, Signature};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

const UPDATER_MANIFEST_URL: &str = "https://updates.cnzone.net/latest.json";
const UPDATER_PUBLIC_KEY: &str = "RWToKvTedmdiGsiT/Ok2cP+2Uug/xmHR6TvAptCnotadoNh1qYBaBD4C";
const MANIFEST_LIMIT_BYTES: usize = 256 * 1024;
const SIGNATURE_LIMIT_BYTES: usize = 8 * 1024;
const UPDATER_HTTP_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
const UPDATER_HTTP_TOTAL_TIMEOUT: Duration = Duration::from_secs(15);

static UPDATER_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .redirect(Policy::none())
        .build()
        .expect("build updater manifest HTTP client")
});

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SignedReleaseManifest {
    version: String,
    notes: String,
    pub_date: String,
    release_tag: String,
    source_sha: String,
    release_counter: u64,
    platforms: HashMap<String, SignedReleasePlatform>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedReleaseDocument {
    version: String,
    notes: String,
    pub_date: String,
    release_tag: String,
    source_sha: String,
    release_counter: u64,
    platforms: HashMap<String, SignedReleasePlatform>,
    signed_envelope: String,
    envelope_signature: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedReleasePlatform {
    pub url: String,
    pub signature: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedUpdaterRelease {
    pub version: String,
    pub release_tag: String,
    pub source_sha: String,
    pub release_counter: u64,
    pub target: String,
    pub platform: SignedReleasePlatform,
}

fn validate_hex(value: &str, expected_len: usize, label: &str) -> Result<(), String> {
    if value.len() != expected_len || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("signed updater {label} is invalid"));
    }
    Ok(())
}

fn parse_semver(value: &str) -> Result<(u64, u64, u64), String> {
    let mut parts = value.split('.');
    let parse = |part: Option<&str>| -> Result<u64, String> {
        let part = part.ok_or_else(|| "signed updater version is invalid".to_string())?;
        if part.is_empty()
            || (part.len() > 1 && part.starts_with('0'))
            || !part.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("signed updater version is invalid".to_string());
        }
        part.parse::<u64>()
            .map_err(|_| "signed updater version is invalid".to_string())
    };
    let version = (
        parse(parts.next())?,
        parse(parts.next())?,
        parse(parts.next())?,
    );
    if parts.next().is_some() {
        return Err("signed updater version is invalid".to_string());
    }
    Ok(version)
}

fn release_counter(version: (u64, u64, u64)) -> Result<u64, String> {
    if version.0 > 999_999 || version.1 > 999_999 || version.2 > 999_999 {
        return Err("signed updater version is outside the release-counter contract".to_string());
    }
    version
        .0
        .checked_mul(1_000_000_000_000)
        .and_then(|value| value.checked_add(version.1 * 1_000_000))
        .and_then(|value| value.checked_add(version.2))
        .ok_or_else(|| "signed updater release counter overflowed".to_string())
}

fn validate_manifest(
    manifest: SignedReleaseManifest,
    target: &str,
) -> Result<VerifiedUpdaterRelease, String> {
    let candidate = parse_semver(&manifest.version)?;
    let current = parse_semver(env!("CARGO_PKG_VERSION"))?;
    if candidate <= current {
        return Err("signed updater release is not newer than this application".to_string());
    }
    if manifest.release_tag != format!("v{}", manifest.version) {
        return Err("signed updater tag does not match its version".to_string());
    }
    validate_hex(&manifest.source_sha, 40, "source SHA")?;
    if manifest.release_counter != release_counter(candidate)? || manifest.release_counter == 0 {
        return Err("signed updater release counter does not match its version".to_string());
    }
    if manifest.notes.len() > 128 * 1024 || manifest.pub_date.len() > 64 {
        return Err("signed updater metadata exceeds its limit".to_string());
    }
    if manifest.platforms.len() != 3
        || !manifest.platforms.contains_key("windows-x86_64-nsis")
        || !manifest.platforms.contains_key("windows-x86_64-msi")
        || manifest
            .platforms
            .keys()
            .filter(|name| matches!(name.as_str(), "darwin-aarch64" | "darwin-x86_64"))
            .count()
            != 1
    {
        return Err("signed updater platform set is invalid".to_string());
    }
    let platform = manifest
        .platforms
        .get(target)
        .cloned()
        .ok_or_else(|| "signed updater target is unavailable".to_string())?;
    let url = reqwest::Url::parse(&platform.url)
        .map_err(|_| "signed updater payload URL is invalid".to_string())?;
    if url.scheme() != "https"
        || url.host_str() != Some("updates.cnzone.net")
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url
            .path()
            .starts_with(&format!("/releases/{}/", manifest.release_tag))
    {
        return Err("signed updater payload URL is outside the trusted release origin".to_string());
    }
    if platform.signature.len() > SIGNATURE_LIMIT_BYTES || platform.signature.trim().is_empty() {
        return Err("signed updater payload signature is invalid".to_string());
    }
    validate_hex(&platform.sha256, 64, "payload digest")?;
    Ok(VerifiedUpdaterRelease {
        version: manifest.version,
        release_tag: manifest.release_tag,
        source_sha: manifest.source_sha.to_ascii_lowercase(),
        release_counter: manifest.release_counter,
        target: target.to_string(),
        platform,
    })
}

async fn fetch_bounded(url: &str, limit: usize) -> Result<bytes::Bytes, String> {
    let response = UPDATER_HTTP_CLIENT
        .get(url)
        .send()
        .await
        .map_err(|error| format!("signed updater request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "signed updater endpoint returned HTTP {}",
            response.status()
        ));
    }
    bounded_http::collect_response(
        response,
        limit,
        UPDATER_HTTP_IDLE_TIMEOUT,
        UPDATER_HTTP_TOTAL_TIMEOUT,
    )
    .await
    .map(|(body, _)| body)
}

#[tauri::command]
pub async fn verify_updater_release(target: String) -> AppResult<VerifiedUpdaterRelease> {
    if !matches!(
        target.as_str(),
        "windows-x86_64-nsis" | "windows-x86_64-msi" | "darwin-aarch64" | "darwin-x86_64"
    ) {
        return Err("unsupported updater target".to_string().into());
    }
    let document_bytes = fetch_bounded(UPDATER_MANIFEST_URL, MANIFEST_LIMIT_BYTES).await?;
    let document: SignedReleaseDocument = serde_json::from_slice(&document_bytes)
        .map_err(|error| format!("signed updater document is invalid: {error}"))?;
    if document.signed_envelope.len() > MANIFEST_LIMIT_BYTES
        || document.envelope_signature.len() > SIGNATURE_LIMIT_BYTES
    {
        return Err("signed updater envelope exceeds its limit"
            .to_string()
            .into());
    }
    let public_key = PublicKey::from_base64(UPDATER_PUBLIC_KEY)
        .map_err(|error| format!("updater public key is invalid: {error}"))?;
    let signature = Signature::decode(&document.envelope_signature)
        .map_err(|error| format!("updater manifest signature is invalid: {error}"))?;
    public_key
        .verify(document.signed_envelope.as_bytes(), &signature, false)
        .map_err(|error| format!("updater manifest signature verification failed: {error}"))?;
    let manifest: SignedReleaseManifest = serde_json::from_str(&document.signed_envelope)
        .map_err(|error| format!("signed updater manifest is invalid: {error}"))?;
    let projected = SignedReleaseManifest {
        version: document.version,
        notes: document.notes,
        pub_date: document.pub_date,
        release_tag: document.release_tag,
        source_sha: document.source_sha,
        release_counter: document.release_counter,
        platforms: document.platforms,
    };
    if projected != manifest {
        return Err("updater document does not match its signed envelope"
            .to_string()
            .into());
    }
    Ok(validate_manifest(manifest, &target)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str) -> SignedReleaseManifest {
        let platform = SignedReleasePlatform {
            url: format!(
                "https://updates.cnzone.net/releases/v{version}/LlamaServerManager_{version}_windows-x86_64-nsis-setup.exe"
            ),
            signature: "signed payload".to_string(),
            sha256: "a".repeat(64),
        };
        SignedReleaseManifest {
            version: version.to_string(),
            notes: String::new(),
            pub_date: "2026-08-23T00:00:00Z".to_string(),
            release_tag: format!("v{version}"),
            source_sha: "b".repeat(40),
            release_counter: release_counter(parse_semver(version).unwrap()).unwrap(),
            platforms: HashMap::from([
                ("windows-x86_64-nsis".to_string(), platform.clone()),
                ("windows-x86_64-msi".to_string(), platform.clone()),
                ("darwin-aarch64".to_string(), platform),
            ]),
        }
    }

    #[test]
    fn signed_manifest_binds_newer_version_target_and_payload_identity() {
        let verified = validate_manifest(manifest("999.0.0"), "windows-x86_64-nsis")
            .expect("valid signed release");
        assert_eq!(verified.version, "999.0.0");
        assert_eq!(verified.platform.sha256, "a".repeat(64));
    }

    #[test]
    fn historical_or_relabelled_signed_release_is_rejected() {
        let mut old = manifest(env!("CARGO_PKG_VERSION"));
        assert!(validate_manifest(old.clone(), "windows-x86_64-nsis")
            .unwrap_err()
            .contains("not newer"));
        old.version = "999.0.0".to_string();
        assert!(validate_manifest(old, "windows-x86_64-nsis")
            .unwrap_err()
            .contains("tag does not match"));
    }

    #[test]
    fn signed_manifest_rejects_untrusted_payload_origin_and_bad_digest() {
        let mut invalid = manifest("999.0.0");
        invalid
            .platforms
            .get_mut("windows-x86_64-nsis")
            .unwrap()
            .url = "https://example.test/payload.exe".to_string();
        assert!(validate_manifest(invalid, "windows-x86_64-nsis")
            .unwrap_err()
            .contains("trusted release origin"));
        let mut invalid = manifest("999.0.0");
        invalid
            .platforms
            .get_mut("windows-x86_64-nsis")
            .unwrap()
            .sha256 = "00".to_string();
        assert!(validate_manifest(invalid, "windows-x86_64-nsis")
            .unwrap_err()
            .contains("payload digest"));
    }
}
