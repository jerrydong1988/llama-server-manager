use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
#[cfg(windows)]
use std::io::Write;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Instant, UNIX_EPOCH};

pub const ARTIFACT_IDENTITY_SCHEMA_VERSION: u8 = 1;
pub const DEPLOYMENT_IDENTITY_SCHEMA_VERSION: u8 = 1;
const HASH_BUFFER_SIZE: usize = 1024 * 1024;

static ACTIVE_LAUNCH_ARTIFACTS: LazyLock<Mutex<HashMap<String, LaunchArtifactLeases>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ACTIVE_LAUNCH_PROCESSES: LazyLock<Mutex<HashMap<String, AuthorizedLaunchProcess>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
struct AuthorizedLaunchProcess {
    pid: u32,
    start_time: u64,
    executable_path: PathBuf,
}

pub struct LaunchProcessAuthorization {
    instance_id: String,
    pid: u32,
    armed: bool,
}

impl LaunchProcessAuthorization {
    pub fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for LaunchProcessAuthorization {
    fn drop(&mut self) {
        if self.armed {
            remove_authorized_launch_process(&self.instance_id, self.pid);
        }
    }
}

fn remove_authorized_launch_process(instance_id: &str, expected_pid: u32) {
    let mut processes = ACTIVE_LAUNCH_PROCESSES.lock().unwrap();
    if processes
        .get(instance_id)
        .is_some_and(|process| process.pid == expected_pid)
    {
        processes.remove(instance_id);
    }
}

pub fn register_authorized_launch_process(
    instance_id: &str,
    pid: u32,
    start_time: u64,
    executable_path: &Path,
) -> LaunchProcessAuthorization {
    ACTIVE_LAUNCH_PROCESSES.lock().unwrap().insert(
        instance_id.to_string(),
        AuthorizedLaunchProcess {
            pid,
            start_time,
            executable_path: executable_path.to_path_buf(),
        },
    );
    LaunchProcessAuthorization {
        instance_id: instance_id.to_string(),
        pid,
        armed: true,
    }
}

pub fn is_authorized_launch_process(pid: u32) -> bool {
    let Some((start_time, executable_path)) = crate::commands::server::read_process_identity(pid)
    else {
        return false;
    };
    ACTIVE_LAUNCH_PROCESSES
        .lock()
        .unwrap()
        .values()
        .any(|process| {
            process.pid == pid
                && process.start_time == start_time
                && crate::path_utils::paths_equal(&process.executable_path, &executable_path)
        })
}

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
            && (self.algorithm == "sha256-full-v1"
                || (self.kind == "engine" && self.algorithm == "sha256-engine-bundle-v1"))
            && self
                .artifact_id
                .starts_with(&format!("urn:lsm:{}:v1:sha256:", self.kind))
            && self.sample_size == 0
            && self.sample_count == 0
    }
}

#[derive(Debug)]
struct EngineBundleMemberLease {
    relative_path: String,
    file: File,
    identity: ArtifactIdentity,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuxiliaryArtifactIdentity {
    pub role: String,
    pub artifact_id: String,
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
    pub auxiliary_artifacts: Vec<AuxiliaryArtifactIdentity>,
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
        Self::new_with_auxiliary(
            engine_artifact_id,
            model_artifact_id,
            Vec::new(),
            config_revision_id,
            configuration_id,
            qualification_evidence_id,
        )
    }

    pub fn new_with_auxiliary(
        engine_artifact_id: String,
        model_artifact_id: String,
        mut auxiliary_artifacts: Vec<AuxiliaryArtifactIdentity>,
        config_revision_id: String,
        configuration_id: String,
        qualification_evidence_id: String,
    ) -> Result<Self, String> {
        auxiliary_artifacts.sort_by(|left, right| left.role.cmp(&right.role));
        if auxiliary_artifacts
            .windows(2)
            .any(|pair| pair[0].role == pair[1].role)
            || auxiliary_artifacts.iter().any(|artifact| {
                !matches!(artifact.role.as_str(), "draft_model" | "mmproj")
                    || !artifact.artifact_id.starts_with("urn:lsm:model:v1:sha256:")
            })
        {
            return Err("deployment identity contains invalid auxiliary artifacts".to_string());
        }
        let mut identity = Self {
            schema_version: DEPLOYMENT_IDENTITY_SCHEMA_VERSION,
            deployment_id: String::new(),
            engine_artifact_id,
            model_artifact_id,
            auxiliary_artifacts,
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
            #[serde(skip_serializing_if = "Option::is_none")]
            auxiliary_artifacts: Option<&'a [AuxiliaryArtifactIdentity]>,
            config_revision_id: &'a str,
            configuration_id: &'a str,
            qualification_evidence_id: &'a str,
        }
        let bytes = serde_json::to_vec(&Material {
            schema_version: self.schema_version,
            engine_artifact_id: &self.engine_artifact_id,
            model_artifact_id: &self.model_artifact_id,
            auxiliary_artifacts: (!self.auxiliary_artifacts.is_empty())
                .then_some(self.auxiliary_artifacts.as_slice()),
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
            && self
                .auxiliary_artifacts
                .windows(2)
                .all(|pair| pair[0].role < pair[1].role)
            && self.auxiliary_artifacts.iter().all(|artifact| {
                matches!(artifact.role.as_str(), "draft_model" | "mmproj")
                    && artifact.artifact_id.starts_with("urn:lsm:model:v1:sha256:")
            })
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

fn open_artifact_file(path: &Path, allow_delete: bool) -> Result<File, String> {
    #[cfg(not(windows))]
    let _ = allow_delete;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_DELETE: u32 = 0x0000_0004;
        let share_mode = FILE_SHARE_READ | if allow_delete { FILE_SHARE_DELETE } else { 0 };
        options
            .share_mode(share_mode)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
        .open(path)
        .map_err(|error| format!("failed to open artifact {}: {error}", path.display()))
}

fn artifact_identity_from_open_file_with_deadline(
    kind: &str,
    file: &mut File,
    deadline: Option<Instant>,
) -> Result<ArtifactIdentity, String> {
    if !matches!(kind, "engine" | "model") {
        return Err(format!("unsupported artifact identity kind: {kind}"));
    }
    let metadata_before = file
        .metadata()
        .map_err(|error| format!("failed to inspect {} artifact: {error}", kind))?;
    if !metadata_before.is_file() {
        return Err(format!("{} artifact is not a regular file", kind));
    }
    let file_size = metadata_before.len();
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("failed to rewind {} artifact: {error}", kind))?;
    let mut digest = Sha256::new();
    digest.update(b"llama-server-manager:full-artifact:v1\0");
    digest.update(kind.as_bytes());
    digest.update(file_size.to_le_bytes());
    let mut buffer = vec![0_u8; HASH_BUFFER_SIZE];
    let mut total_read = 0_u64;
    loop {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(format!(
                "{} artifact hashing exceeded the scan work deadline",
                kind
            ));
        }
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to hash {} artifact: {error}", kind))?;
        if count == 0 {
            break;
        }
        total_read = total_read
            .checked_add(count as u64)
            .ok_or_else(|| format!("{} artifact size overflowed while hashing", kind))?;
        if total_read > file_size {
            return Err(format!(
                "{} artifact changed while it was being hashed",
                kind
            ));
        }
        digest.update(&buffer[..count]);
    }
    let metadata_after = file
        .metadata()
        .map_err(|error| format!("failed to re-inspect {} artifact: {error}", kind))?;
    if total_read != file_size || metadata_after.len() != file_size {
        return Err(format!(
            "{} artifact changed while it was being hashed",
            kind
        ));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("failed to restore {} artifact position: {error}", kind))?;
    Ok(ArtifactIdentity {
        schema_version: ARTIFACT_IDENTITY_SCHEMA_VERSION,
        kind: kind.to_string(),
        artifact_id: format!("urn:lsm:{kind}:v1:sha256:{:x}", digest.finalize()),
        algorithm: "sha256-full-v1".to_string(),
        file_size,
        sample_size: 0,
        sample_count: 0,
    })
}

#[cfg(windows)]
fn aggregate_engine_bundle_identity(
    primary: &ArtifactIdentity,
    members: &[EngineBundleMemberLease],
) -> Result<ArtifactIdentity, String> {
    let mut digest = Sha256::new();
    digest.update(b"llama-server-manager:engine-bundle:v1\0");
    digest.update(primary.artifact_id.as_bytes());
    let mut total_size = primary.file_size;
    for member in members {
        total_size = total_size
            .checked_add(member.identity.file_size)
            .ok_or_else(|| "engine bundle size overflow".to_string())?;
        digest.update((member.relative_path.len() as u64).to_le_bytes());
        digest.update(member.relative_path.as_bytes());
        digest.update(member.identity.file_size.to_le_bytes());
        digest.update(member.identity.artifact_id.as_bytes());
    }
    Ok(ArtifactIdentity {
        schema_version: ARTIFACT_IDENTITY_SCHEMA_VERSION,
        kind: "engine".to_string(),
        artifact_id: format!("urn:lsm:engine:v1:sha256:{:x}", digest.finalize()),
        algorithm: "sha256-engine-bundle-v1".to_string(),
        file_size: total_size,
        sample_size: 0,
        sample_count: 0,
    })
}

#[cfg(windows)]
fn engine_identity_and_bundle_leases(
    canonical_path: &Path,
    primary_file: &mut File,
    deadline: Option<Instant>,
    validate_owner: bool,
) -> Result<(ArtifactIdentity, Vec<EngineBundleMemberLease>), String> {
    const MAX_BUNDLE_MEMBERS: usize = 512;
    const MAX_BUNDLE_DEPTH: usize = 8;
    const MAX_BUNDLE_BYTES: u64 = 16 * 1024 * 1024 * 1024;

    let primary = artifact_identity_from_open_file_with_deadline("engine", primary_file, deadline)?;
    let root = canonical_path
        .parent()
        .ok_or_else(|| "engine executable has no parent directory".to_string())?;
    let primary_name = canonical_path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut candidates = Vec::new();
    let mut directories = vec![(root.to_path_buf(), 0_usize)];
    while let Some((directory, depth)) = directories.pop() {
        let entries = std::fs::read_dir(&directory).map_err(|error| {
            format!(
                "failed to enumerate engine bundle {}: {error}",
                directory.display()
            )
        })?;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("failed to enumerate engine bundle: {error}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("failed to inspect engine bundle member: {error}"))?;
            if file_type.is_symlink() {
                return Err(format!(
                    "engine bundle cannot contain links or reparse points: {}",
                    path.display()
                ));
            }
            if file_type.is_dir() {
                if depth >= MAX_BUNDLE_DEPTH {
                    return Err(format!(
                        "engine bundle exceeds {MAX_BUNDLE_DEPTH} directory levels"
                    ));
                }
                directories.push((path, depth + 1));
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let extension_is_dll = path
                .extension()
                .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("dll"));
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if extension_is_dll && name != primary_name {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "engine bundle member escaped its root".to_string())?
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_ascii_lowercase();
                candidates.push((relative, path));
                if candidates.len() > MAX_BUNDLE_MEMBERS {
                    return Err(format!(
                        "engine bundle exceeds {MAX_BUNDLE_MEMBERS} DLL members"
                    ));
                }
            }
        }
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    if candidates.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err("engine bundle contains duplicate case-insensitive DLL paths".to_string());
    }

    let mut members = Vec::with_capacity(candidates.len());
    let mut total_size = primary.file_size;
    for (relative_path, path) in candidates {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err("engine bundle hashing exceeded the scan work deadline".to_string());
        }
        if validate_owner {
            validate_owner_protected_tree(&path, root)?;
        }
        let mut file = open_artifact_file(&path, false)?;
        FileExt::try_lock_shared(&file)
            .map_err(|error| format!("engine bundle member is being modified: {error}"))?;
        let identity =
            artifact_identity_from_open_file_with_deadline("engine", &mut file, deadline)?;
        total_size = total_size
            .checked_add(identity.file_size)
            .ok_or_else(|| "engine bundle size overflow".to_string())?;
        if total_size > MAX_BUNDLE_BYTES {
            return Err(format!(
                "engine bundle exceeds {MAX_BUNDLE_BYTES} retained bytes"
            ));
        }
        members.push(EngineBundleMemberLease {
            relative_path,
            file,
            identity,
        });
    }
    let identity = aggregate_engine_bundle_identity(&primary, &members)?;
    Ok((identity, members))
}

#[cfg(not(windows))]
fn engine_identity_and_bundle_leases(
    _canonical_path: &Path,
    primary_file: &mut File,
    deadline: Option<Instant>,
    _validate_owner: bool,
) -> Result<(ArtifactIdentity, Vec<EngineBundleMemberLease>), String> {
    Ok((
        artifact_identity_from_open_file_with_deadline("engine", primary_file, deadline)?,
        Vec::new(),
    ))
}

fn identity_and_bundle_leases(
    kind: &str,
    canonical_path: &Path,
    file: &mut File,
    deadline: Option<Instant>,
) -> Result<(ArtifactIdentity, Vec<EngineBundleMemberLease>), String> {
    identity_and_bundle_leases_with_owner_validation(kind, canonical_path, file, deadline, true)
}

fn identity_and_bundle_leases_with_owner_validation(
    kind: &str,
    canonical_path: &Path,
    file: &mut File,
    deadline: Option<Instant>,
    validate_owner: bool,
) -> Result<(ArtifactIdentity, Vec<EngineBundleMemberLease>), String> {
    if kind == "engine" {
        engine_identity_and_bundle_leases(canonical_path, file, deadline, validate_owner)
    } else {
        Ok((
            artifact_identity_from_open_file_with_deadline(kind, file, deadline)?,
            Vec::new(),
        ))
    }
}

fn modified_nanos(file: &File) -> u128 {
    file.metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn engine_fingerprint_material(
    canonical_path: &Path,
    modified: u128,
    identity: &ArtifactIdentity,
) -> String {
    let normalized_path = crate::path_utils::path_identity_key(canonical_path);
    let mut digest = Sha256::new();
    digest.update(b"llama-server-manager:engine-fingerprint:v3\0");
    digest.update(normalized_path.as_bytes());
    digest.update(identity.file_size.to_le_bytes());
    digest.update(modified.to_le_bytes());
    digest.update(identity.artifact_id.as_bytes());
    format!(
        "v3:{normalized_path}:{}:{modified}:{:x}",
        identity.file_size,
        digest.finalize()
    )
}

pub fn engine_fingerprint_for_path(path: &Path) -> String {
    let canonical_path = match std::fs::canonicalize(path) {
        Ok(path) => path,
        Err(_) => return String::new(),
    };
    let mut file = match open_artifact_file(&canonical_path, false) {
        Ok(file) => file,
        Err(_) => return String::new(),
    };
    let modified = modified_nanos(&file);
    let identity = match identity_and_bundle_leases_with_owner_validation(
        "engine",
        &canonical_path,
        &mut file,
        None,
        false,
    ) {
        Ok((identity, _)) => identity,
        Err(_) => return String::new(),
    };
    engine_fingerprint_material(&canonical_path, modified, &identity)
}

pub fn artifact_identity_for_path(kind: &str, path: &Path) -> Result<ArtifactIdentity, String> {
    let canonical_path = std::fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve {} artifact: {error}", kind))?;
    let mut file = open_artifact_file(&canonical_path, false)?;
    identity_and_bundle_leases_with_owner_validation(kind, &canonical_path, &mut file, None, false)
        .map(|(identity, _)| identity)
}

pub fn artifact_identity_for_path_with_deadline(
    kind: &str,
    path: &Path,
    deadline: Instant,
) -> Result<ArtifactIdentity, String> {
    let canonical_path = std::fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve {} artifact: {error}", kind))?;
    let mut file = open_artifact_file(&canonical_path, false)?;
    identity_and_bundle_leases_with_owner_validation(
        kind,
        &canonical_path,
        &mut file,
        Some(deadline),
        false,
    )
    .map(|(identity, _)| identity)
}

#[cfg(unix)]
fn validate_owner_protected_tree(path: &Path, root: &Path) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    // SAFETY: geteuid has no preconditions and does not mutate process state.
    let effective_uid = unsafe { libc::geteuid() };

    let mut current = path.to_path_buf();
    loop {
        let metadata = std::fs::symlink_metadata(&current).map_err(|error| {
            format!(
                "failed to inspect protected artifact path {}: {error}",
                current.display()
            )
        })?;
        if metadata.uid() != effective_uid && metadata.uid() != 0 {
            return Err(format!(
                "artifact path is owned by an untrusted OS principal: {}",
                current.display()
            ));
        }
        let mode = metadata.permissions().mode();
        let shared_writable = mode & 0o022 != 0;
        if shared_writable {
            return Err(format!(
                "artifact path is writable by another OS principal: {}",
                current.display()
            ));
        }
        if metadata.is_file() && metadata.nlink() != 1 {
            return Err(format!(
                "artifact file must have exactly one hard link: {}",
                current.display()
            ));
        }
        if crate::path_utils::paths_equal(&current, root) {
            break;
        }
        current = current
            .parent()
            .ok_or_else(|| {
                format!(
                    "artifact path escaped its authorized root {}",
                    root.display()
                )
            })?
            .to_path_buf();
    }
    Ok(())
}

#[cfg(windows)]
fn windows_sid_string(sid: windows_sys::Win32::Security::PSID) -> Result<String, String> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;

    if sid.is_null() {
        return Err("Windows security descriptor contains a null SID".to_string());
    }
    let mut text = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(format!(
            "failed to format Windows artifact SID: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut length = 0_usize;
    unsafe {
        while *text.add(length) != 0 {
            length += 1;
        }
    }
    let value = unsafe {
        std::ffi::OsString::from_wide(std::slice::from_raw_parts(text, length))
            .to_string_lossy()
            .into_owned()
    };
    unsafe {
        LocalFree(text.cast());
    }
    Ok(value)
}

#[cfg(windows)]
fn trusted_windows_owner_sid(sid: &str, current_sid: &str) -> bool {
    sid.eq_ignore_ascii_case(current_sid)
        || matches!(
            sid,
            // Local System, BUILTIN\Administrators, and the Windows Modules
            // Installer (TrustedInstaller).
            "S-1-5-18"
                | "S-1-5-32-544"
                | "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"
        )
}

#[cfg(windows)]
fn trusted_windows_artifact_sid(sid: &str, owner_sid: &str, current_sid: &str) -> bool {
    sid.eq_ignore_ascii_case(owner_sid)
        || trusted_windows_owner_sid(sid, current_sid)
        // Inheritance templates that resolve to the already trusted owner.
        || matches!(sid, "S-1-3-0" | "S-1-3-4")
}

#[cfg(windows)]
fn validate_windows_owner_protected_handle(file: &File, display: &Path) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        GetAce, ACCESS_ALLOWED_ACE, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    };

    const ERROR_SUCCESS: u32 = 0;
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACCESS_ALLOWED_COMPOUND_ACE_TYPE: u8 = 4;
    const ACCESS_ALLOWED_OBJECT_ACE_TYPE: u8 = 5;
    const ACCESS_ALLOWED_CALLBACK_ACE_TYPE: u8 = 9;
    const ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE: u8 = 11;
    const INHERIT_ONLY_ACE: u8 = 0x08;
    const DANGEROUS_ACCESS: u32 = 0x4000_0000 // GENERIC_WRITE
        | 0x1000_0000 // GENERIC_ALL
        | 0x0200_0000 // MAXIMUM_ALLOWED
        | 0x0001_0000 // DELETE
        | 0x0004_0000 // WRITE_DAC
        | 0x0008_0000 // WRITE_OWNER
        | 0x0000_0002 // FILE_WRITE_DATA / FILE_ADD_FILE
        | 0x0000_0004 // FILE_APPEND_DATA / FILE_ADD_SUBDIRECTORY
        | 0x0000_0010 // FILE_WRITE_EA
        | 0x0000_0040 // FILE_DELETE_CHILD
        | 0x0000_0100; // FILE_WRITE_ATTRIBUTES

    let mut owner = std::ptr::null_mut();
    let mut dacl = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "failed to inspect Windows artifact ACL {}: OS error {status}",
            display.display()
        ));
    }
    let result = (|| {
        if dacl.is_null() {
            return Err(format!(
                "artifact path has an unrestricted null DACL: {}",
                display.display()
            ));
        }
        let owner_sid = windows_sid_string(owner)?;
        let current_sid = crate::persistence::windows_process_sid(None)?;
        if !trusted_windows_owner_sid(&owner_sid, &current_sid) {
            return Err(format!(
                "artifact path is owned by an untrusted Windows principal: {}",
                display.display()
            ));
        }
        let acl = unsafe { &*dacl };
        for index in 0..u32::from(acl.AceCount) {
            let mut raw_ace = std::ptr::null_mut();
            if unsafe { GetAce(dacl, index, &mut raw_ace) } == 0 || raw_ace.is_null() {
                return Err(format!(
                    "failed to inspect Windows artifact ACL entry for {}",
                    display.display()
                ));
            }
            let header = unsafe { &*(raw_ace.cast::<windows_sys::Win32::Security::ACE_HEADER>()) };
            if header.AceFlags & INHERIT_ONLY_ACE != 0 {
                continue;
            }
            if !matches!(
                header.AceType,
                ACCESS_ALLOWED_ACE_TYPE
                    | ACCESS_ALLOWED_COMPOUND_ACE_TYPE
                    | ACCESS_ALLOWED_OBJECT_ACE_TYPE
                    | ACCESS_ALLOWED_CALLBACK_ACE_TYPE
                    | ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE
            ) {
                continue;
            }
            if usize::from(header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>() {
                return Err(format!(
                    "Windows artifact ACL contains a malformed allow entry: {}",
                    display.display()
                ));
            }
            let allow = unsafe { &*(raw_ace.cast::<ACCESS_ALLOWED_ACE>()) };
            if allow.Mask & DANGEROUS_ACCESS == 0 {
                continue;
            }
            if !matches!(
                header.AceType,
                ACCESS_ALLOWED_ACE_TYPE | ACCESS_ALLOWED_CALLBACK_ACE_TYPE
            ) {
                return Err(format!(
                    "artifact path grants write-like access through an unsupported Windows ACL entry: {}",
                    display.display()
                ));
            }
            let sid = std::ptr::addr_of!(allow.SidStart) as windows_sys::Win32::Security::PSID;
            let sid = windows_sid_string(sid)?;
            if !trusted_windows_artifact_sid(&sid, &owner_sid, &current_sid) {
                return Err(format!(
                    "artifact path is writable by another Windows principal: {}",
                    display.display()
                ));
            }
        }
        Ok(())
    })();
    unsafe {
        LocalFree(descriptor.cast());
    }
    result
}

#[cfg(windows)]
fn validate_owner_protected_tree(path: &Path, root: &Path) -> Result<(), String> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;

    let mut current = path.to_path_buf();
    loop {
        let mut options = std::fs::OpenOptions::new();
        options
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
        let handle = options.open(&current).map_err(|error| {
            format!(
                "failed to open protected Windows artifact path {}: {error}",
                current.display()
            )
        })?;
        validate_windows_owner_protected_handle(&handle, &current)?;
        if crate::path_utils::paths_equal(&current, root) {
            break;
        }
        current = current
            .parent()
            .ok_or_else(|| {
                format!(
                    "artifact path escaped its authorized root {}",
                    root.display()
                )
            })?
            .to_path_buf();
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn validate_owner_protected_tree(_path: &Path, _root: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn open_windows_ancestor_guards(path: &Path, root: &Path) -> Result<Vec<File>, String> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;

    let mut ancestors = Vec::new();
    let mut current = path
        .parent()
        .ok_or_else(|| "artifact has no parent directory".to_string())?
        .to_path_buf();
    loop {
        if !crate::path_utils::path_is_within(&current, root) {
            return Err(format!(
                "artifact ancestor escaped its authorized root: {}",
                current.display()
            ));
        }
        ancestors.push(current.clone());
        if crate::path_utils::paths_equal(&current, root) {
            break;
        }
        current = current
            .parent()
            .ok_or_else(|| {
                format!(
                    "artifact ancestor escaped its authorized root {}",
                    root.display()
                )
            })?
            .to_path_buf();
    }

    // Bind the path from the authorized root toward the leaf. Once a parent
    // handle is retained without write/delete sharing, a later component
    // cannot be swapped out behind the remaining path walk.
    ancestors.reverse();
    let mut guards = Vec::with_capacity(ancestors.len());
    for current in ancestors {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
        let guard = options.open(&current).map_err(|error| {
            format!(
                "failed to lock artifact ancestor {}: {error}",
                current.display()
            )
        })?;
        if !guard
            .metadata()
            .map_err(|error| format!("failed to inspect artifact ancestor: {error}"))?
            .is_dir()
        {
            return Err(format!(
                "artifact ancestor is not a directory: {}",
                current.display()
            ));
        }
        guards.push(guard);
    }
    Ok(guards)
}

#[cfg(not(windows))]
fn open_windows_ancestor_guards(_path: &Path, _root: &Path) -> Result<Vec<File>, String> {
    Ok(Vec::new())
}

#[cfg(target_os = "linux")]
fn inheritable_artifact_path(file: &File) -> Result<PathBuf, String> {
    use std::os::fd::AsRawFd;

    let descriptor = file.as_raw_fd();
    // SAFETY: fcntl is called for a live descriptor owned by `file`; the
    // returned flags are checked before updating only FD_CLOEXEC.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    if flags < 0 {
        return Err(format!(
            "failed to inspect artifact descriptor flags: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: the descriptor remains owned by `file` and the new flag value
    // preserves every bit except FD_CLOEXEC so the verified object reaches the child.
    if unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        return Err(format!(
            "failed to make artifact descriptor inheritable: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(PathBuf::from(format!("/proc/self/fd/{descriptor}")))
}

#[cfg(target_os = "linux")]
fn artifact_launch_path(_canonical_path: &Path, file: &File) -> Result<PathBuf, String> {
    inheritable_artifact_path(file)
}

#[cfg(not(target_os = "linux"))]
fn artifact_launch_path(canonical_path: &Path, _file: &File) -> Result<PathBuf, String> {
    // macOS exposes descriptors through /dev/fd, but that filesystem cannot
    // be used as an exec path. On platforms without executable fd paths, the
    // owner-protected tree excludes other OS principals while the retained
    // lease and post-spawn verification detect in-place artifact mutation.
    Ok(canonical_path.to_path_buf())
}

#[derive(Debug)]
pub struct ArtifactLease {
    kind: String,
    canonical_path: PathBuf,
    authorized_root: PathBuf,
    launch_path: PathBuf,
    file: File,
    identity: ArtifactIdentity,
    // Evidence remains bound to the user-selected source path even when the
    // executable is launched from an application-owned snapshot.
    fingerprint: Option<String>,
    // Integrity checks always bind the object retained by this lease.
    integrity_fingerprint: Option<String>,
    managed_snapshot: bool,
    bundle_members: Vec<EngineBundleMemberLease>,
    _ancestor_guards: Vec<File>,
}

#[cfg(windows)]
fn is_owner_protection_error(error: &str) -> bool {
    error.contains("artifact path is writable by another Windows principal")
        || error.contains("artifact path is owned by an untrusted Windows principal")
        || error.contains("artifact path has an unrestricted null DACL")
        || error.contains(
            "artifact path grants write-like access through an unsupported Windows ACL entry",
        )
}

#[cfg(windows)]
struct ManagedSnapshotStaging {
    path: PathBuf,
    armed: bool,
}

#[cfg(windows)]
impl ManagedSnapshotStaging {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(windows)]
impl Drop for ManagedSnapshotStaging {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(windows)]
fn copy_leased_artifact(
    source: &mut File,
    expected_size: u64,
    destination: &Path,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "managed engine snapshot destination has no parent".to_string())?;
    crate::persistence::enforce_private_directory(parent)?;
    source.seek(SeekFrom::Start(0)).map_err(|error| {
        format!(
            "failed to rewind managed engine snapshot source {}: {error}",
            destination.display()
        )
    })?;
    let mut destination_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|error| {
            format!(
                "failed to create managed engine snapshot file {}: {error}",
                destination.display()
            )
        })?;
    let copied = std::io::copy(source, &mut destination_file).map_err(|error| {
        format!(
            "failed to copy managed engine snapshot file {}: {error}",
            destination.display()
        )
    })?;
    if copied != expected_size {
        return Err(format!(
            "managed engine snapshot copied {copied} bytes but expected {expected_size}: {}",
            destination.display()
        ));
    }
    destination_file.flush().map_err(|error| {
        format!(
            "failed to flush managed engine snapshot file {}: {error}",
            destination.display()
        )
    })?;
    destination_file.sync_all().map_err(|error| {
        format!(
            "failed to persist managed engine snapshot file {}: {error}",
            destination.display()
        )
    })?;
    drop(destination_file);
    crate::persistence::enforce_private_file(destination)
}

impl ArtifactLease {
    pub fn open_beneath_authorized_root_with_deadline(
        kind: &str,
        path: &Path,
        root: &Path,
        deadline: Instant,
    ) -> Result<Self, String> {
        match Self::open_beneath_authorized_root_with_policy(kind, path, root, Some(deadline), true)
        {
            Ok(lease) => Ok(lease),
            Err(strict_error) => {
                #[cfg(windows)]
                {
                    if kind == "model" && is_owner_protection_error(&strict_error) {
                        return Self::open_beneath_authorized_root_with_policy(
                            kind,
                            path,
                            root,
                            Some(deadline),
                            false,
                        )
                        .map_err(|binding_error| {
                            format!(
                                "model source did not satisfy the strict ACL policy ({strict_error}); stable read-only binding failed: {binding_error}"
                            )
                        });
                    }
                }
                Err(strict_error)
            }
        }
    }

    fn open_beneath_authorized_root_with_policy(
        kind: &str,
        path: &Path,
        root: &Path,
        deadline: Option<Instant>,
        validate_owner: bool,
    ) -> Result<Self, String> {
        let canonical_path = std::fs::canonicalize(path)
            .map_err(|error| format!("failed to resolve {kind} artifact: {error}"))?;
        let authorized_root = std::fs::canonicalize(root)
            .map_err(|error| format!("failed to resolve authorized artifact root: {error}"))?;
        if !crate::path_utils::paths_equal(&canonical_path, &authorized_root)
            && !crate::path_utils::path_is_within(&canonical_path, &authorized_root)
        {
            return Err(format!("{kind} artifact escaped its authorized root"));
        }
        if validate_owner {
            validate_owner_protected_tree(&canonical_path, &authorized_root)?;
        }
        let ancestor_guards = open_windows_ancestor_guards(&canonical_path, &authorized_root)?;
        let mut file = open_artifact_file(&canonical_path, false)?;
        FileExt::try_lock_shared(&file)
            .map_err(|error| format!("artifact is being modified: {error}"))?;
        let modified = modified_nanos(&file);
        let (identity, bundle_members) = identity_and_bundle_leases_with_owner_validation(
            kind,
            &canonical_path,
            &mut file,
            deadline,
            validate_owner,
        )?;
        let fingerprint = (kind == "engine")
            .then(|| engine_fingerprint_material(&canonical_path, modified, &identity));
        let integrity_fingerprint = fingerprint.clone();
        let launch_path = artifact_launch_path(&canonical_path, &file)?;
        Ok(Self {
            kind: kind.to_string(),
            canonical_path,
            authorized_root,
            launch_path,
            file,
            identity,
            fingerprint,
            integrity_fingerprint,
            managed_snapshot: false,
            bundle_members,
            _ancestor_guards: ancestor_guards,
        })
    }

    pub fn open_owner_protected_executable(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err("executable path must be absolute".to_string());
        }
        let original = std::fs::symlink_metadata(path)
            .map_err(|error| format!("failed to inspect executable path: {error}"))?;
        if original.file_type().is_symlink() || !original.is_file() {
            return Err("executable must be a regular non-link file".to_string());
        }
        let canonical_path = std::fs::canonicalize(path)
            .map_err(|error| format!("failed to resolve executable: {error}"))?;
        let authorized_root = canonical_path
            .parent()
            .ok_or_else(|| "executable has no parent directory".to_string())?
            .to_path_buf();
        validate_owner_protected_tree(&canonical_path, &authorized_root)?;
        let ancestor_guards = open_windows_ancestor_guards(&canonical_path, &authorized_root)?;
        let mut file = open_artifact_file(&canonical_path, false)?;
        FileExt::try_lock_shared(&file)
            .map_err(|error| format!("executable is being modified: {error}"))?;
        let modified = modified_nanos(&file);
        let (identity, bundle_members) =
            identity_and_bundle_leases("engine", &canonical_path, &mut file, None)?;
        let fingerprint = Some(engine_fingerprint_material(
            &canonical_path,
            modified,
            &identity,
        ));
        let integrity_fingerprint = fingerprint.clone();
        let launch_path = artifact_launch_path(&canonical_path, &file)?;
        Ok(Self {
            kind: "engine".to_string(),
            canonical_path,
            authorized_root,
            launch_path,
            file,
            identity,
            fingerprint,
            integrity_fingerprint,
            managed_snapshot: false,
            bundle_members,
            _ancestor_guards: ancestor_guards,
        })
    }

    pub fn open_authorized(kind: &str, path: &Path) -> Result<Self, String> {
        Self::open_authorized_with_mode(kind, path, false, None)
    }

    pub fn open_authorized_engine(path: &Path) -> Result<Self, String> {
        match Self::open_authorized("engine", path) {
            Ok(lease) => Ok(lease),
            Err(strict_error) => {
                #[cfg(windows)]
                {
                    if is_owner_protection_error(&strict_error) {
                        Self::open_managed_engine_snapshot(path).map_err(|snapshot_error| {
                            format!(
                                "engine source did not satisfy direct execution policy ({strict_error}); managed private snapshot failed: {snapshot_error}"
                            )
                        })
                    } else {
                        Err(strict_error)
                    }
                }
                #[cfg(not(windows))]
                {
                    Err(strict_error)
                }
            }
        }
    }

    pub fn open_authorized_model_for_launch(path: &Path) -> Result<Self, String> {
        // A GGUF may be tens or hundreds of gigabytes, so copying it into an
        // application-private snapshot would impose unacceptable storage and
        // startup costs. On Windows, an authorized external model instead
        // falls back to a complete identity check plus retained file and
        // root-to-leaf ancestor handles that deny write/delete sharing for the
        // lifetime of qualification or the managed server process.
        match Self::open_authorized("model", path) {
            Ok(lease) => Ok(lease),
            Err(strict_error) => {
                #[cfg(windows)]
                {
                    if is_owner_protection_error(&strict_error) {
                        let (canonical_path, authorized_root) =
                            crate::security::require_authorized_artifact_path("model", path)?;
                        return Self::open_beneath_authorized_root_with_policy(
                            "model",
                            &canonical_path,
                            &authorized_root,
                            None,
                            false,
                        )
                        .map_err(|binding_error| {
                            format!(
                                "model source did not satisfy the strict ACL policy ({strict_error}); stable read-only binding failed: {binding_error}"
                            )
                        });
                    }
                }
                Err(strict_error)
            }
        }
    }

    #[cfg(windows)]
    fn open_managed_engine_snapshot(path: &Path) -> Result<Self, String> {
        let (canonical_path, authorized_root) =
            crate::security::require_authorized_artifact_path("engine", path)?;
        let ancestor_guards = open_windows_ancestor_guards(&canonical_path, &authorized_root)?;
        let mut file = open_artifact_file(&canonical_path, false)?;
        FileExt::try_lock_shared(&file).map_err(|error| {
            format!(
                "engine source is being modified by another process {}: {error}",
                canonical_path.display()
            )
        })?;
        let modified = modified_nanos(&file);
        let (identity, bundle_members) = identity_and_bundle_leases_with_owner_validation(
            "engine",
            &canonical_path,
            &mut file,
            None,
            false,
        )?;
        let fingerprint = Some(engine_fingerprint_material(
            &canonical_path,
            modified,
            &identity,
        ));
        let launch_path = artifact_launch_path(&canonical_path, &file)?;
        let mut source = Self {
            kind: "engine".to_string(),
            canonical_path,
            authorized_root,
            launch_path,
            file,
            identity,
            integrity_fingerprint: fingerprint.clone(),
            fingerprint,
            managed_snapshot: false,
            bundle_members,
            _ancestor_guards: ancestor_guards,
        };
        let data_dir = crate::utils::get_data_dir();
        source.stage_managed_engine_snapshot_at(&data_dir)
    }

    #[cfg(windows)]
    fn stage_managed_engine_snapshot_at(&mut self, data_dir: &Path) -> Result<Self, String> {
        let executable_name = self
            .canonical_path
            .file_name()
            .ok_or_else(|| "engine executable has no file name".to_string())?
            .to_os_string();
        let mut cache_key = Sha256::new();
        cache_key.update(b"llama-server-manager:managed-engine-snapshot:v1\0");
        cache_key.update(self.identity.artifact_id.as_bytes());
        cache_key.update(
            executable_name
                .to_string_lossy()
                .to_ascii_lowercase()
                .as_bytes(),
        );
        let cache_key = format!("{:x}", cache_key.finalize());
        let snapshot_root = data_dir.join("engine-snapshots");
        let version_root = snapshot_root.join("v1");
        crate::persistence::enforce_private_directory(data_dir)?;
        crate::persistence::enforce_private_directory(&snapshot_root)?;
        crate::persistence::enforce_private_directory(&version_root)?;

        let final_dir = version_root.join(cache_key);
        let final_executable = final_dir.join(&executable_name);
        let source_identity = self.identity.clone();
        let evidence_fingerprint = self.fingerprint.clone();
        if final_dir.exists() {
            return Self::bind_existing_managed_snapshot(
                &final_executable,
                &source_identity,
                evidence_fingerprint,
            );
        }

        let staging_path = version_root.join(format!(".staging-{}", uuid::Uuid::new_v4().simple()));
        crate::persistence::enforce_private_directory(&staging_path)?;
        let mut staging = ManagedSnapshotStaging::new(staging_path.clone());
        let staged_executable = staging_path.join(&executable_name);
        let primary_size = self
            .file
            .metadata()
            .map_err(|error| {
                format!(
                    "failed to inspect engine source {}: {error}",
                    self.canonical_path.display()
                )
            })?
            .len();
        copy_leased_artifact(&mut self.file, primary_size, &staged_executable)?;
        for member in &mut self.bundle_members {
            let destination = staging_path.join(Path::new(&member.relative_path));
            copy_leased_artifact(&mut member.file, member.identity.file_size, &destination)?;
        }

        {
            let staged = Self::open_owner_protected_executable(&staged_executable)?;
            if staged.identity != source_identity {
                return Err(
                    "managed engine snapshot does not match the verified source bundle".to_string(),
                );
            }
        }
        match std::fs::rename(&staging_path, &final_dir) {
            Ok(()) => staging.disarm(),
            Err(_error) if final_dir.exists() => {
                drop(staging);
                return Self::bind_existing_managed_snapshot(
                    &final_executable,
                    &source_identity,
                    evidence_fingerprint,
                );
            }
            Err(error) => {
                return Err(format!(
                    "failed to publish managed engine snapshot {}: {error}",
                    final_dir.display()
                ));
            }
        }
        Self::bind_existing_managed_snapshot(
            &final_executable,
            &source_identity,
            evidence_fingerprint,
        )
    }

    #[cfg(windows)]
    fn bind_existing_managed_snapshot(
        executable: &Path,
        expected_identity: &ArtifactIdentity,
        evidence_fingerprint: Option<String>,
    ) -> Result<Self, String> {
        let mut snapshot = Self::open_owner_protected_executable(executable)?;
        if snapshot.identity != *expected_identity {
            return Err(format!(
                "managed engine snapshot identity mismatch: {}",
                executable.display()
            ));
        }
        snapshot.fingerprint = evidence_fingerprint;
        snapshot.managed_snapshot = true;
        Ok(snapshot)
    }

    pub fn open_authorized_with_deadline(
        kind: &str,
        path: &Path,
        deadline: Instant,
    ) -> Result<Self, String> {
        Self::open_authorized_with_mode(kind, path, false, Some(deadline))
    }

    pub fn open_authorized_for_removal(kind: &str, path: &Path) -> Result<Self, String> {
        Self::open_authorized_with_mode(kind, path, true, None)
    }

    pub fn open_authorized_for_removal_with_deadline(
        kind: &str,
        path: &Path,
        deadline: Instant,
    ) -> Result<Self, String> {
        Self::open_authorized_with_mode(kind, path, true, Some(deadline))
    }

    fn open_authorized_with_mode(
        kind: &str,
        path: &Path,
        allow_delete: bool,
        deadline: Option<Instant>,
    ) -> Result<Self, String> {
        let (canonical_path, authorized_root) =
            crate::security::require_authorized_artifact_path(kind, path)?;
        validate_owner_protected_tree(&canonical_path, &authorized_root)?;
        let ancestor_guards = open_windows_ancestor_guards(&canonical_path, &authorized_root)?;
        let mut file = open_artifact_file(&canonical_path, allow_delete)?;
        FileExt::try_lock_shared(&file).map_err(|error| {
            format!(
                "artifact is being modified by another process {}: {error}",
                canonical_path.display()
            )
        })?;
        let modified = modified_nanos(&file);
        let (identity, bundle_members) =
            identity_and_bundle_leases(kind, &canonical_path, &mut file, deadline)?;
        let fingerprint = (kind == "engine")
            .then(|| engine_fingerprint_material(&canonical_path, modified, &identity));
        let integrity_fingerprint = fingerprint.clone();
        let launch_path = artifact_launch_path(&canonical_path, &file)?;
        Ok(Self {
            kind: kind.to_string(),
            canonical_path,
            authorized_root,
            launch_path,
            file,
            identity,
            fingerprint,
            integrity_fingerprint,
            managed_snapshot: false,
            bundle_members,
            _ancestor_guards: ancestor_guards,
        })
    }

    pub fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    pub fn fingerprint(&self) -> Option<&str> {
        self.fingerprint.as_deref()
    }

    pub fn uses_managed_snapshot(&self) -> bool {
        self.managed_snapshot
    }

    pub fn launch_path(&self) -> &Path {
        &self.launch_path
    }

    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub fn try_clone_file(&self) -> Result<File, String> {
        self.file
            .try_clone()
            .map_err(|error| format!("failed to clone verified artifact handle: {error}"))
    }

    /// Atomically quarantines the directory entry under a retained directory
    /// capability, verifies that the moved object is the leased artifact, and
    /// only then unlinks it. A raced replacement is restored and never deleted.
    pub fn remove_verified(&mut self) -> Result<(), String> {
        self.remove_verified_inner(None)
    }

    pub fn remove_verified_with_deadline(&mut self, deadline: Instant) -> Result<(), String> {
        self.remove_verified_inner(Some(deadline))
    }

    fn remove_verified_inner(&mut self, deadline: Option<Instant>) -> Result<(), String> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};

        if self.kind != "model" {
            return Err("only model artifacts may be removed through a lease".into());
        }
        self.verify_unchanged_inner(deadline)?;
        let parent = self
            .canonical_path
            .parent()
            .ok_or_else(|| "artifact has no parent directory".to_string())?;
        let name = self
            .canonical_path
            .file_name()
            .ok_or_else(|| "artifact has no file name".to_string())?;
        let root =
            cap_std::fs::Dir::open_ambient_dir(&self.authorized_root, cap_std::ambient_authority())
                .map_err(|error| format!("failed to bind authorized artifact root: {error}"))?;
        let relative_parent = parent
            .strip_prefix(&self.authorized_root)
            .map_err(|_| "artifact parent escaped its authorized root".to_string())?;
        let directory = if relative_parent.as_os_str().is_empty() {
            root.try_clone()
        } else {
            root.open_dir(relative_parent)
        }
        .map_err(|error| format!("failed to bind artifact parent directory: {error}"))?;
        let quarantine = format!(
            ".{}.{}.delete",
            name.to_string_lossy(),
            uuid::Uuid::new_v4().simple()
        );
        directory
            .rename(name, &directory, &quarantine)
            .map_err(|error| format!("failed to quarantine verified model: {error}"))?;
        let result = (|| {
            let mut options = cap_std::fs::OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            let mut moved = directory
                .open_with(&quarantine, &options)
                .map(cap_std::fs::File::into_std)
                .map_err(|error| format!("failed to open quarantined model: {error}"))?;
            let identity =
                artifact_identity_from_open_file_with_deadline("model", &mut moved, deadline)?;
            if identity != self.identity {
                return Err("model entry changed before quarantine; deletion refused".to_string());
            }
            directory
                .remove_file(&quarantine)
                .map_err(|error| format!("failed to remove quarantined model: {error}"))
        })();
        if result.is_err() && directory.symlink_metadata(name).is_err() {
            let _ = directory.rename(&quarantine, &directory, name);
        }
        result
    }

    pub fn verify_unchanged(&mut self) -> Result<(), String> {
        self.verify_unchanged_inner(None)
    }

    pub fn verify_unchanged_with_deadline(&mut self, deadline: Instant) -> Result<(), String> {
        self.verify_unchanged_inner(Some(deadline))
    }

    fn verify_unchanged_inner(&mut self, deadline: Option<Instant>) -> Result<(), String> {
        let current = if self.kind == "engine" {
            let primary =
                artifact_identity_from_open_file_with_deadline("engine", &mut self.file, deadline)?;
            for member in &mut self.bundle_members {
                let current = artifact_identity_from_open_file_with_deadline(
                    "engine",
                    &mut member.file,
                    deadline,
                )?;
                if current != member.identity {
                    return Err(format!(
                        "engine bundle member changed after validation: {}",
                        member.relative_path
                    ));
                }
            }
            #[cfg(windows)]
            {
                aggregate_engine_bundle_identity(&primary, &self.bundle_members)?
            }
            #[cfg(not(windows))]
            {
                primary
            }
        } else {
            artifact_identity_from_open_file_with_deadline(&self.kind, &mut self.file, deadline)?
        };
        if current != self.identity {
            return Err(format!(
                "{} artifact changed after validation: {}",
                self.kind,
                self.canonical_path.display()
            ));
        }
        if self.kind == "engine" {
            let current_fingerprint = engine_fingerprint_material(
                &self.canonical_path,
                modified_nanos(&self.file),
                &current,
            );
            if self.integrity_fingerprint.as_deref() != Some(current_fingerprint.as_str()) {
                return Err(format!(
                    "engine artifact metadata changed after validation: {}",
                    self.canonical_path.display()
                ));
            }
        }
        Ok(())
    }
}

impl Drop for ArtifactLease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
        for member in &self.bundle_members {
            let _ = FileExt::unlock(&member.file);
        }
    }
}

#[derive(Debug)]
pub struct LaunchArtifactLeases {
    engine: ArtifactLease,
    model: ArtifactLease,
    auxiliary: Vec<(String, ArtifactLease)>,
}

impl LaunchArtifactLeases {
    pub fn engine_identity(&self) -> &ArtifactIdentity {
        self.engine.identity()
    }

    pub fn model_identity(&self) -> &ArtifactIdentity {
        self.model.identity()
    }

    pub fn engine_fingerprint(&self) -> &str {
        self.engine.fingerprint().unwrap_or_default()
    }

    pub fn verify_unchanged(&mut self) -> Result<(), String> {
        self.engine.verify_unchanged()?;
        self.model.verify_unchanged()?;
        for (_, artifact) in &mut self.auxiliary {
            artifact.verify_unchanged()?;
        }
        Ok(())
    }
}

enum ModelArgumentLocation {
    Separate(usize),
    Inline(usize),
}

fn effective_auxiliary_argument(
    command: &[String],
    role: &str,
) -> Result<ModelArgumentLocation, String> {
    let flags: &[&str] = match role {
        "draft_model" => &["-md", "--draft-model", "--model-draft"],
        "mmproj" => &["--mmproj"],
        _ => return Err("managed launch has an unsupported auxiliary artifact role".to_string()),
    };
    let mut found = None;
    let mut index = 1;
    while index < command.len() {
        let argument = command[index].as_str();
        let candidate = if flags.contains(&argument) {
            let value_index = index + 1;
            if value_index >= command.len() || command[value_index].trim().is_empty() {
                return Err(format!("managed launch {role} flag has no value"));
            }
            index += 2;
            Some(ModelArgumentLocation::Separate(value_index))
        } else if let Some(flag) = flags
            .iter()
            .find(|flag| argument.starts_with(&format!("{flag}=")))
        {
            if argument.len() <= flag.len() + 1 {
                return Err(format!("managed launch {role} flag has no value"));
            }
            index += 1;
            Some(ModelArgumentLocation::Inline(index - 1))
        } else {
            index += 1;
            None
        };
        if let Some(candidate) = candidate {
            if found.is_some() {
                return Err(format!("managed launch contains duplicate {role} flags"));
            }
            found = Some(candidate);
        }
    }
    found.ok_or_else(|| format!("managed launch has no effective {role} flag"))
}

fn effective_model_argument(command: &[String]) -> Result<ModelArgumentLocation, String> {
    let mut found = None;
    let mut index = 1;
    while index < command.len() {
        let argument = command[index].as_str();
        let candidate = if matches!(argument, "-m" | "--model") {
            let value_index = index + 1;
            if value_index >= command.len() || command[value_index].trim().is_empty() {
                return Err("managed launch model flag has no value".to_string());
            }
            index += 2;
            Some(ModelArgumentLocation::Separate(value_index))
        } else if argument.starts_with("--model=") && argument.len() > "--model=".len() {
            index += 1;
            Some(ModelArgumentLocation::Inline(index - 1))
        } else {
            index += 1;
            None
        };
        if let Some(candidate) = candidate {
            if found.is_some() {
                return Err("managed launch contains duplicate model flags".to_string());
            }
            found = Some(candidate);
        }
    }
    found.ok_or_else(|| "managed launch has no effective model flag".to_string())
}

pub fn bind_launch_artifacts(
    command: &[String],
    identity: &DeploymentIdentity,
) -> Result<(Vec<String>, LaunchArtifactLeases), String> {
    let (mut bound, mut leases) = bind_expected_artifacts(
        command,
        &identity.engine_artifact_id,
        &identity.model_artifact_id,
    )?;
    for expected in &identity.auxiliary_artifacts {
        let location = effective_auxiliary_argument(&bound, &expected.role)?;
        let path = match location {
            ModelArgumentLocation::Separate(index) => PathBuf::from(&bound[index]),
            ModelArgumentLocation::Inline(index) => PathBuf::from(
                bound[index]
                    .split_once('=')
                    .map(|(_, value)| value)
                    .unwrap_or_default(),
            ),
        };
        let artifact = ArtifactLease::open_authorized_model_for_launch(&path)?;
        if artifact.identity().artifact_id != expected.artifact_id {
            return Err(format!(
                "DEPLOYMENT_AUXILIARY_IDENTITY_STALE: verified {} object changed",
                expected.role
            ));
        }
        match location {
            ModelArgumentLocation::Separate(index) => {
                bound[index] = artifact.launch_path().to_string_lossy().to_string();
            }
            ModelArgumentLocation::Inline(index) => {
                let flag = bound[index]
                    .split_once('=')
                    .map(|(flag, _)| flag)
                    .unwrap_or_default();
                bound[index] = format!("{flag}={}", artifact.launch_path().to_string_lossy());
            }
        }
        leases.auxiliary.push((expected.role.clone(), artifact));
    }
    Ok((bound, leases))
}

pub fn bind_expected_artifacts(
    command: &[String],
    expected_engine_artifact_id: &str,
    expected_model_artifact_id: &str,
) -> Result<(Vec<String>, LaunchArtifactLeases), String> {
    if command.is_empty() || command[0].trim().is_empty() {
        return Err("launch command has no executable".to_string());
    }
    let model_location = effective_model_argument(command)?;
    let model_path = match model_location {
        ModelArgumentLocation::Separate(index) => PathBuf::from(&command[index]),
        ModelArgumentLocation::Inline(index) => {
            PathBuf::from(command[index].trim_start_matches("--model="))
        }
    };
    let engine = ArtifactLease::open_authorized_engine(Path::new(&command[0]))?;
    if engine.identity().artifact_id != expected_engine_artifact_id {
        return Err("DEPLOYMENT_ENGINE_IDENTITY_STALE: verified engine object changed".to_string());
    }
    let model = ArtifactLease::open_authorized_model_for_launch(&model_path)?;
    if model.identity().artifact_id != expected_model_artifact_id {
        return Err("DEPLOYMENT_MODEL_IDENTITY_STALE: verified model object changed".to_string());
    }
    let mut bound = command.to_vec();
    bound[0] = engine.launch_path().to_string_lossy().to_string();
    match model_location {
        ModelArgumentLocation::Separate(index) => {
            bound[index] = model.launch_path().to_string_lossy().to_string();
        }
        ModelArgumentLocation::Inline(index) => {
            bound[index] = format!("--model={}", model.launch_path().to_string_lossy());
        }
    }
    Ok((
        bound,
        LaunchArtifactLeases {
            engine,
            model,
            auxiliary: Vec::new(),
        },
    ))
}

pub fn retain_launch_artifacts(instance_id: &str, leases: LaunchArtifactLeases) {
    ACTIVE_LAUNCH_ARTIFACTS
        .lock()
        .unwrap()
        .insert(instance_id.to_string(), leases);
}

pub fn release_launch_artifacts(instance_id: &str) {
    ACTIVE_LAUNCH_ARTIFACTS.lock().unwrap().remove(instance_id);
    ACTIVE_LAUNCH_PROCESSES.lock().unwrap().remove(instance_id);
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
    fn artifact_launch_path_matches_platform_exec_capability() {
        let dir = std::env::temp_dir().join(format!("lsm-launch-path-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("engine");
        fs::write(&artifact, b"engine").unwrap();
        let canonical = fs::canonicalize(&artifact).unwrap();
        let file = open_artifact_file(&canonical, false).unwrap();
        let launch_path = artifact_launch_path(&canonical, &file).unwrap();
        #[cfg(target_os = "linux")]
        assert!(launch_path.starts_with("/proc/self/fd"));
        #[cfg(not(target_os = "linux"))]
        assert_eq!(launch_path, canonical);
        drop(file);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn full_identity_survives_move_and_changes_with_any_content() {
        let dir = std::env::temp_dir().join(format!("lsm-identity-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let original = dir.join("model.gguf");
        let moved = dir.join("renamed.gguf");
        let mut bytes = vec![7_u8; 384 * 1024];
        fs::write(&original, &bytes).unwrap();
        let before = artifact_identity_for_path("model", &original).unwrap();
        fs::rename(&original, &moved).unwrap();
        let after_move = artifact_identity_for_path("model", &moved).unwrap();
        assert_eq!(before, after_move);
        // This offset sat between the fixed windows used by the retired
        // sampled identity. A complete digest must still observe it.
        bytes[70_000] = 8;
        fs::write(&moved, &bytes).unwrap();
        let after_change = artifact_identity_for_path("model", &moved).unwrap();
        assert_ne!(before.artifact_id, after_change.artifact_id);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scan_identity_hashing_observes_an_expired_deadline() {
        let dir = std::env::temp_dir().join(format!("lsm-identity-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("model.gguf");
        fs::write(&artifact, vec![7_u8; HASH_BUFFER_SIZE + 1]).unwrap();

        let result = artifact_identity_for_path_with_deadline(
            "model",
            &artifact,
            Instant::now() - std::time::Duration::from_millis(1),
        );
        assert!(result
            .unwrap_err()
            .contains("exceeded the scan work deadline"));
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

    #[cfg(windows)]
    #[test]
    fn windows_engine_identity_binds_all_bundle_dlls() {
        let dir = std::env::temp_dir().join(format!("lsm-engine-bundle-{}", uuid::Uuid::new_v4()));
        crate::persistence::enforce_private_directory(&dir).unwrap();
        let engine = dir.join("llama-server.exe");
        let companion = dir.join("ggml-cuda.dll");
        fs::write(&engine, b"engine").unwrap();
        fs::write(&companion, b"companion-v1").unwrap();
        crate::persistence::enforce_private_file(&engine).unwrap();
        crate::persistence::enforce_private_file(&companion).unwrap();
        let first = artifact_identity_for_path("engine", &engine).unwrap();

        fs::write(&companion, b"companion-v2").unwrap();
        let second = artifact_identity_for_path("engine", &engine).unwrap();
        assert_ne!(first.artifact_id, second.artifact_id);

        let unrelated = dir.join("unrelated");
        fs::create_dir(&unrelated).unwrap();
        let plugin = unrelated.join("backend-plugin.dll");
        fs::write(&plugin, b"plugin").unwrap();
        crate::persistence::enforce_private_file(&plugin).unwrap();
        let third = artifact_identity_for_path("engine", &engine).unwrap();
        assert_ne!(second.artifact_id, third.artifact_id);
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn managed_engine_snapshot_preserves_source_evidence_and_private_execution() {
        let root = std::env::temp_dir().join(format!(
            "lsm-managed-engine-snapshot-{}",
            uuid::Uuid::new_v4()
        ));
        let source_dir = root.join("source");
        let data_dir = root.join("data");
        crate::persistence::enforce_private_directory(&source_dir).unwrap();
        let engine = source_dir.join("llama-server.exe");
        let companion_dir = source_dir.join("backend");
        crate::persistence::enforce_private_directory(&companion_dir).unwrap();
        let companion = companion_dir.join("ggml-vulkan.dll");
        fs::write(&engine, b"engine-v1").unwrap();
        fs::write(&companion, b"companion-v1").unwrap();
        crate::persistence::enforce_private_file(&engine).unwrap();
        crate::persistence::enforce_private_file(&companion).unwrap();

        let source_fingerprint = engine_fingerprint_for_path(&engine);
        let mut source = ArtifactLease::open_owner_protected_executable(&engine).unwrap();
        let source_identity = source.identity().clone();
        let mut snapshot = source.stage_managed_engine_snapshot_at(&data_dir).unwrap();

        assert!(snapshot.uses_managed_snapshot());
        assert_eq!(snapshot.identity(), &source_identity);
        assert_eq!(snapshot.fingerprint(), Some(source_fingerprint.as_str()));
        let canonical_snapshot_root = fs::canonicalize(data_dir.join("engine-snapshots")).unwrap();
        assert!(crate::path_utils::path_is_within(
            snapshot.launch_path(),
            &canonical_snapshot_root
        ));
        assert_ne!(snapshot.launch_path(), engine);
        assert!(snapshot.launch_path().is_file());
        assert!(snapshot
            .launch_path()
            .parent()
            .unwrap()
            .join("backend/ggml-vulkan.dll")
            .is_file());
        snapshot.verify_unchanged().unwrap();

        let mut second_source = ArtifactLease::open_owner_protected_executable(&engine).unwrap();
        let second_snapshot = second_source
            .stage_managed_engine_snapshot_at(&data_dir)
            .unwrap();
        assert_eq!(second_snapshot.launch_path(), snapshot.launch_path());
        assert_eq!(second_snapshot.identity(), snapshot.identity());

        drop(second_snapshot);
        drop(second_source);
        drop(snapshot);
        drop(source);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_acl_policy_errors_are_fallback_eligible() {
        assert!(is_owner_protection_error(
            "artifact path is writable by another Windows principal: C:\\engine"
        ));
        assert!(!is_owner_protection_error(
            "engine bundle exceeds 512 DLL members"
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_external_model_uses_stable_read_only_binding() {
        use std::process::{Command, Stdio};

        fn replace_acl(path: &Path, directory: bool) {
            let current_sid = crate::persistence::windows_process_sid(None).unwrap();
            let current_grant = if directory {
                format!("*{current_sid}:(OI)(CI)(F)")
            } else {
                format!("*{current_sid}:(F)")
            };
            let system_grant = if directory {
                "*S-1-5-18:(OI)(CI)(F)"
            } else {
                "*S-1-5-18:(F)"
            };
            let users_grant = if directory {
                "*S-1-5-32-545:(OI)(CI)(M)"
            } else {
                "*S-1-5-32-545:(M)"
            };
            let status = Command::new("icacls")
                .arg(path)
                .args(["/inheritance:r", "/grant:r"])
                .arg(current_grant)
                .arg(system_grant)
                .arg(users_grant)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
        }

        let root = std::env::temp_dir().join(format!(
            "lsm-stable-external-model-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let model = root.join("model.gguf");
        let replacement = root.join("replacement.gguf");
        fs::write(&model, b"verified-model").unwrap();
        replace_acl(&root, true);
        replace_acl(&model, false);

        let strict = ArtifactLease::open_beneath_authorized_root_with_policy(
            "model",
            &model,
            &root,
            Some(Instant::now() + std::time::Duration::from_secs(30)),
            true,
        )
        .unwrap_err();
        assert!(is_owner_protection_error(&strict));

        let mut lease = ArtifactLease::open_beneath_authorized_root_with_deadline(
            "model",
            &model,
            &root,
            Instant::now() + std::time::Duration::from_secs(30),
        )
        .unwrap();
        assert!(lease.identity().is_verified());
        assert_eq!(lease.launch_path(), fs::canonicalize(&model).unwrap());
        assert!(fs::write(&model, b"changed-model").is_err());
        assert!(fs::rename(&model, &replacement).is_err());
        lease.verify_unchanged().unwrap();

        drop(lease);
        fs::write(&model, b"changed-after-release").unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn stable_artifact_handle_blocks_path_replacement_on_windows() {
        let dir = std::env::temp_dir().join(format!("lsm-stable-handle-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("engine.exe");
        let moved = dir.join("engine.old.exe");
        fs::write(&artifact, b"verified-engine").unwrap();

        let lease = open_artifact_file(&artifact, false).unwrap();
        assert!(fs::rename(&artifact, &moved).is_err());
        drop(lease);
        fs::rename(&artifact, &moved).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
