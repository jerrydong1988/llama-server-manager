/// Manages application auto-start on login.
/// Windows: HKCU\Software\Microsoft\Windows\CurrentVersion\Run registry entry.
/// macOS:   ~/Library/LaunchAgents/com.llama-server-manager.plist
/// Linux:   ~/.config/autostart/llama-server-manager.desktop
use std::path::PathBuf;

#[cfg(target_os = "windows")]
const APP_NAME: &str = "LlamaServerManager";

fn exe_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[cfg(any(target_os = "linux", test))]
fn linux_autostart_executable(
    appimage: Option<&std::ffi::OsStr>,
    current_exe: &std::path::Path,
) -> PathBuf {
    appimage
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| current_exe.to_path_buf())
}

#[cfg(any(target_os = "linux", test))]
fn desktop_exec_value(executable: &std::path::Path) -> Result<String, String> {
    let mut quoted = String::from("\"");
    for ch in executable.to_string_lossy().chars() {
        if ch.is_control() {
            return Err("登录自启动路径包含 XDG Exec 不支持的控制字符".into());
        }
        match ch {
            '\\' => quoted.push_str("\\\\\\\\"),
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(ch);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

#[allow(dead_code)]
fn home_dir() -> Result<PathBuf, String> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("USERPROFILE").map(PathBuf::from))
        .map_err(|_| "无法获取 HOME 目录".to_string())
}

#[cfg(any(target_os = "linux", test))]
fn desktop_data_arguments(data: Option<&std::path::Path>) -> Result<String, String> {
    data.map(|path| desktop_exec_value(path).map(|quoted| format!(" --data-dir {quoted}")))
        .transpose()
        .map(Option::unwrap_or_default)
}

pub fn enable_autostart() -> Result<(), String> {
    let exe = exe_path()?;

    #[cfg(target_os = "windows")]
    {
        let mut quoted =
            crate::runtime_service::autostart::windows_quote_argument(std::path::Path::new(&exe));
        if let Some(data) = crate::utils::data_dir_override() {
            quoted.push_str(" --data-dir ");
            quoted.push_str(&crate::runtime_service::autostart::windows_quote_argument(
                data,
            ));
        }
        crate::runtime_service::autostart::set_windows_run_entry(APP_NAME, &quoted)?;
    }

    #[cfg(target_os = "macos")]
    {
        let escaped = exe
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;");
        let data_arguments = crate::utils::data_dir_override()
            .map(|data| {
                format!(
                    "<string>--data-dir</string><string>{}</string>",
                    data.to_string_lossy()
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;")
                        .replace('"', "&quot;")
                        .replace('\'', "&apos;")
                )
            })
            .unwrap_or_default();
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
    <key>Label</key><string>com.llama-server-manager</string>
    <key>ProgramArguments</key><array><string>{}</string>{}</array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><false/>
</dict></plist>"#,
            escaped, data_arguments
        );
        let agents = home_dir()?.join("Library/LaunchAgents");
        std::fs::create_dir_all(&agents).map_err(|e| e.to_string())?;
        std::fs::write(agents.join("com.llama-server-manager.plist"), plist)
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        let current_exe = PathBuf::from(&exe);
        let executable =
            linux_autostart_executable(std::env::var_os("APPIMAGE").as_deref(), &current_exe);
        let arguments = desktop_data_arguments(crate::utils::data_dir_override())?;
        let desktop = format!(
            "[Desktop Entry]\nType=Application\nName=Llama Server Manager\nExec={}{}\n\
             Hidden=false\nNoDisplay=false\nX-GNOME-Autostart-enabled=true\n",
            desktop_exec_value(&executable)?,
            arguments
        );
        let dir = home_dir()?.join(".config/autostart");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("llama-server-manager.desktop"), desktop)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

pub fn disable_autostart() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        crate::runtime_service::autostart::delete_windows_run_entry(APP_NAME)?;
    }

    #[cfg(target_os = "macos")]
    {
        let path = home_dir()?.join("Library/LaunchAgents/com.llama-server-manager.plist");
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("删除登录启动项失败: {error}")),
        }
    }

    #[cfg(target_os = "linux")]
    {
        let path = home_dir()?.join(".config/autostart/llama-server-manager.desktop");
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("删除登录启动项失败: {error}")),
        }
    }

    if is_autostart_enabled()? {
        return Err("系统仍报告登录自启动已启用".into());
    }
    Ok(())
}

pub fn is_autostart_enabled() -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        let registered = crate::runtime_service::autostart::read_windows_run_entry(APP_NAME)?;
        let executable = exe_path()?;
        Ok(registered.is_some_and(|command| {
            windows_registration_matches(&command, &executable, crate::utils::data_dir_override())
        }))
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(dir) = home_dir() {
            Ok(dir
                .join("Library/LaunchAgents/com.llama-server-manager.plist")
                .exists())
        } else {
            Ok(false)
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(dir) = home_dir() {
            let path = dir.join(".config/autostart/llama-server-manager.desktop");
            let current_exe = PathBuf::from(exe_path()?);
            let executable =
                linux_autostart_executable(std::env::var_os("APPIMAGE").as_deref(), &current_exe);
            let expected = format!(
                "Exec={}{}",
                desktop_exec_value(&executable)?,
                desktop_data_arguments(crate::utils::data_dir_override())?
            );
            Ok(std::fs::read_to_string(path)
                .map(|contents| contents.lines().any(|line| line == expected))
                .unwrap_or(false))
        } else {
            Ok(false)
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_registration_matches(
    command: &str,
    executable: &str,
    data: Option<&std::path::Path>,
) -> bool {
    !executable.is_empty()
        && command.contains(executable)
        && data.map_or(true, |data| {
            command.contains(&format!(
                "--data-dir {}",
                crate::runtime_service::autostart::windows_quote_argument(data)
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_registration_keeps_executable_and_data_identity() {
        let executable = r"C:\测试 程序\llama-server-manager.exe";
        let data = std::path::Path::new(r"C:\测试 数据");
        let command = format!("\"{executable}\" --data-dir \"{}\"", data.display());
        assert!(windows_registration_matches(&command, executable, Some(data)));
        assert!(windows_registration_matches(&command, executable, None));
        assert!(!windows_registration_matches(&command, r"C:\old\app.exe", Some(data)));
        assert!(!windows_registration_matches(
            &command, executable, Some(std::path::Path::new(r"C:\other")),
        ));
        assert!(!windows_registration_matches("", "", None));
    }

    #[test]
    fn appimage_autostart_uses_persistent_bundle_path() {
        let executable = linux_autostart_executable(
            Some(std::ffi::OsStr::new("/home/Jerry Apps/Llama.AppImage")),
            std::path::Path::new("/tmp/.mount_Llama/usr/bin/llama-server-manager"),
        );
        assert_eq!(executable, PathBuf::from("/home/Jerry Apps/Llama.AppImage"));
    }

    #[test]
    fn desktop_exec_quotes_reserved_characters_and_field_codes() {
        assert_eq!(
            desktop_exec_value(std::path::Path::new("/home/A B/app$`\\\"100%")).unwrap(),
            "\"/home/A B/app\\\\$\\\\`\\\\\\\\\\\\\"100%%\""
        );
    }

    #[test]
    fn desktop_exec_rejects_control_characters() {
        assert!(desktop_exec_value(std::path::Path::new("/home/app\nname")).is_err());
        assert!(desktop_exec_value(std::path::Path::new("/home/app\tname")).is_err());
    }

    #[test]
    fn desktop_autostart_retains_the_isolated_data_directory() {
        assert_eq!(desktop_data_arguments(None).unwrap(), "");
        assert_eq!(
            desktop_data_arguments(Some(std::path::Path::new("/home/Test Data"))).unwrap(),
            " --data-dir \"/home/Test Data\""
        );
    }
}

// IPC compatibility boundary: legacy command internals keep their existing error flow,
// while every registered command serializes a stable AppError object.
#[allow(dead_code, unused_imports, unused_mut)] // Tauri references adapters through generated macros.
pub mod ipc {
    async fn run<R: Send + 'static>(operation: fn() -> Result<R, String>) -> crate::error::AppResult<R> {
        tauri::async_runtime::spawn_blocking(operation)
            .await
            .map_err(|error| crate::error::AppError::new("AUTOSTART_WORKER", error.to_string(), true))?
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn enable_autostart() -> crate::error::AppResult<()> {
        run(super::enable_autostart).await
    }

    #[tauri::command]
    pub async fn disable_autostart() -> crate::error::AppResult<()> {
        run(super::disable_autostart).await
    }

    #[tauri::command]
    pub async fn is_autostart_enabled() -> crate::error::AppResult<bool> {
        run(super::is_autostart_enabled).await
    }
}
