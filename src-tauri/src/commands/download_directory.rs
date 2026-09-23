//! Directory capabilities keep mutations attached to opened directories across awaits.
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use std::{
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub(super) struct DownloadDirectory {
    pub path: PathBuf,
    dir: Arc<Dir>,
}
impl DownloadDirectory {
    // Callers pass the resolved authorized root. Do not canonicalize here: an
    // attacker may have replaced an ancestor with a link since authorization.
    pub fn open(path: &Path, create: bool) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::other("download directory must be absolute"));
        }
        let mut anchor = PathBuf::new();
        let mut names = Vec::new();
        for component in path.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => anchor.push(component.as_os_str()),
                Component::Normal(name) => names.push(name),
                _ => return Err(io::Error::other("invalid download directory component")),
            }
        }
        let dir = Dir::open_ambient_dir(anchor, cap_std::ambient_authority())?;
        Self {
            path: PathBuf::new(),
            dir: Arc::new(dir),
        }
        .descend_names(path.to_path_buf(), names, create)
    }
    fn descend_names(
        &self,
        path: PathBuf,
        names: Vec<&std::ffi::OsStr>,
        create: bool,
    ) -> io::Result<Self> {
        let mut dir = self.dir.try_clone()?;
        for name in names {
            if create {
                match dir.create_dir(name) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
            }
            dir = dir.open_dir_nofollow(name)?;
        }
        Ok(Self {
            path,
            dir: Arc::new(dir),
        })
    }
    pub fn descend(&self, path: &Path, create: bool) -> io::Result<Self> {
        let relative = path.strip_prefix(&self.path).map_err(io::Error::other)?;
        let mut names = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(name) => names.push(name),
                _ => return Err(io::Error::other("download directory escapes capability")),
            }
        }
        self.descend_names(path.to_path_buf(), names, create)
    }
    fn leaf<'a>(&self, path: &'a Path) -> io::Result<&'a std::ffi::OsStr> {
        if path.parent() != Some(self.path.as_path()) {
            return Err(io::Error::other("artifact is outside pinned directory"));
        }
        path.file_name()
            .ok_or_else(|| io::Error::other("artifact has no name"))
    }
    pub fn open_file(&self, path: &Path, append: bool, new: bool) -> io::Result<std::fs::File> {
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create(true)
            .create_new(new)
            .append(append)
            .truncate(false)
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = self.dir.open_with(self.leaf(path)?, &options)?.into_std();
        Self::file_identity(&file)?;
        if !append {
            file.set_len(0)?;
        }
        Ok(file)
    }
    pub fn metadata(&self, path: &Path) -> io::Result<cap_std::fs::Metadata> {
        self.dir.symlink_metadata(self.leaf(path)?)
    }
    pub fn open_owned_file(
        &self,
        path: &Path,
        append: bool,
        expected: &[String],
    ) -> io::Result<std::fs::File> {
        let mut options = OpenOptions::new();
        options
            .write(true)
            .append(append)
            .follow(FollowSymlinks::No);
        let file = self.dir.open_with(self.leaf(path)?, &options)?.into_std();
        if !expected.contains(&Self::file_identity(&file)?) {
            return Err(io::Error::other(
                "download artifact identity changed; write refused",
            ));
        }
        if !append {
            file.set_len(0)?;
        }
        Ok(file)
    }
    pub fn identity(&self, path: &Path) -> io::Result<String> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = self.dir.open_with(self.leaf(path)?, &options)?.into_std();
        Self::file_identity(&file)
    }
    pub fn file_identity(file: &std::fs::File) -> io::Result<String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = file.metadata()?;
            if metadata.nlink() != 1 {
                return Err(io::Error::other("hard-linked download artifact"));
            }
            Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
            };
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if info.nNumberOfLinks != 1 {
                return Err(io::Error::other("hard-linked download artifact"));
            }
            Ok(format!(
                "{}:{}:{}:{}:{}",
                info.dwVolumeSerialNumber,
                info.nFileIndexHigh,
                info.nFileIndexLow,
                info.ftCreationTime.dwHighDateTime,
                info.ftCreationTime.dwLowDateTime
            ))
        }
    }
    pub fn read(&self, path: &Path) -> io::Result<String> {
        use std::io::Read;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = self.dir.open_with(self.leaf(path)?, &options)?;
        let mut text = String::new();
        file.take(1024 * 1024).read_to_string(&mut text)?;
        Ok(text)
    }
    pub fn remove(&self, path: &Path) -> io::Result<()> {
        self.dir.remove_file(self.leaf(path)?)
    }
    pub fn remove_owned(&self, path: &Path, expected: &[String]) -> io::Result<()> {
        // Quarantine before checking the leaf identity so a replaced leaf is never deleted.
        let isolated = self
            .path
            .join(format!(".download-delete.{}.tmp", uuid::Uuid::new_v4()));
        self.dir
            .rename(self.leaf(path)?, &self.dir, self.leaf(&isolated)?)?;
        if self
            .identity(&isolated)
            .is_ok_and(|id| expected.contains(&id))
        {
            return self.remove(&isolated);
        }
        // hard_link is exclusive: restoration must not overwrite a concurrent new leaf.
        self.dir
            .hard_link(self.leaf(&isolated)?, &self.dir, self.leaf(path)?)?;
        self.remove(&isolated)?;
        Err(io::Error::other(
            "download artifact identity changed; deletion refused",
        ))
    }
    pub fn remove_empty_directory(&self, path: &Path) -> io::Result<()> {
        self.dir.remove_dir(self.leaf(path)?)
    }
    pub fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).follow(FollowSymlinks::No);
        self.dir
            .open_with(self.leaf(from)?, &options)?
            .into_std()
            .sync_all()?;
        self.dir
            .rename(self.leaf(from)?, &self.dir, self.leaf(to)?)?;
        #[cfg(unix)]
        // Directory capabilities may use O_PATH on Linux, which cannot be
        // fsynced. Reopen "." relative to the pinned directory for a readable
        // descriptor instead of reopening its replaceable ambient path.
        self.dir.open(".")?.into_std().sync_all()?;
        Ok(())
    }
    #[cfg(test)]
    pub fn atomic_write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let scratch = self.stage_write(bytes)?;
        let result = self.rename(&scratch, path);
        if result.is_err() {
            let _ = self.remove(&scratch);
        }
        result
    }
    pub fn stage_write(&self, bytes: &[u8]) -> io::Result<PathBuf> {
        use std::io::Write;
        let scratch = self
            .path
            .join(format!(".download-state.{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = self.open_file(&scratch, false, true)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            Ok(scratch.clone())
        })();
        if result.is_err() {
            let _ = self.remove(&scratch);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_directory_never_writes_or_removes_from_replacement() {
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("download-capability-{}", uuid::Uuid::new_v4()));
        let original = base.join("original");
        let moved = base.join("moved");
        std::fs::create_dir_all(&original).unwrap();
        let dir = DownloadDirectory::open(&original, false).unwrap();
        let target = original.join("model.part");
        dir.atomic_write(&target, b"owned").unwrap();
        if std::fs::rename(&original, &moved).is_ok() {
            std::fs::create_dir_all(&original).unwrap();
            std::fs::write(&target, b"victim").unwrap();
            dir.atomic_write(&target, b"update").unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"victim");
            dir.remove(&target).unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"victim");
        } // Windows may pin the original directory against rename.
        drop(dir);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn artifact_writes_preserve_shared_directory_and_download_modes() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("download-permissions-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o750)).unwrap();
        let source = directory.join("model.gguf.part");
        let destination = directory.join("model.gguf");
        let artifact_state = directory.join("model.gguf.part.json");
        std::fs::write(&source, b"model").unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o640)).unwrap();

        let dir = DownloadDirectory::open(&directory, false).unwrap();
        dir.rename(&source, &destination).unwrap();
        dir.atomic_write(&artifact_state, b"{}").unwrap();

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
        drop(dir);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn linked_root_and_descendants_remain_rejected_after_authorization() {
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("download-links-{}", uuid::Uuid::new_v4()));
        let original = base.join("authorized");
        let outside = base.join("outside");
        std::fs::create_dir_all(&original).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let dir = DownloadDirectory::open(&original, false).unwrap();
        let alias = original.join("linked");
        std::os::unix::fs::symlink(&outside, &alias).unwrap();
        assert!(DownloadDirectory::open(&alias, false).is_err());
        assert!(dir.descend(&alias, false).is_err());
        assert!(dir.descend(&alias.join("new"), true).is_err());
        assert!(!outside.join("new").exists());
        let legitimate = original.join("real");
        let child = dir.descend(&legitimate, true).unwrap();
        child
            .atomic_write(&legitimate.join("state.json"), b"{}")
            .unwrap();
        drop(child);
        drop(dir);
        std::fs::remove_dir_all(base).unwrap();
    }
}
