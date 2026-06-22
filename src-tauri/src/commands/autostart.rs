/// 程序开机自启动管理。
/// Windows: 注册表 HKCU\Software\Microsoft\Windows\CurrentVersion\Run
/// macOS:   ~/Library/LaunchAgents/com.llama-server-manager.plist
/// Linux:   ~/.config/autostart/llama-server-manager.desktop

use std::path::PathBuf;

const APP_NAME: &str = "LlamaServerManager";

fn exe_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
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
        std::process::Command::new("reg")
            .args(["add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                   "/v", APP_NAME, "/t", "REG_SZ", "/d", &exe, "/f"])
            .output().map_err(|e| format!("注册表写入失败: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
    <key>Label</key><string>com.llama-server-manager</string>
    <key>ProgramArguments</key><array><string>{}</string></array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><false/>
</dict></plist>"#, exe);
        let agents = home_dir()?.join("Library/LaunchAgents");
        std::fs::create_dir_all(&agents).map_err(|e| e.to_string())?;
        std::fs::write(agents.join("com.llama-server-manager.plist"), plist)
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        let desktop = format!(
            "[Desktop Entry]\nType=Application\nName=Llama Server Manager\nExec={}\n\
             Hidden=false\nNoDisplay=false\nX-GNOME-Autostart-enabled=true\n", exe);
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
            .args(["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                   "/v", APP_NAME, "/f"])
            .output().map_err(|e| format!("注册表删除失败: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(dir) = home_dir() {
            let _ = std::fs::remove_file(
                dir.join("Library/LaunchAgents/com.llama-server-manager.plist"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(dir) = home_dir() {
            let _ = std::fs::remove_file(
                dir.join(".config/autostart/llama-server-manager.desktop"));
        }
    }

    Ok(())
}

#[tauri::command]
pub fn is_autostart_enabled() -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        let out = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                   "/v", APP_NAME])
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
            Ok(dir.join("Library/LaunchAgents/com.llama-server-manager.plist").exists())
        } else {
            Ok(false)
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(dir) = home_dir() {
            Ok(dir.join(".config/autostart/llama-server-manager.desktop").exists())
        } else {
            Ok(false)
        }
    }
}
