#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::path::Path;
use std::path::PathBuf;

#[cfg(target_os = "windows")]
const WINDOWS_VALUE_NAME: &str = "LlamaServerManagerRuntime";
#[cfg(target_os = "macos")]
const MACOS_LABEL: &str = "com.llama.manager.runtime";
#[cfg(target_os = "linux")]
const LINUX_SERVICE_NAME: &str = "llama-server-manager-runtime.service";

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "无法获取当前用户目录".to_string())
}

fn runtime_executable() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    if let Some(appimage) = std::env::var_os("APPIMAGE").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(appimage));
    }
    std::env::current_exe().map_err(|error| format!("无法获取程序路径: {error}"))
}

#[cfg(target_os = "windows")]
fn windows_quote_argument(value: &Path) -> String {
    let mut quoted = String::from("\"");
    let mut backslashes = 0_usize;
    for character in value.to_string_lossy().chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                quoted.push(character);
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(target_os = "windows")]
fn windows_command(executable: &Path, data_dir: &Path) -> String {
    format!(
        "{} --runtime-service --autostart --runtime-data-dir {}",
        windows_quote_argument(executable),
        windows_quote_argument(data_dir),
    )
}

#[cfg(target_os = "windows")]
fn windows_wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(target_os = "windows")]
fn windows_registry_error(action: &str, code: u32) -> String {
    format!(
        "{action}: {}",
        std::io::Error::from_raw_os_error(code as i32)
    )
}

#[cfg(target_os = "windows")]
fn set_windows_autostart(command: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    let subkey = windows_wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let value_name = windows_wide(WINDOWS_VALUE_NAME);
    let value = windows_wide(command);
    let mut key = std::ptr::null_mut();
    let create_result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if create_result != ERROR_SUCCESS {
        return Err(windows_registry_error(
            "注册后台运行时登录启动失败",
            create_result,
        ));
    }
    let set_result = unsafe {
        RegSetValueExW(
            key,
            value_name.as_ptr(),
            0,
            REG_SZ,
            value.as_ptr().cast(),
            (value.len() * std::mem::size_of::<u16>()) as u32,
        )
    };
    unsafe {
        RegCloseKey(key);
    }
    if set_result != ERROR_SUCCESS {
        return Err(windows_registry_error(
            "写入后台运行时登录启动项失败",
            set_result,
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn delete_windows_autostart() -> Result<(), String> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, HKEY_CURRENT_USER};

    let subkey = windows_wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let value_name = windows_wide(WINDOWS_VALUE_NAME);
    let result =
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), value_name.as_ptr()) };
    if matches!(result, ERROR_SUCCESS | ERROR_FILE_NOT_FOUND) {
        Ok(())
    } else {
        Err(windows_registry_error(
            "删除后台运行时登录启动项失败",
            result,
        ))
    }
}

#[cfg(target_os = "windows")]
fn read_windows_autostart() -> Result<Option<String>, String> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

    let subkey = windows_wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let value_name = windows_wide(WINDOWS_VALUE_NAME);
    let mut byte_len = 0_u32;
    let size_result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut byte_len,
        )
    };
    if size_result == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if size_result != ERROR_SUCCESS {
        return Err(windows_registry_error(
            "读取后台运行时登录启动项失败",
            size_result,
        ));
    }
    let mut value = vec![0_u16; (byte_len as usize).div_ceil(std::mem::size_of::<u16>())];
    let read_result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &mut byte_len,
        )
    };
    if read_result != ERROR_SUCCESS {
        return Err(windows_registry_error(
            "读取后台运行时登录启动项失败",
            read_result,
        ));
    }
    let length = value
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(value.len());
    Ok(Some(String::from_utf16_lossy(&value[..length])))
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(target_os = "linux")]
fn systemd_quote(value: &Path) -> Result<String, String> {
    let mut quoted = String::from("\"");
    for ch in value.to_string_lossy().chars() {
        match ch {
            '\n' | '\r' | '\0' => {
                return Err("后台运行时路径包含 systemd ExecStart 不支持的控制字符".into());
            }
            '\\' | '"' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

#[cfg(target_os = "linux")]
fn desktop_exec_quote(value: &Path) -> Result<String, String> {
    let mut quoted = String::from("\"");
    for ch in value.to_string_lossy().chars() {
        match ch {
            '\n' | '\r' | '\0' => {
                return Err("后台运行时路径包含 XDG Exec 不支持的控制字符".into());
            }
            '\\' => quoted.push_str("\\\\\\\\"),
            '"' | '$' | '`' => {
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

#[cfg(target_os = "linux")]
fn systemd_user_available() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "show-environment"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "linux")]
fn systemd_user_unit_enabled() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "is-enabled", "--quiet", LINUX_SERVICE_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn enable_runtime_autostart() -> Result<(), String> {
    let executable = runtime_executable()?;
    let data_dir = crate::utils::get_data_dir();

    #[cfg(target_os = "windows")]
    {
        set_windows_autostart(&windows_command(&executable, &data_dir))?;
    }

    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::PermissionsExt;
        let agents = home_dir()?.join("Library/LaunchAgents");
        std::fs::create_dir_all(&agents)
            .map_err(|error| format!("创建 LaunchAgents 目录失败: {error}"))?;
        let path = agents.join(format!("{MACOS_LABEL}.plist"));
        let executable = xml_escape(&executable.to_string_lossy());
        let data_dir = xml_escape(&data_dir.to_string_lossy());
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{MACOS_LABEL}</string>
  <key>ProgramArguments</key><array>
    <string>{executable}</string><string>--runtime-service</string><string>--autostart</string>
    <string>--runtime-data-dir</string><string>{data_dir}</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>ProcessType</key><string>Background</string>
</dict></plist>
"#
        );
        crate::persistence::atomic_write(&path, plist.as_bytes(), None)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("保护 LaunchAgent 配置失败: {error}"))?;
    }

    #[cfg(target_os = "linux")]
    {
        if systemd_user_available() {
            let unit_dir = home_dir()?.join(".config/systemd/user");
            std::fs::create_dir_all(&unit_dir)
                .map_err(|error| format!("创建 systemd 用户目录失败: {error}"))?;
            let unit_path = unit_dir.join(LINUX_SERVICE_NAME);
            let unit = format!(
                "[Unit]\nDescription=Llama Server Manager background runtime\nStartLimitIntervalSec=60\nStartLimitBurst=3\n\n\
                 [Service]\nType=exec\nExecStart={} --runtime-service --autostart --runtime-data-dir {}\n\
                 Restart=on-failure\nRestartSec=2\n\n\
                 [Install]\nWantedBy=default.target\n",
                systemd_quote(&executable)?,
                systemd_quote(&data_dir)?
            );
            crate::persistence::atomic_write(&unit_path, unit.as_bytes(), None)?;
            let reload = std::process::Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status()
                .map_err(|error| format!("刷新 systemd 用户服务失败: {error}"))?;
            let enable = std::process::Command::new("systemctl")
                .args(["--user", "enable", LINUX_SERVICE_NAME])
                .status()
                .map_err(|error| format!("启用 systemd 用户服务失败: {error}"))?;
            if !reload.success() || !enable.success() {
                return Err("systemd 用户服务注册失败".into());
            }
            if !systemd_user_unit_enabled() {
                return Err("systemd 用户服务未确认处于 enabled 状态".into());
            }
            let legacy_desktop =
                home_dir()?.join(".config/autostart/llama-server-manager-runtime.desktop");
            match std::fs::remove_file(legacy_desktop) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("清理旧 XDG 启动项失败: {error}")),
            }
        } else {
            if executable.to_string_lossy().contains('=') {
                return Err("后台运行时程序路径包含 XDG Exec 不支持的等号".into());
            }
            let autostart_dir = home_dir()?.join(".config/autostart");
            std::fs::create_dir_all(&autostart_dir)
                .map_err(|error| format!("创建 XDG Autostart 目录失败: {error}"))?;
            let desktop = format!(
                "[Desktop Entry]\nType=Application\nName=Llama Server Manager Runtime\n\
                 Exec={} --runtime-service --autostart --runtime-data-dir {}\nHidden=false\nNoDisplay=true\n\
                 X-GNOME-Autostart-enabled=true\n",
                desktop_exec_quote(&executable)?,
                desktop_exec_quote(&data_dir)?
            );
            crate::persistence::atomic_write(
                &autostart_dir.join("llama-server-manager-runtime.desktop"),
                desktop.as_bytes(),
                None,
            )?;
            let stale_unit = home_dir()?
                .join(".config/systemd/user")
                .join(LINUX_SERVICE_NAME);
            match std::fs::remove_file(stale_unit) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("清理旧 systemd 用户服务失败: {error}")),
            }
        }
    }

    if !is_runtime_autostart_enabled()? {
        return Err("系统未确认后台运行时登录启动已启用".into());
    }
    Ok(())
}

pub fn disable_runtime_autostart() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        delete_windows_autostart()?;
    }

    #[cfg(target_os = "macos")]
    {
        let path = home_dir()?
            .join("Library/LaunchAgents")
            .join(format!("{MACOS_LABEL}.plist"));
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &domain])
            .arg(&path)
            .status();
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("删除 LaunchAgent 失败: {error}")),
        }
    }

    #[cfg(target_os = "linux")]
    {
        if systemd_user_available() {
            let disable = std::process::Command::new("systemctl")
                .args(["--user", "disable", LINUX_SERVICE_NAME])
                .status()
                .map_err(|error| format!("禁用 systemd 用户服务失败: {error}"))?;
            if !disable.success() && systemd_user_unit_enabled() {
                return Err("systemd 用户服务仍处于 enabled 状态".into());
            }
        }
        let paths = [
            home_dir()?
                .join(".config/systemd/user")
                .join(LINUX_SERVICE_NAME),
            home_dir()?.join(".config/autostart/llama-server-manager-runtime.desktop"),
        ];
        for path in paths {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("删除后台运行时启动项失败: {error}")),
            }
        }
        if systemd_user_available() {
            let reload = std::process::Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status()
                .map_err(|error| format!("刷新 systemd 用户服务失败: {error}"))?;
            if !reload.success() {
                return Err("刷新 systemd 用户服务失败".into());
            }
        }
    }

    if is_runtime_autostart_enabled()? {
        return Err("系统仍报告后台运行时登录启动已启用".into());
    }
    Ok(())
}

pub fn is_runtime_autostart_enabled() -> Result<bool, String> {
    let executable = runtime_executable()?;
    let data_dir = crate::utils::get_data_dir();

    #[cfg(target_os = "windows")]
    {
        read_windows_autostart().map(|registered| {
            registered.as_deref() == Some(&windows_command(&executable, &data_dir))
        })
    }

    #[cfg(target_os = "macos")]
    {
        let path = home_dir()?
            .join("Library/LaunchAgents")
            .join(format!("{MACOS_LABEL}.plist"));
        Ok(std::fs::read_to_string(path)
            .map(|contents| {
                contents.contains(&xml_escape(&executable.to_string_lossy()))
                    && contents.contains(&xml_escape(&data_dir.to_string_lossy()))
            })
            .unwrap_or(false))
    }

    #[cfg(target_os = "linux")]
    {
        let unit = home_dir()?
            .join(".config/systemd/user")
            .join(LINUX_SERVICE_NAME);
        let desktop = home_dir()?.join(".config/autostart/llama-server-manager-runtime.desktop");
        let systemd_matches = std::fs::read_to_string(unit)
            .map(|contents| {
                systemd_quote(&executable).is_ok_and(|expected| contents.contains(&expected))
                    && systemd_quote(&data_dir).is_ok_and(|expected| contents.contains(&expected))
            })
            .unwrap_or(false);
        let desktop_matches = std::fs::read_to_string(desktop)
            .map(|contents| {
                desktop_exec_quote(&executable).is_ok_and(|expected| contents.contains(&expected))
                    && desktop_exec_quote(&data_dir)
                        .is_ok_and(|expected| contents.contains(&expected))
            })
            .unwrap_or(false);
        Ok(desktop_matches
            || (systemd_user_available() && systemd_matches && systemd_user_unit_enabled()))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::xml_escape;
    #[cfg(target_os = "linux")]
    use super::{desktop_exec_quote, systemd_quote};
    #[cfg(target_os = "windows")]
    use super::{windows_command, windows_quote_argument};
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    use std::path::Path;

    #[cfg(target_os = "macos")]
    #[test]
    fn launch_agent_values_are_xml_escaped() {
        assert_eq!(
            xml_escape("Llama & <GPU> \"A\" 'B'"),
            "Llama &amp; &lt;GPU&gt; &quot;A&quot; &apos;B&apos;"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_runtime_command_quotes_paths() {
        assert_eq!(
            windows_command(
                Path::new(r"C:\Program Files\Llama\manager.exe"),
                Path::new(r"C:\Users\Jerry\App Data\Llama"),
            ),
            r#""C:\Program Files\Llama\manager.exe" --runtime-service --autostart --runtime-data-dir "C:\Users\Jerry\App Data\Llama""#
        );
        assert_eq!(
            windows_quote_argument(Path::new("C:\\runtime data\\")),
            r#""C:\runtime data\\""#
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn systemd_command_escapes_specifiers() {
        assert_eq!(
            systemd_quote(Path::new("/opt/llama%manager")).unwrap(),
            "\"/opt/llama%%manager\""
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_command_applies_both_string_and_exec_escaping() {
        assert_eq!(
            desktop_exec_quote(Path::new("/opt/llama$%`\\manager")).unwrap(),
            "\"/opt/llama\\\\$%%\\\\`\\\\\\\\manager\""
        );
    }
}
