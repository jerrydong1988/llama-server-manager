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
fn desktop_exec_value(executable: &std::path::Path) -> String {
    let mut quoted = String::from("\"");
    for ch in executable.to_string_lossy().chars() {
        match ch {
            '\\' | '"' | '`' | '$' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

#[allow(dead_code)]
fn home_dir() -> Result<PathBuf, String> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("USERPROFILE").map(PathBuf::from))
        .map_err(|_| "无法获取 HOME 目录".to_string())
}

#[tauri::command]
pub fn enable_autostart() -> Result<(), String> {
    let exe = exe_path()?;

    #[cfg(target_os = "windows")]
    {
        let quoted = format!("\"{}\"", exe);
        std::process::Command::new("reg")
            .args([
                "add",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                APP_NAME,
                "/t",
                "REG_SZ",
                "/d",
                &quoted,
                "/f",
            ])
            .output()
            .map_err(|e| format!("注册表写入失败: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        let escaped = exe
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;");
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
    <key>Label</key><string>com.llama-server-manager</string>
    <key>ProgramArguments</key><array><string>{}</string></array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><false/>
</dict></plist>"#,
            escaped
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
        let desktop = format!(
            "[Desktop Entry]\nType=Application\nName=Llama Server Manager\nExec={}\n\
             Hidden=false\nNoDisplay=false\nX-GNOME-Autostart-enabled=true\n",
            desktop_exec_value(&executable)
        );
        let dir = home_dir()?.join(".config/autostart");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("llama-server-manager.desktop"), desktop)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub fn disable_autostart() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                APP_NAME,
                "/f",
            ])
            .output()
            .map_err(|e| format!("注册表删除失败: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(dir) = home_dir() {
            let _ = std::fs::remove_file(
                dir.join("Library/LaunchAgents/com.llama-server-manager.plist"),
            );
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(dir) = home_dir() {
            let _ =
                std::fs::remove_file(dir.join(".config/autostart/llama-server-manager.desktop"));
        }
    }

    Ok(())
}

#[tauri::command]
pub fn is_autostart_enabled() -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        let out = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                APP_NAME,
            ])
            .output();
        match out {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                Ok(stdout.contains(APP_NAME) && stdout.contains(&exe_path()?))
            }
            Err(_) => Ok(false),
        }
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
            let expected = format!("Exec={}", desktop_exec_value(&executable));
            Ok(std::fs::read_to_string(path)
                .map(|contents| contents.lines().any(|line| line == expected))
                .unwrap_or(false))
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            desktop_exec_value(std::path::Path::new("/home/A B/app$`\\\"100%")),
            "\"/home/A B/app\\$\\`\\\\\\\"100%%\""
        );
    }
}
