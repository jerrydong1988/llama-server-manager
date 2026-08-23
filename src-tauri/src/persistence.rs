#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    parent_dir(path).join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()))
}

#[cfg(unix)]
fn protect_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if path == Path::new(".") {
        return Ok(());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("failed to protect directory {}: {error}", path.display()))
}

#[cfg(windows)]
fn protect_directory(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    if path == Path::new(".") {
        return Ok(());
    }
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    with_private_security_descriptor(|descriptor| {
        let result = unsafe {
            SetFileSecurityW(
                path.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor,
            )
        };
        if result == 0 {
            Err(format!(
                "failed to protect private Windows directory: {}",
                std::io::Error::last_os_error()
            ))
        } else {
            Ok(())
        }
    })
}

#[cfg(not(any(unix, windows)))]
fn protect_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn protect_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("failed to protect file {}: {error}", path.display()))
}

#[cfg(windows)]
pub(crate) fn with_private_security_descriptor<T>(
    operation: impl FnOnce(windows_sys::Win32::Security::PSECURITY_DESCRIPTOR) -> Result<T, String>,
) -> Result<T, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    let sddl = std::ffi::OsStr::new("D:P(A;;FA;;;SY)(A;;FA;;;OW)")
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut descriptor = std::ptr::null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if converted == 0 {
        return Err(format!(
            "failed to create private Windows security descriptor: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result = operation(descriptor);
    unsafe {
        LocalFree(descriptor as _);
    }
    result
}

#[cfg(windows)]
pub(crate) fn windows_process_sid(process_id: Option<u32>) -> Result<String, String> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_INSUFFICIENT_BUFFER};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let process = process_id
            .map(|pid| OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid))
            .unwrap_or_else(|| GetCurrentProcess());
        if process.is_null() {
            return Err("failed to open Windows process".into());
        }
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            if process_id.is_some() {
                CloseHandle(process);
            }
            return Err("failed to open Windows process token".into());
        }
        if process_id.is_some() {
            CloseHandle(process);
        }
        let mut needed = 0_u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        if GetLastError() != ERROR_INSUFFICIENT_BUFFER || needed == 0 {
            CloseHandle(token);
            return Err("failed to size Windows process token".into());
        }
        let mut buffer = vec![0_u8; needed as usize];
        if GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        ) == 0
        {
            CloseHandle(token);
            return Err("failed to inspect Windows process token".into());
        }
        CloseHandle(token);
        let sid = (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let mut text = std::ptr::null_mut();
        if ConvertSidToStringSidW(sid, &mut text) == 0 {
            return Err("failed to format Windows process SID".into());
        }
        let mut length = 0_usize;
        while *text.add(length) != 0 {
            length += 1;
        }
        let value = std::ffi::OsString::from_wide(std::slice::from_raw_parts(text, length))
            .to_string_lossy()
            .into_owned();
        windows_sys::Win32::Foundation::LocalFree(text.cast());
        Ok(value)
    }
}

#[cfg(windows)]
fn protect_file(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    with_private_security_descriptor(|descriptor| {
        let result = unsafe {
            SetFileSecurityW(
                path.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor,
            )
        };
        if result == 0 {
            Err(format!(
                "failed to protect private Windows file: {}",
                std::io::Error::last_os_error()
            ))
        } else {
            Ok(())
        }
    })
}

#[cfg(not(any(unix, windows)))]
fn protect_file(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn create_private_file_new(path: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true).mode(0o600);
    options
        .open(path)
        .map_err(|error| format!("failed to create private file {}: {error}", path.display()))
}

#[cfg(windows)]
fn create_private_file_new(path: &Path) -> Result<std::fs::File, String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ,
    };
    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    with_private_security_descriptor(|descriptor| {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let handle = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ,
                &attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            Err(format!(
                "failed to create private Windows file {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ))
        } else {
            Ok(unsafe { std::fs::File::from_raw_handle(handle as _) })
        }
    })
}

#[cfg(not(any(unix, windows)))]
fn create_private_file_new(path: &Path) -> Result<std::fs::File, String> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("failed to create private file {}: {error}", path.display()))
}

pub fn enforce_private_file(path: &Path) -> Result<(), String> {
    protect_directory(parent_dir(path))?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "private state file cannot be a symlink: {}",
            path.display()
        )),
        Ok(metadata) if metadata.is_file() => protect_file(path),
        Ok(_) => Err(format!(
            "private state path is not a file: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to inspect private state file {}: {error}",
            path.display()
        )),
    }
}

/// Read an application-owned private file through one non-following, stable
/// handle. Missing files are represented as `Ok(None)`; insecure or oversized
/// files fail closed before any bytes are consumed.
pub fn read_private_file_bounded(path: &Path, max_bytes: u64) -> Result<Option<Vec<u8>>, String> {
    enforce_private_file(path)?;
    read_regular_file_nofollow_bounded(path, max_bytes)
}

/// Read caller-selected input without following or mutating it. Callers must
/// copy accepted bytes into application-owned private storage before use.
pub fn read_regular_file_nofollow_bounded(
    path: &Path,
    max_bytes: u64,
) -> Result<Option<Vec<u8>>, String> {
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
        options
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to open private state file {}: {error}",
                path.display()
            ));
        }
    };
    let metadata = file.metadata().map_err(|error| {
        format!(
            "failed to inspect private state file {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(format!(
            "private state file is not a regular file within the {max_bytes}-byte limit: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(format!(
                "private state file has multiple hard links: {}",
                path.display()
            ));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: the handle remains owned by `file` and `info` is writable.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
            return Err(format!(
                "failed to inspect private state identity {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        if info.nNumberOfLinks != 1 {
            return Err(format!(
                "private state file has multiple hard links: {}",
                path.display()
            ));
        }
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::io::Read::by_ref(&mut file)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            format!(
                "failed to read private state file {}: {error}",
                path.display()
            )
        })?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!(
            "private state file exceeds the {max_bytes}-byte limit: {}",
            path.display()
        ));
    }
    Ok(Some(bytes))
}

pub fn enforce_private_directory(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| {
        format!(
            "failed to create private directory {}: {error}",
            path.display()
        )
    })?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "private state directory cannot be a symlink: {}",
            path.display()
        )),
        Ok(metadata) if metadata.is_dir() => protect_directory(path),
        Ok(_) => Err(format!(
            "private state path is not a directory: {}",
            path.display()
        )),
        Err(error) => Err(format!(
            "failed to inspect private state directory {}: {error}",
            path.display()
        )),
    }
}

/// Creates a security-sensitive state file with owner-only access before a
/// library such as SQLite reopens it by pathname. The protected parent makes
/// the create/open handoff inaccessible to other local principals.
pub fn prepare_private_file(path: &Path) -> Result<(), String> {
    let parent = parent_dir(path);
    enforce_private_directory(parent)?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "private state file cannot be a symlink: {}",
            path.display()
        )),
        Ok(metadata) if metadata.is_file() => enforce_private_file(path),
        Ok(_) => Err(format!(
            "private state path is not a file: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            drop(create_private_file_new(path)?);
            enforce_private_file(path)
        }
        Err(error) => Err(format!(
            "failed to inspect private state file {}: {error}",
            path.display()
        )),
    }
}

pub fn protect_sqlite_files(path: &Path) -> Result<(), String> {
    prepare_private_file(path)?;
    let base = path.to_string_lossy();
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{base}{suffix}"));
        if sidecar.exists() {
            enforce_private_file(&sidecar)?;
        }
    }
    Ok(())
}

fn sync_file(path: &Path) -> Result<(), String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("failed to sync {}: {error}", path.display()))
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), String> {
    File::open(parent_dir(path))
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "failed to sync parent directory {}: {error}",
                parent_dir(path).display()
            )
        })
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn replace_path_raw(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_IGNORE_MERGE_ERRORS,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let source_wide = wide(source);
    let destination_wide = wide(destination);
    let succeeded = unsafe {
        if destination.exists() {
            ReplaceFileW(
                destination_wide.as_ptr(),
                source_wide.as_ptr(),
                std::ptr::null(),
                REPLACEFILE_IGNORE_MERGE_ERRORS,
                std::ptr::null(),
                std::ptr::null(),
            )
        } else {
            MoveFileExW(
                source_wide.as_ptr(),
                destination_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    if succeeded == 0 {
        return Err(format!(
            "failed to replace {} with {}: {}",
            destination.display(),
            source.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn replace_path_raw(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::rename(source, destination).map_err(|error| {
        format!(
            "failed to replace {} with {}: {error}",
            destination.display(),
            source.display()
        )
    })
}

pub fn replace_file(
    source: &Path,
    destination: &Path,
    backup: Option<&Path>,
) -> Result<(), String> {
    if !source.is_file() {
        return Err(format!(
            "replacement source is not a file: {}",
            source.display()
        ));
    }
    std::fs::create_dir_all(parent_dir(destination)).map_err(|error| {
        format!(
            "failed to create directory {}: {error}",
            parent_dir(destination).display()
        )
    })?;
    protect_directory(parent_dir(destination))?;
    protect_file(source)?;
    sync_file(source)?;

    if let Some(backup_path) = backup.filter(|_| destination.is_file()) {
        std::fs::create_dir_all(parent_dir(backup_path)).map_err(|error| {
            format!(
                "failed to create backup directory {}: {error}",
                parent_dir(backup_path).display()
            )
        })?;
        protect_directory(parent_dir(backup_path))?;
        let backup_temp = temporary_path(backup_path);
        let backup_result = (|| {
            std::fs::copy(destination, &backup_temp).map_err(|error| {
                format!(
                    "failed to prepare backup {}: {error}",
                    backup_path.display()
                )
            })?;
            protect_file(&backup_temp)?;
            sync_file(&backup_temp)?;
            replace_path_raw(&backup_temp, backup_path)?;
            protect_file(backup_path)?;
            sync_parent(backup_path)
        })();
        if backup_result.is_err() {
            let _ = std::fs::remove_file(&backup_temp);
        }
        backup_result?;
    }

    replace_path_raw(source, destination)?;
    protect_file(destination)?;
    sync_parent(destination)
}

pub fn replace_artifact_file(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_file() {
        return Err(format!(
            "replacement source is not a file: {}",
            source.display()
        ));
    }
    std::fs::create_dir_all(parent_dir(destination)).map_err(|error| {
        format!(
            "failed to create directory {}: {error}",
            parent_dir(destination).display()
        )
    })?;
    sync_file(source)?;
    replace_path_raw(source, destination)?;
    sync_parent(destination)
}

pub fn atomic_write(path: &Path, contents: &[u8], backup: Option<&Path>) -> Result<(), String> {
    std::fs::create_dir_all(parent_dir(path)).map_err(|error| {
        format!(
            "failed to create directory {}: {error}",
            parent_dir(path).display()
        )
    })?;
    protect_directory(parent_dir(path))?;
    let temporary = temporary_path(path);
    let result = (|| {
        let mut file = create_private_file_new(&temporary)?;
        file.write_all(contents).map_err(|error| {
            format!(
                "failed to write temporary file {}: {error}",
                temporary.display()
            )
        })?;
        file.flush().map_err(|error| {
            format!(
                "failed to flush temporary file {}: {error}",
                temporary.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "failed to sync temporary file {}: {error}",
                temporary.display()
            )
        })?;
        drop(file);
        protect_file(&temporary)?;
        replace_file(&temporary, path, backup)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub fn atomic_write_artifact_state(path: &Path, contents: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(parent_dir(path)).map_err(|error| {
        format!(
            "failed to create directory {}: {error}",
            parent_dir(path).display()
        )
    })?;
    let temporary = temporary_path(path);
    let result = (|| {
        let mut file = create_private_file_new(&temporary)?;
        file.write_all(contents).map_err(|error| {
            format!(
                "failed to write temporary file {}: {error}",
                temporary.display()
            )
        })?;
        file.flush().map_err(|error| {
            format!(
                "failed to flush temporary file {}: {error}",
                temporary.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "failed to sync temporary file {}: {error}",
                temporary.display()
            )
        })?;
        drop(file);
        protect_file(&temporary)?;
        replace_artifact_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("lsm-persistence-{name}-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn atomic_write_replaces_content_and_preserves_backup() {
        let directory = test_dir("backup");
        let path = directory.join("state.json");
        let backup = directory.join("state.json.bak");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, b"old").unwrap();

        atomic_write(&path, b"new", Some(&backup)).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read(&backup).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_replacement_keeps_existing_destination() {
        let directory = test_dir("failure");
        let path = directory.join("state.json");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, b"old").unwrap();
        let missing = directory.join("missing.tmp");

        assert!(replace_file(&missing, &path, None).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_forces_private_directory_and_file_modes() {
        use std::os::unix::fs::PermissionsExt;

        let directory = test_dir("permissions");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = directory.join("state.json");

        atomic_write(&path, b"private", None).unwrap();

        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn artifact_writes_preserve_shared_directory_and_download_modes() {
        use std::os::unix::fs::PermissionsExt;

        let directory = test_dir("artifact-permissions");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o750)).unwrap();
        let source = directory.join("model.gguf.part");
        let destination = directory.join("model.gguf");
        let artifact_state = directory.join("model.gguf.part.json");
        std::fs::write(&source, b"model").unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o640)).unwrap();

        replace_artifact_file(&source, &destination).unwrap();
        atomic_write_artifact_state(&artifact_state, b"{}").unwrap();

        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o750
        );
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        assert_eq!(
            std::fs::metadata(&artifact_state)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let _ = std::fs::remove_dir_all(directory);
    }
}
