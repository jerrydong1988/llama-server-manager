#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod models;
mod utils;

use crate::commands::autostart::{disable_autostart, enable_autostart, is_autostart_enabled};
use crate::commands::cluster::{
    add_worker, find_rpc_server_binary, generate_rpc_launch_cmd, get_cluster_metrics,
    get_local_host, get_worker_info, get_workers, is_local_host, remove_worker, scan_workers_tcp,
    start_local_rpc, stop_local_worker, test_worker,
};
use crate::commands::cluster_mdns::{start_mdns_discovery, stop_mdns_discovery};
use crate::commands::cluster_network::{detect_usb4_adapters, get_usb4_adapters};
use crate::commands::cluster_ssh::ssh_launch_rpc;
use crate::commands::config::{
    load_config, load_window_state, resolve_path, save_config, save_window_state,
    update_and_persist,
};
use crate::commands::download::{
    browse_huggingface, browse_modelscope, cancel_all_downloads, cancel_and_cleanup_download,
    cancel_file_download, check_local_file, clear_download_tasks_by_status,
    download_huggingface_files, download_modelscope_files, enqueue_download_queue,
    flush_download_manager_state, get_download_bandwidth_limit, get_download_concurrency,
    get_download_low_priority_throttle, get_download_manager_snapshot, get_download_resume_policy,
    pause_all_downloads, pause_file_download, persist_download_queue, process_download_queue,
    remove_download_queue_entry, reset_download_for_redownload, restore_download_queue,
    resume_all_downloads, resume_download_task, set_download_bandwidth_limit,
    set_download_concurrency, set_download_low_priority_throttle, set_download_resume_policy,
};
use crate::commands::proxy::{
    get_proxy_config, get_proxy_status, list_proxy_targets, restart_proxy, save_proxy_config,
    start_proxy, stop_proxy, test_proxy_route,
};
use crate::commands::scanner::{
    delete_engine, delete_model_file, get_cached_scan, get_engines, get_models, load_app_data,
    open_engine_folder, open_model_folder, read_gguf_metadata, rename_engine, scan_engines,
    scan_models,
};
use crate::commands::server::{
    check_port, generate_server_command, get_metrics, get_slots, get_system_health,
    get_system_metrics, open_browser, start_server, stop_server, test_connection,
};
use crate::commands::telemetry::{
    get_telemetry_overview, get_telemetry_session_analysis, get_telemetry_session_diagnostics,
    get_telemetry_session_samples, list_inference_requests, list_telemetry_sessions,
    prune_telemetry,
};
use crate::models::{AppState, WindowState};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Instant;
use tauri::{Emitter, Manager};

static NATIVE_START: OnceLock<Instant> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProxyQuitDecision {
    ExitNow,
    RequestConfirmation,
    KeepAlive,
}

fn decide_proxy_quit(proxy_running: bool, background_service_mode: bool) -> ProxyQuitDecision {
    if !proxy_running {
        ProxyQuitDecision::ExitNow
    } else if background_service_mode {
        ProxyQuitDecision::KeepAlive
    } else {
        ProxyQuitDecision::RequestConfirmation
    }
}

fn proxy_quit_inputs(app: &tauri::AppHandle) -> (bool, bool) {
    if let Some(state) = app.try_state::<AppState>() {
        let proxy_running = state.proxy_shutdown.lock().unwrap().is_some();
        let background_service_mode = state.proxy_config.lock().unwrap().background_service_mode;
        (proxy_running, background_service_mode)
    } else {
        (false, false)
    }
}

fn persist_runtime_state(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        let running = state.running.lock().unwrap().clone();
        let engine_names = state.engine_names.lock().unwrap().clone();
        let proxy_config = state.proxy_config.lock().unwrap().clone();
        let _ = update_and_persist(&state, |global| {
            global.running = running;
            global.engine_names = engine_names;
            global.proxy_config = proxy_config;
        });
    }
}

fn finalize_app_exit(app: &tauri::AppHandle) {
    persist_runtime_state(app);
    flush_download_manager_state(app);
    crate::commands::nvml::shutdown();
    app.exit(0);
}

fn request_proxy_exit_confirmation(app: &tauri::AppHandle) {
    let payload = serde_json::json!({
        "reason": "proxy-running",
    });

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit("proxy-exit-confirmation-requested", payload);
    } else {
        let _ = app.emit("proxy-exit-confirmation-requested", payload);
    }
}

fn keep_proxy_alive_in_tray(app: &tauri::AppHandle) {
    persist_runtime_state(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let _ = app.emit(
        "proxy-background-keepalive",
        serde_json::json!({
            "reason": "proxy-background-service-mode",
        }),
    );
}

#[tauri::command]
fn get_startup_elapsed() -> u64 {
    NATIVE_START
        .get()
        .map(|t| t.elapsed().as_millis() as u64)
        .unwrap_or(0)
}

#[tauri::command]
fn show_window(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn hide_window(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    finalize_app_exit(&app);
}

fn main() {
    let native_start = std::time::Instant::now();
    NATIVE_START.set(native_start).ok();
    let default_models: Vec<models::ModelInfo> = vec![];
    let default_engines: Vec<models::EngineInfo> = vec![];

    let data_dir = crate::utils::get_data_dir();
    let config_dir = data_dir.join("configs");
    let initial_config = crate::commands::config::read_config_from_disk(&config_dir);

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
                            let (proxy_running, background_service_mode) = proxy_quit_inputs(app);
                            match decide_proxy_quit(proxy_running, background_service_mode) {
                                ProxyQuitDecision::ExitNow => finalize_app_exit(app),
                                ProxyQuitDecision::RequestConfirmation => request_proxy_exit_confirmation(app),
                                ProxyQuitDecision::KeepAlive => keep_proxy_alive_in_tray(app),
                            }
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

            // Read config early and inject it into the frontend through initialization_script.
            // Avoid IPC cold-start delay; frontend JS can read window.__INITIAL_CONFIG__ immediately.
            let data_dir = crate::utils::get_data_dir();
            let config_dir = data_dir.join("configs");
            let config = crate::commands::config::read_config_from_disk(&config_dir);
            let config_json = serde_json::to_string(&config).unwrap_or_else(|_| "{}".to_string());
            timings.push(("setup-config-read".into(), now()));

            // Create the window programmatically and inject config data.
            let init_script = format!(
                "window.__INITIAL_CONFIG__ = {};\nif(window.__lsm_setInitialConfig)window.__lsm_setInitialConfig(window.__INITIAL_CONFIG__);",
                config_json
            );
            let window_state = crate::commands::config::read_window_state_from_disk(&config_dir);
            let mut window_builder = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("Llama 服务器管理器")
                .inner_size(1280.0, 800.0)
                .min_inner_size(1024.0, 720.0)
                .resizable(true)
                .fullscreen(false)
                .visible(false)
                .background_color(tauri::utils::config::Color(26, 26, 46, 255))
                .initialization_script(&init_script);

            // Restore window position.
            if let Some(ws) = window_state {
                window_builder = window_builder
                    .position(ws.x as f64, ws.y as f64)
                    .inner_size(ws.width as f64, ws.height as f64);
            }

            window_builder.build()?;
            timings.push(("setup-window-built".into(), now()));

            // Synchronize AppState; process restoration also runs asynchronously in load_config IPC.
            {
                let st = app.state::<AppState>();
                *st.instances.lock().unwrap() = config.instances.clone();
                *st.engine_names.lock().unwrap() = config.engine_names.clone();
                *st.running.lock().unwrap() = config.running.clone();
                *st.proxy_config.lock().unwrap() = config.proxy_config.clone();
            }

            // Start health checks, log recovery, and metrics monitoring for restored running instances.
            for (id, ri) in &config.running {
                if !crate::commands::server::register_restored_runtime_instance(app.handle(), id, ri.pid) {
                    continue;
                }
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

            if config.proxy_config.enabled {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = crate::commands::proxy::start_proxy_for_app(app_handle).await;
                });
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
            active_downloads: Mutex::new(std::collections::HashSet::new()),
            download_queue: Mutex::new(Vec::new()),
            download_active_batches: Mutex::new(std::collections::HashSet::new()),
            download_active_entries: Mutex::new(HashMap::new()),
            download_max_concurrent: Mutex::new(initial_config.download_max_concurrent.max(1)),
            download_bandwidth_limit_bytes_per_sec: Mutex::new(initial_config.download_bandwidth_limit_bytes_per_sec),
            download_low_priority_throttle: Mutex::new(initial_config.download_low_priority_throttle),
            download_bandwidth_limiter: Mutex::new(models::DownloadBandwidthLimiter::default()),
            workers: Mutex::new(Vec::new()),
            usb4_adapters: Mutex::new(Vec::new()),
            proxy_config: Mutex::new(initial_config.proxy_config),
            proxy_shutdown: Mutex::new(None),
            proxy_bound_addr: Mutex::new(None),
            proxy_last_error: Mutex::new(None),
            restored_runtime_instances: Mutex::new(std::collections::HashSet::new()),
        })
        .invoke_handler(tauri::generate_handler![
            scan_models, get_models, delete_model_file, open_model_folder, read_gguf_metadata,
            scan_engines, get_engines, delete_engine, rename_engine, open_engine_folder,
            load_app_data, get_cached_scan,
            generate_server_command, start_server, stop_server, open_browser,
            save_config, load_config,
            browse_modelscope, download_modelscope_files,
            browse_huggingface, download_huggingface_files, check_local_file,
            enqueue_download_queue, remove_download_queue_entry, clear_download_tasks_by_status, process_download_queue,
            cancel_file_download, pause_file_download, cancel_and_cleanup_download, reset_download_for_redownload,
            persist_download_queue, restore_download_queue,
            get_download_resume_policy, set_download_resume_policy, resume_download_task, resume_all_downloads,
            pause_all_downloads, cancel_all_downloads,
            set_download_concurrency, get_download_concurrency,
            set_download_bandwidth_limit, get_download_bandwidth_limit,
            set_download_low_priority_throttle, get_download_low_priority_throttle,
            get_download_manager_snapshot,
            test_connection, check_port,
            get_system_metrics, get_system_health, get_slots, get_metrics,
            get_telemetry_overview, list_telemetry_sessions, get_telemetry_session_samples, get_telemetry_session_analysis, get_telemetry_session_diagnostics, list_inference_requests, prune_telemetry,
            get_proxy_config, save_proxy_config, get_proxy_status, list_proxy_targets, test_proxy_route, start_proxy, stop_proxy, restart_proxy,
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
            hide_window,
            quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use crate::commands::server::generate_command;
    use crate::models::{InstanceConfig, ProxyConfig};

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

    #[test]
    fn proxy_quit_prompts_when_route_is_running_without_keep_alive() {
        assert_eq!(
            super::decide_proxy_quit(true, false),
            super::ProxyQuitDecision::RequestConfirmation
        );
    }

    #[test]
    fn proxy_quit_keeps_process_alive_when_keep_alive_is_enabled() {
        assert_eq!(
            super::decide_proxy_quit(true, true),
            super::ProxyQuitDecision::KeepAlive
        );
    }

    #[test]
    fn proxy_quit_exits_when_proxy_is_not_running() {
        assert_eq!(
            super::decide_proxy_quit(false, false),
            super::ProxyQuitDecision::ExitNow
        );
    }

    #[test]
    fn proxy_config_disables_background_keep_alive_by_default() {
        assert!(!ProxyConfig::default().background_service_mode);
    }
}
