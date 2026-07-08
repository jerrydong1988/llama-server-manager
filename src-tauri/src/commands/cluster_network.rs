use crate::models::{AppState, Usb4Adapter};
use tauri::State;

#[tauri::command]
pub async fn detect_usb4_adapters(state: State<'_, AppState>) -> Result<Vec<Usb4Adapter>, String> {
    #[cfg(target_os = "windows")]
    {
        let adapters = detect_usb4_windows().unwrap_or_default();
        if let Ok(mut a) = state.usb4_adapters.lock() {
            *a = adapters.clone();
        }
        return Ok(adapters);
    }

    #[cfg(not(target_os = "windows"))]
    {
        // macOS/Linux stub: return empty for now.
        let _ = state;
        Ok(Vec::new())
    }
}

#[cfg(target_os = "windows")]
fn detect_usb4_windows() -> Result<Vec<Usb4Adapter>, String> {
    use std::process::Command;

    let ps_script = r#"
$ErrorActionPreference="SilentlyContinue"
$adapters = Get-NetAdapter | ForEach-Object {
    $ips = @(Get-NetIPAddress -InterfaceIndex $_.ifIndex -AddressFamily IPv4 -EA SilentlyContinue | Select-Object -ExpandProperty IPAddress)
    @{ name=$_.Name; ifIndex=$_.ifIndex; description=$_.InterfaceDescription; status=$_.Status; ips=$ips }
}
$adapters | ConvertTo-Json -Compress
"#;

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", ps_script])
        .output()
        .map_err(|e| format!("PowerShell 执行失败: {}", e))?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }

    #[derive(serde::Deserialize)]
    struct RawAdapter {
        name: String,
        #[serde(rename = "ifIndex")]
        if_index: Option<u32>,
        description: Option<String>,
        status: Option<String>,
        ips: Option<Vec<String>>,
    }

    let raw: Vec<RawAdapter> = serde_json::from_str(stdout.trim()).unwrap_or_default();

    let include_keyword = regex_lite::Regex::new(
        r"(?i)(usb|rndis|remote.ndis|usb.ethernet|thunderbolt|usb4|p2p.network)",
    )
    .ok();
    let exclude_keyword = regex_lite::Regex::new(
        r"(?i)(vmware|hyper-v|loopback|bluetooth|wireless|wi-fi|wlan|tap|vpn|virtualbox)",
    )
    .ok();

    let filtered: Vec<Usb4Adapter> = raw
        .into_iter()
        .filter(|a| {
            let desc = a.description.as_deref().unwrap_or("");
            if let Some(inc) = &include_keyword {
                if !inc.is_match(desc) {
                    return false;
                }
            }
            if let Some(exc) = &exclude_keyword {
                if exc.is_match(desc) {
                    return false;
                }
            }
            a.status.as_deref() != Some("Disabled")
        })
        .map(|a| Usb4Adapter {
            name: a.name,
            if_index: a.if_index.unwrap_or(0),
            description: a.description.unwrap_or_default(),
            status: a.status.unwrap_or_default(),
            ip: a.ips.and_then(|i| i.into_iter().next()),
        })
        .collect();

    Ok(filtered)
}

#[tauri::command]
pub async fn get_usb4_adapters(state: State<'_, AppState>) -> Result<Vec<Usb4Adapter>, String> {
    if let Ok(a) = state.usb4_adapters.lock() {
        Ok(a.clone())
    } else {
        Ok(Vec::new())
    }
}
