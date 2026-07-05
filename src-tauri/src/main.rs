#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod models;
mod utils;
mod commands;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;
use tauri::Manager;
use crate::models::{AppState, WindowState};
use crate::commands::scanner::{scan_models, get_models, delete_model_file, open_model_folder, read_gguf_metadata, scan_engines, get_engines, delete_engine, rename_engine, open_engine_folder, load_app_data, get_cached_scan};
use crate::commands::config::{save_config, load_config, save_window_state, load_window_state, resolve_path};
use crate::commands::server::{generate_server_command, start_server, stop_server, open_browser, test_connection, check_port, get_system_metrics, get_system_health, get_slots, get_metrics};
use crate::commands::download::{browse_modelscope, download_modelscope_files, browse_huggingface, download_huggingface_files, cancel_file_download, pause_file_download, cancel_and_cleanup_download, check_local_file, persist_download_queue, restore_download_queue};
use crate::commands::cluster::{scan_workers_tcp, test_worker, get_worker_info, add_worker, remove_worker, get_workers, find_rpc_server_binary, generate_rpc_launch_cmd, get_cluster_metrics, stop_local_worker, is_local_host, start_local_rpc, get_local_host};
use crate::commands::cluster_network::{detect_usb4_adapters, get_usb4_adapters};
use crate::commands::cluster_mdns::{start_mdns_discovery, stop_mdns_discovery};
use crate::commands::autostart::{enable_autostart, disable_autostart, is_autostart_enabled};
use crate::commands::cluster_ssh::ssh_launch_rpc;
use std::sync::OnceLock;

static NATIVE_START: OnceLock<Instant> = OnceLock::new();

#[tauri::command]
fn get_startup_elapsed() -> u64 {
    NATIVE_START.get().map(|t| t.elapsed().as_millis() as u64).unwrap_or(0)
}

#[tauri::command]
fn show_window(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn main() {
    let native_start = std::time::Instant::now();
    NATIVE_START.set(native_start).ok();
    let default_models: Vec<models::ModelInfo> = vec![];
    let default_engines: Vec<models::EngineInfo> = vec![];

    let data_dir = crate::utils::get_data_dir();
    let config_dir = data_dir.join("configs");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if let Ok(pos) = window.outer_position() {
                    if let Ok(size) = window.outer_size() {
                        let ws = WindowState { x: pos.x, y: pos.y, width: size.width, height: size.height };
                        if let Some(s) = window.try_state::<AppState>() {
                            let config_dir = s.config_dir.lock().unwrap().clone();
                            let _ = std::fs::create_dir_all(&config_dir);
                            if let Ok(json) = serde_json::to_string(&ws) {
                                let _ = std::fs::write(config_dir.join("window_state.json"), json);
                            }
                        }
                    }
                }
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let _t0 = std::time::Instant::now();
            let mut timings: Vec<(String, u64)> = Vec::new();
            let now = || NATIVE_START.get().map(|t| t.elapsed().as_millis() as u64).unwrap_or(0);
            timings.push(("setup-enter".into(), now()));
            use tauri::Emitter;
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
            use tauri::menu::{MenuBuilder, MenuItemBuilder};

            let show = MenuItemBuilder::with_id("show", "显示窗口").build(app.handle())?;
            let quit = MenuItemBuilder::with_id("quit", "退出").build(app.handle())?;
            let menu = MenuBuilder::new(app.handle())
                .item(&show).item(&quit).build()?;

            if let Some(icon) = app.default_window_icon().cloned() {
                TrayIconBuilder::new()
                    .icon(icon)
                    .menu(&menu)
                    .on_menu_event(|app, event| {
                    match event.id().as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => {
                            if let Some(s) = app.try_state::<AppState>() {
                                let config_dir = s.config_dir.lock().unwrap().clone();
                                let path = config_dir.join("instances.json");
                                let _ = std::fs::create_dir_all(&config_dir);
                                let mut global = std::fs::read_to_string(&path).ok()
                                    .and_then(|j| serde_json::from_str::<models::GlobalConfig>(&j).ok())
                                    .unwrap_or(models::GlobalConfig {
                                        instances: HashMap::new(), model_dirs: vec![], engine_dirs: vec![],
                                        default_engine_id: String::new(), running: HashMap::new(),
                                        instance_order: vec![], last_tab: "model-repo".into(), dark_mode: true,
                                        engine_names: HashMap::new(),
                                    });
                                global.running = s.running.lock().unwrap().clone();
                                global.engine_names = s.engine_names.lock().unwrap().clone();
                                let _ = std::fs::write(&path, serde_json::to_string_pretty(&global).unwrap_or_default());
                            }
                            crate::commands::nvml::shutdown();
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app.handle())?;
            }
            timings.push(("setup-tray-done".into(), now()));

            // ── 提前读取配置，通过 initialization_script 注入到前端 ──
            // 绕过 IPC 冷启动延迟：前端 JS 第一行就能读到 window.__INITIAL_CONFIG__
            let data_dir = crate::utils::get_data_dir();
            let config_dir = data_dir.join("configs");
            let config = crate::commands::config::read_config_from_disk(&config_dir);
            let config_json = serde_json::to_string(&config).unwrap_or_else(|_| "{}".to_string());
            timings.push(("setup-config-read".into(), now()));

            // 程序化创建窗口，注入配置数据
            let init_script = format!(
                "window.__INITIAL_CONFIG__ = {};\nif(window.__lsm_setInitialConfig)window.__lsm_setInitialConfig(window.__INITIAL_CONFIG__);",
                config_json
            );
            let window_state = crate::commands::config::read_window_state_from_disk(&config_dir);
            let mut window_builder = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("Llama服务器管理器")
                .inner_size(1280.0, 800.0)
                .min_inner_size(1024.0, 720.0)
                .resizable(true)
                .fullscreen(false)
                .visible(false)
                .background_color(tauri::utils::config::Color(26, 26, 46, 255))
                .initialization_script(&init_script);

            // 恢复窗口位置
            if let Some(ws) = window_state {
                window_builder = window_builder
                    .position(ws.x as f64, ws.y as f64)
                    .inner_size(ws.width as f64, ws.height as f64);
            }

            window_builder.build()?;
            timings.push(("setup-window-built".into(), now()));

            // 同步更新 AppState（进程恢复逻辑在 load_config IPC 中异步执行）
            {
                let st = app.state::<AppState>();
                *st.instances.lock().unwrap() = config.instances.clone();
                *st.engine_names.lock().unwrap() = config.engine_names.clone();
                *st.running.lock().unwrap() = config.running.clone();
            }

            // 为恢复的运行中实例启动健康检查 + 日志恢复 + 指标监控
            for (id, ri) in &config.running {
                let id_hc = id.clone();
                let host = if ri.host == "0.0.0.0" { "localhost".to_string() } else { ri.host.clone() };
                let host2 = host.clone();
                let port = ri.port;
                let pid = ri.pid;
                let app_hc = app.handle().clone();
                let app_reconnect = app.handle().clone();
                let config_dir_clone = config_dir.clone();

                let api_key_health = config.instances.get(&id_hc)
                    .and_then(|c| if c.api_key.is_empty() { None } else { Some(c.api_key.clone()) })
                    .unwrap_or_default();
                let api_key_reconnect = api_key_health.clone();
                std::thread::spawn(move || {
                    crate::commands::server::health_check_loop(&id_hc, &host, port, pid, &api_key_health, app_hc);
                });
                crate::commands::server::reconnect_running_instance(&id, pid, &host2, port, &config_dir_clone, &api_key_reconnect, app_reconnect);
            }

            // Write accumulated timings in a single I/O operation
            let log_dir = crate::utils::get_data_dir().join("configs");
            let log_path = log_dir.join(".startup-log");
            let _ = std::fs::create_dir_all(&log_dir);
            let log_content = timings.iter()
                .map(|(label, ms)| format!("{}: {}ms\n", label, ms))
                .collect::<String>();
            let _ = std::fs::write(&log_path, log_content);
            let _ = app.emit("startup-timing", serde_json::json!({
                "name": "rust-setup", "ms": _t0.elapsed().as_millis()
            }));
            Ok(())
        })
        .manage(AppState {
            models: Mutex::new(default_models),
            engines: Mutex::new(default_engines),
            engine_names: Mutex::new(HashMap::new()),
            instances: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            config_dir: Mutex::new(config_dir),
            cancel_flags: Mutex::new(HashMap::new()),
            pause_flags: Mutex::new(HashMap::new()),
            workers: Mutex::new(Vec::new()),
            usb4_adapters: Mutex::new(Vec::new()),
        })
        .invoke_handler(tauri::generate_handler![
            scan_models, get_models, delete_model_file, open_model_folder, read_gguf_metadata,
            scan_engines, get_engines, delete_engine, rename_engine, open_engine_folder,
            load_app_data, get_cached_scan,
            generate_server_command, start_server, stop_server, open_browser,
            save_config, load_config,
            browse_modelscope, download_modelscope_files,
            browse_huggingface, download_huggingface_files, check_local_file,
            cancel_file_download, pause_file_download, cancel_and_cleanup_download,
            persist_download_queue, restore_download_queue,
            test_connection, check_port,
            get_system_metrics, get_system_health, get_slots, get_metrics,
            save_window_state, load_window_state,
            resolve_path,
            scan_workers_tcp, test_worker, get_worker_info,
            add_worker, remove_worker, get_workers,
            find_rpc_server_binary, generate_rpc_launch_cmd, get_cluster_metrics,
            stop_local_worker, is_local_host, start_local_rpc, get_local_host,
            detect_usb4_adapters, get_usb4_adapters,
            start_mdns_discovery, stop_mdns_discovery,
            ssh_launch_rpc,
            enable_autostart, disable_autostart, is_autostart_enabled,
            get_startup_elapsed,
            show_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use crate::models::InstanceConfig;
    use crate::commands::server::generate_command;

    fn cfg() -> InstanceConfig {
        let mut c = InstanceConfig::default();
        c.model_path = "/test/model.gguf".into();
        c.gpu_layers_auto = false;
        c
    }

    #[test]
    fn test_baseline_command() {
        let cmd = generate_command(&cfg(), "llama-server");
        assert_eq!(cmd[0], "llama-server");
        assert_eq!(cmd[1], "-m");
        assert_eq!(cmd[2], "/test/model.gguf");
        assert!(cmd.iter().any(|a| a == "-ngl"));
        assert!(cmd.iter().any(|a| a == "--host"));
        assert!(cmd.iter().any(|a| a == "--port"));
    }

    #[test]
    fn test_embedding_omits_generation() {
        let mut c = cfg();
        c.embedding = true;
        let cmd = generate_command(&c, "");
        assert!(cmd.iter().any(|a| a == "--embedding"));
        assert!(!cmd.iter().any(|a| a == "--temp"));
        assert!(!cmd.iter().any(|a| a == "--top-k"));
    }

    #[test]
    fn test_parallel_auto() {
        let mut c = cfg();
        c.parallel = -1;
        let cmd = generate_command(&c, "");
        let pos = cmd.iter().position(|a| a == "-np").unwrap();
        assert_eq!(cmd[pos + 1], "-1");
    }

    #[test]
    fn test_fit_flag() {
        let mut c = cfg();
        c.fit = true;
        let cmd = generate_command(&c, "");
        let pos = cmd.iter().position(|a| a == "--fit").unwrap();
        assert_eq!(cmd[pos + 1], "on");
    }

    #[test]
    fn test_advanced_sampling_defaults_omitted() {
        let c = cfg();
        let cmd = generate_command(&c, "");
        assert!(!cmd.iter().any(|a| a == "--mirostat"));
        assert!(!cmd.iter().any(|a| a == "--mirostat-lr"));
        assert!(!cmd.iter().any(|a| a == "--xtc-probability"));
        assert!(!cmd.iter().any(|a| a == "--dry-multiplier"));
    }

    #[test]
    fn test_spec_type_appears() {
        let mut c = cfg();
        c.spec_type = "draft-mtp".into();
        c.draft_tokens = 3;
        let cmd = generate_command(&c, "");
        assert!(cmd.iter().any(|a| a == "--spec-type"));
        assert!(cmd.iter().any(|a| a == "draft-mtp"));
    }

    #[test]
    fn test_rope_params() {
        let mut c = cfg();
        c.rope_scaling = "linear".into();
        c.rope_scale = 2.0;
        let cmd = generate_command(&c, "");
        assert!(cmd.iter().any(|a| a == "--rope-scaling"));
        assert!(cmd.iter().any(|a| a == "--rope-scale"));
    }

    #[test]
    fn test_no_slots() {
        let mut c = cfg();
        c.slots_enabled = false;
        let cmd = generate_command(&c, "");
        assert!(cmd.iter().any(|a| a == "--no-slots"));
    }

    #[test]
    fn test_draft_gpu_default_omitted() {
        let mut c = cfg();
        c.draft_gpu_layers = 99;
        let cmd = generate_command(&c, "");
        assert!(!cmd.iter().any(|a| a == "-ngld"));
    }

    #[test]
    fn test_reasoning_off() {
        let mut c = cfg();
        c.reasoning = "off".into();
        let cmd = generate_command(&c, "");
        let pos = cmd.iter().position(|a| a == "--reasoning").unwrap();
        assert_eq!(cmd[pos + 1], "off");
    }

    #[test]
    fn test_custom_args() {
        let mut c = cfg();
        c.custom_args = vec!["--verbose".into()];
        let cmd = generate_command(&c, "");
        assert!(cmd.iter().any(|a| a == "--verbose"));
    }
}
