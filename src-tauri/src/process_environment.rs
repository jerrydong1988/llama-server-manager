use std::ffi::OsStr;
use std::process::Command;

/// Use host libraries and desktop modules for external engines and system tools.
/// Do not use this for WebKit helpers or a direct re-exec of our bundled binary.
pub(crate) fn external_command(program: impl AsRef<OsStr>) -> Command {
    let command = Command::new(program);
    #[cfg(target_os = "linux")]
    let command = {
        let mut command = command;
        if let Some(app_dir) = std::env::var_os("APPDIR") {
            let app_dir = std::path::Path::new(&app_dir);
            if std::env::current_exe().is_ok_and(|exe| exe.starts_with(app_dir)) {
                remove_bundle_paths(&mut command, app_dir, std::env::vars_os());
            }
        }
        command
    };
    command
}

#[cfg(target_os = "linux")]
fn remove_bundle_paths(
    command: &mut Command,
    app_dir: &std::path::Path,
    environment: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) {
    // Reject an empty or root marker, which would otherwise match every path.
    if !app_dir.is_absolute() || app_dir.parent().is_none() {
        return;
    }
    const PATH_VARIABLES: &[&str] = &[
        "PATH",
        "LD_LIBRARY_PATH",
        "GIO_MODULE_DIR",
        "GIO_EXTRA_MODULES",
        "GI_TYPELIB_PATH",
        "GSETTINGS_SCHEMA_DIR",
        "GTK_DATA_PREFIX",
        "GTK_EXE_PREFIX",
        "GTK_PATH",
        "GTK_IM_MODULE_FILE",
        "GDK_PIXBUF_MODULE_FILE",
        "GDK_PIXBUF_MODULEDIR",
        "XDG_DATA_DIRS",
        "GST_PLUGIN_PATH",
        "GST_PLUGIN_PATH_1_0",
        "GST_PLUGIN_SYSTEM_PATH",
        "GST_PLUGIN_SYSTEM_PATH_1_0",
        "GST_PLUGIN_SCANNER",
        "GST_PLUGIN_SCANNER_1_0",
    ];
    for (name, value) in environment {
        if matches!(name.to_str(), Some("APPDIR" | "APPIMAGE" | "ARGV0" | "OWD")) {
            command.env_remove(name);
            continue;
        }
        if !PATH_VARIABLES.iter().any(|candidate| name == *candidate) {
            continue;
        }
        let paths = std::env::split_paths(&value).collect::<Vec<_>>();
        let host_paths = paths
            .iter()
            .filter(|path| !path.starts_with(app_dir))
            .collect::<Vec<_>>();
        if host_paths.len() == paths.len() {
            continue;
        }
        if host_paths.is_empty() {
            command.env_remove(name);
        } else if let Ok(value) = std::env::join_paths(host_paths) {
            command.env(name, value);
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::Path;

    #[test]
    fn external_process_gets_host_paths_without_losing_gpu_or_session_environment() {
        let environment: Vec<(OsString, OsString)> = [
            ("APPDIR", "/tmp/test.AppDir"),
            ("APPIMAGE", "/home/test/manager.AppImage"),
            ("PATH", "/tmp/test.AppDir/usr/bin:/usr/bin:/bin"),
            (
                "LD_LIBRARY_PATH",
                "/tmp/test.AppDir/usr/lib:/opt/rocm/lib:/opt/cuda/lib64",
            ),
            ("GIO_MODULE_DIR", "/tmp/test.AppDir/usr/lib/gio/modules"),
            ("GIO_EXTRA_MODULES", "/opt/host/gio"),
            ("GDK_PIXBUF_MODULE_FILE", "/tmp/test.AppDir/loaders.cache"),
            ("XDG_DATA_DIRS", "/tmp/test.AppDir/usr/share:/usr/share"),
            ("DISPLAY", ":1"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("CUDA_VISIBLE_DEVICES", "1"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let mut child = Command::new("/usr/bin/env");
        child.env_clear().envs(environment.clone());
        remove_bundle_paths(&mut child, Path::new("/tmp/test.AppDir"), environment);
        let output = child.output().unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        let actual = text
            .lines()
            .filter_map(|line| line.split_once('='))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(actual["PATH"], "/usr/bin:/bin");
        assert_eq!(actual["LD_LIBRARY_PATH"], "/opt/rocm/lib:/opt/cuda/lib64");
        assert!(!actual.contains_key("GIO_MODULE_DIR"));
        assert!(!actual.contains_key("APPDIR"));
        assert!(!actual.contains_key("APPIMAGE"));
        assert!(!actual.contains_key("GDK_PIXBUF_MODULE_FILE"));
        assert_eq!(actual["GIO_EXTRA_MODULES"], "/opt/host/gio");
        assert_eq!(actual["XDG_DATA_DIRS"], "/usr/share");
        assert_eq!(actual["DISPLAY"], ":1");
        assert_eq!(actual["WAYLAND_DISPLAY"], "wayland-0");
        assert_eq!(actual["CUDA_VISIBLE_DEVICES"], "1");
    }

    #[test]
    fn keeps_sibling_paths_empty_entries_and_non_utf8_values() {
        use std::os::unix::ffi::OsStringExt;
        let mut child = Command::new("unused");
        let value = OsString::from_vec(
            b"/tmp/test.AppDir/lib:/tmp/test.AppDir-other/lib::/opt/\xff".to_vec(),
        );
        remove_bundle_paths(
            &mut child,
            Path::new("/tmp/test.AppDir"),
            [("LD_LIBRARY_PATH".into(), value)],
        );
        let (_, value) = child.get_envs().next().unwrap();
        assert_eq!(
            value.unwrap(),
            OsString::from_vec(b"/tmp/test.AppDir-other/lib::/opt/\xff".to_vec())
        );
    }

    #[test]
    fn invalid_bundle_roots_and_host_only_environment_are_unchanged() {
        for root in ["", "/", "relative", "/tmp/test.AppDir"] {
            let mut child = Command::new("unused");
            remove_bundle_paths(
                &mut child,
                Path::new(root),
                [("GIO_MODULE_DIR".into(), "/usr/lib/gio/modules".into())],
            );
            assert_eq!(child.get_envs().count(), 0);
        }
    }
}
