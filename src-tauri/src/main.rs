#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod error;
mod models;
mod persistence;
mod runtime_service;
mod utils;
mod vector_policy;

use crate::commands::autostart::{disable_autostart, enable_autostart, is_autostart_enabled};
use crate::commands::cluster::{
    add_worker, find_rpc_server_binary, generate_rpc_launch_cmd, get_cluster_metrics,
    get_local_host, get_worker_info, get_workers, is_local_host, load_workers, remove_worker,
    scan_workers_tcp, start_local_rpc, stop_local_worker, test_worker,
};
use crate::commands::cluster_mdns::{start_mdns_discovery, stop_mdns_discovery};
use crate::commands::cluster_network::{detect_usb4_adapters, get_usb4_adapters};
use crate::commands::cluster_ssh::ipc::ssh_launch_rpc;
use crate::commands::config::{
    load_config, load_window_state, resolve_path, save_config, save_window_state,
    update_and_persist,
};
use crate::commands::download::{
    browse_huggingface, browse_modelscope, cancel_all_downloads, cancel_and_cleanup_download,
    cancel_file_download, check_local_file, clear_download_tasks_by_status,
    delete_managed_local_file, download_huggingface_files, download_modelscope_files,
    enqueue_download_queue, flush_download_manager_state, get_download_bandwidth_limit,
    get_download_concurrency, get_download_low_priority_throttle, get_download_manager_snapshot,
    get_download_resume_policy, pause_all_downloads, pause_file_download, persist_download_queue,
    process_download_queue, remove_download_queue_entry, reset_download_for_redownload,
    restore_download_queue, resume_all_downloads, resume_download_task,
    set_download_bandwidth_limit, set_download_concurrency, set_download_low_priority_throttle,
    set_download_resume_policy,
};
use crate::commands::engine_capabilities::ipc::probe_engine_capabilities;
use crate::commands::monitoring::get_monitoring_series;
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
    get_telemetry_overview, get_telemetry_session_analysis, get_telemetry_session_detail,
    get_telemetry_session_diagnostics, get_telemetry_session_samples, list_inference_requests,
    list_telemetry_sessions, prune_telemetry,
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
}

fn decide_proxy_quit(proxy_running: bool, _background_service_mode: bool) -> ProxyQuitDecision {
    if !proxy_running {
        ProxyQuitDecision::ExitNow
    } else {
        ProxyQuitDecision::RequestConfirmation
    }
}

fn proxy_quit_inputs(app: &tauri::AppHandle) -> (bool, bool) {
    if let Some(state) = app.try_state::<AppState>() {
        let proxy_config = state.proxy_config.lock().unwrap().clone();
        let runtime_active = proxy_config.runtime_service_enabled
            || proxy_config.enabled
            || !state.running.lock().unwrap().is_empty();
        (runtime_active, proxy_config.runtime_service_enabled)
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

fn finalize_app_exit(app: &tauri::AppHandle, keep_runtime: bool) {
    flush_download_manager_state(app);
    let failures = if keep_runtime {
        Vec::new()
    } else {
        crate::commands::server::terminate_all_servers_for_exit(app)
    };
    if !failures.is_empty() {
        eprintln!(
            "Failed to terminate server processes during shutdown: {}",
            failures.join(", ")
        );
    }
    persist_runtime_state(app);
    if let Err(error) = crate::commands::telemetry::flush_telemetry_writer() {
        eprintln!("Telemetry flush failed during shutdown: {error}");
    }
    crate::commands::nvml::shutdown();
    app.exit(0);
}

fn request_proxy_exit_confirmation(app: &tauri::AppHandle, background_service_mode: bool) {
    let payload = serde_json::json!({
        "reason": "proxy-running",
        "backgroundServiceMode": background_service_mode,
        "background_service_mode": background_service_mode,
    });

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit("proxy-exit-confirmation-requested", payload);
    } else {
        let _ = app.emit("proxy-exit-confirmation-requested", payload);
    }
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
async fn quit_app(app: tauri::AppHandle) -> Result<(), String> {
    crate::commands::proxy::shutdown_proxy_for_app(&app).await?;
    crate::runtime_service::shutdown(true).await?;
    finalize_app_exit(&app, false);
    Ok(())
}

#[tauri::command]
async fn quit_keep_runtime(app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let runtime_enabled = state.proxy_config.lock().unwrap().runtime_service_enabled;
    if !runtime_enabled {
        return Err("独立后台运行时尚未启用".into());
    }
    crate::runtime_service::autostart::enable_runtime_autostart()?;
    crate::runtime_service::set_background_enabled(true).await?;
    persist_runtime_state(&app);
    finalize_app_exit(&app, true);
    Ok(())
}

fn main() {
    if crate::runtime_service::is_runtime_service_invocation() {
        if let Err(error) = crate::runtime_service::configure_runtime_data_dir_from_args() {
            eprintln!("Runtime service configuration failed: {error}");
            std::process::exit(1);
        }
        if let Err(error) = crate::runtime_service::run_runtime_service() {
            eprintln!("Runtime service failed: {error}");
            std::process::exit(1);
        }
        return;
    }

    let native_start = std::time::Instant::now();
    NATIVE_START.set(native_start).ok();
    let default_models: Vec<models::ModelInfo> = vec![];
    let default_engines: Vec<models::EngineInfo> = vec![];

    let data_dir = crate::utils::get_data_dir();
    let config_dir = data_dir.join("configs");
    let initial_config = crate::commands::config::read_config_from_disk(&config_dir);
    let initial_workers = load_workers();

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
                            if let Err(error) = crate::commands::config::persist_window_state(&config_dir, &ws) {
                                eprintln!("Window state persistence failed: {error}");
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
            if let Err(error) = crate::commands::telemetry::initialize_telemetry_storage() {
                eprintln!("Telemetry storage initialization failed: {error}");
            }
            if let Err(error) = crate::commands::model_inventory::initialize_inventory_storage() {
                eprintln!("Model inventory initialization failed: {error}");
            }
            std::thread::spawn(|| {
                if let Err(error) =
                    crate::commands::telemetry::prune_telemetry_storage(14)
                {
                    eprintln!("Telemetry retention cleanup failed: {}", error);
                }
            });
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
                                ProxyQuitDecision::ExitNow => finalize_app_exit(app, false),
                                ProxyQuitDecision::RequestConfirmation => {
                                    request_proxy_exit_confirmation(app, background_service_mode)
                                }
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

            // Restore log capture, metrics, and the single authoritative health monitor.
            let runtime_managed = crate::runtime_service::persisted_managed_instance_ids();
            for (id, ri) in &config.running {
                if !crate::commands::server::register_restored_runtime_instance(app.handle(), id, ri.pid) {
                    continue;
                }
                let pid = ri.pid;
                let app_reconnect = app.handle().clone();
                let config_dir_clone = config_dir.clone();

                let launch_config = ri
                    .launch_config
                    .as_ref()
                    .or_else(|| config.instances.get(id));
                if runtime_managed.contains(id) {
                    app.state::<AppState>()
                        .runtime_managed_instances
                        .lock()
                        .unwrap()
                        .insert(id.clone());
                    crate::commands::server::reconnect_runtime_instance_logs(
                        id,
                        pid,
                        &config_dir_clone,
                        app_reconnect,
                    );
                } else if let Some(launch_config) = launch_config {
                    crate::commands::server::reconnect_running_instance(
                        id,
                        pid,
                        launch_config,
                        &config_dir_clone,
                        app_reconnect,
                    );
                }
            }

            crate::runtime_service::start_app_bridge(app.handle().clone());

            if !crate::runtime_service::manages_instances() && config.proxy_config.enabled {
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
            model_scan_generation: std::sync::atomic::AtomicU64::new(0),
            engine_scan_generation: std::sync::atomic::AtomicU64::new(0),
            engine_names: Mutex::new(HashMap::new()),
            instances: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            starting: Mutex::new(std::collections::HashSet::new()),
            config_dir: Mutex::new(config_dir),
            cancel_flags: Mutex::new(HashMap::new()),
            pause_flags: Mutex::new(HashMap::new()),
            active_downloads: Mutex::new(std::collections::HashSet::new()),
            active_download_paths: Mutex::new(std::collections::HashSet::new()),
            download_queue: Mutex::new(Vec::new()),
            download_active_batches: Mutex::new(std::collections::HashSet::new()),
            download_active_entries: Mutex::new(HashMap::new()),
            download_last_inflight_persist: Mutex::new(Instant::now()),
            download_scheduler_lock: Mutex::new(()),
            download_inflight_lock: Mutex::new(()),
            download_shutting_down: std::sync::atomic::AtomicBool::new(false),
            download_active_file_slots: std::sync::atomic::AtomicUsize::new(0),
            download_slot_notify: std::sync::Arc::new(tokio::sync::Notify::new()),
            download_max_concurrent: Mutex::new(initial_config.download_max_concurrent.max(1)),
            download_bandwidth_limit_bytes_per_sec: Mutex::new(initial_config.download_bandwidth_limit_bytes_per_sec),
            download_low_priority_throttle: Mutex::new(initial_config.download_low_priority_throttle),
            download_bandwidth_limiter: Mutex::new(models::DownloadBandwidthLimiter::default()),
            workers: Mutex::new(initial_workers),
            usb4_adapters: Mutex::new(Vec::new()),
            proxy_config: Mutex::new(initial_config.proxy_config),
            proxy_shutdown: Mutex::new(None),
            proxy_task: Mutex::new(None),
            proxy_bound_addr: Mutex::new(None),
            proxy_last_error: Mutex::new(None),
            proxy_lifecycle_lock: tokio::sync::Mutex::new(()),
            runtime_managed_instances: Mutex::new(std::collections::HashSet::new()),
            restored_runtime_instances: Mutex::new(std::collections::HashSet::new()),
        })
        .invoke_handler(tauri::generate_handler![
            scan_models, get_models, delete_model_file, open_model_folder, read_gguf_metadata,
            scan_engines, get_engines, delete_engine, rename_engine, open_engine_folder,
            probe_engine_capabilities,
            load_app_data, get_cached_scan,
            generate_server_command, start_server, stop_server, open_browser,
            save_config, load_config,
            browse_modelscope, download_modelscope_files,
            browse_huggingface, download_huggingface_files, check_local_file, delete_managed_local_file,
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
            get_system_metrics, get_system_health, get_slots, get_metrics, get_monitoring_series,
            get_telemetry_overview, list_telemetry_sessions, get_telemetry_session_samples, get_telemetry_session_detail, get_telemetry_session_analysis, get_telemetry_session_diagnostics, list_inference_requests, prune_telemetry,
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
            quit_keep_runtime,
            crate::runtime_service::get_runtime_service_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use crate::commands::server::generate_command;
    use crate::models::{InstanceConfig, ProxyConfig};
    use crate::vector_policy::{classify_model_workload, ModelWorkload};
    use std::path::Path;

    fn cfg() -> InstanceConfig {
        InstanceConfig {
            model_path: "/test/model.gguf".into(),
            gpu_layers_auto: false,
            ..InstanceConfig::default()
        }
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
    fn embedding_command_rejects_all_inference_only_flags() {
        let mut c = cfg();
        c.embedding = true;
        c.spec_type = "draft-mtp".into();
        c.draft_model_path = "/test/draft.gguf".into();
        c.cache_type_draft_k = "q8_0".into();
        c.cache_type_draft_v = "q8_0".into();
        c.cache_prompt = false;
        c.keep = 32;
        c.cache_reuse = 64;
        c.ctx_checkpoints = 16;
        c.swa_full = true;
        c.context_shift = true;
        c.chat_template = "chatml".into();
        c.temp = 1.5;
        c.mmproj_path = "/test/mmproj.gguf".into();
        c.ui_config_file = "/test/ui.json".into();
        c.ui_config = "{}".into();
        c.ui_mcp_proxy = true;
        c.agent = true;
        c.slot_save_path = "/test/slots".into();
        c.models_dir = "/test/models".into();
        c.models_preset = "preset".into();
        c.models_max = 8;
        c.image_min_tokens = 32;
        c.image_max_tokens = 512;
        c.mtmd_batch_max_tokens = 2048;
        c.tags = "vision".into();
        c.media_path = "/test/media".into();
        c.tools = "tool.json".into();
        c.slot_prompt_similarity = 0.9;
        let cmd = generate_command(&c, "");

        for forbidden in [
            "--spec-type",
            "--temp",
            "--draft-model",
            "-ctkd",
            "-ctvd",
            "--no-cache-prompt",
            "--keep",
            "--cache-reuse",
            "-ctxcp",
            "--swa-full",
            "--context-shift",
            "--chat-template",
            "--mmproj",
            "--ui-config-file",
            "--ui-config",
            "--ui-mcp-proxy",
            "--agent",
            "--slot-save-path",
            "--tools",
            "-sps",
            "--models-dir",
            "--models-preset",
            "--models-max",
            "--image-min-tokens",
            "--image-max-tokens",
            "--mtmd-batch-max-tokens",
            "--tags",
            "--media-path",
            "-cram",
            "-cms",
            "--prefill-assistant",
            "--models-autoload",
        ] {
            assert!(
                !cmd.iter().any(|arg| arg == forbidden),
                "leaked {forbidden}"
            );
        }
    }

    #[test]
    fn classifies_vector_workloads_without_matching_directories() {
        assert_eq!(
            classify_model_workload(Some("bert"), Path::new("C:/models/model.gguf")),
            ModelWorkload::Embedding
        );
        assert_eq!(
            classify_model_workload(None, Path::new("C:/models/bge-reranker-v2.gguf")),
            ModelWorkload::Reranker
        );
        assert_eq!(
            classify_model_workload(None, Path::new("C:/embedding/model.gguf")),
            ModelWorkload::Inference
        );
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
    fn test_sampling_defaults_are_omitted() {
        let c = cfg();
        let cmd = generate_command(&c, "");
        for omitted in [
            "-n",
            "--temp",
            "--top-k",
            "--top-p",
            "--repeat-penalty",
            "--seed",
            "--min-p",
            "--presence-penalty",
            "--frequency-penalty",
            "--repeat-last-n",
            "--mirostat",
            "--xtc-probability",
            "--xtc-threshold",
            "--dynatemp-range",
            "--dynatemp-exp",
            "--typical-p",
            "--dry-multiplier",
            "--dry-base",
            "--dry-allowed-length",
            "--dry-penalty-last-n",
            "--adaptive-target",
            "--adaptive-decay",
            "--top-n-sigma",
        ] {
            assert!(!cmd.iter().any(|arg| arg == omitted), "leaked {omitted}");
        }
    }

    #[test]
    fn common_sampling_tuning_emits_only_changed_values() {
        let mut c = cfg();
        c.temp = 0.6;
        c.top_k = 20;
        c.top_p = 0.7;
        c.repeat_last_n = 0;
        let cmd = generate_command(&c, "");

        for (flag, value) in [
            ("--temp", "0.6"),
            ("--top-k", "20"),
            ("--top-p", "0.7"),
            ("--repeat-last-n", "0"),
        ] {
            assert!(
                cmd.windows(2).any(|args| args == [flag, value]),
                "missing tuned value {flag} {value}"
            );
        }
        for omitted in [
            "-n",
            "--repeat-penalty",
            "--seed",
            "--min-p",
            "--presence-penalty",
            "--frequency-penalty",
            "--mirostat",
            "--xtc-probability",
            "--dynatemp-range",
            "--typical-p",
            "--dry-multiplier",
            "--adaptive-target",
            "--top-n-sigma",
        ] {
            assert!(!cmd.iter().any(|arg| arg == omitted), "leaked {omitted}");
        }
    }

    #[test]
    fn meaningful_zero_sampling_overrides_are_preserved() {
        let mut c = cfg();
        c.n_predict = 0;
        c.temp = 0.0;
        c.top_k = 0;
        c.top_p = 1.0;
        c.min_p = 0.0;
        c.repeat_last_n = 0;
        c.adaptive_target = 0.0;
        c.top_n_sigma = 0.0;
        let cmd = generate_command(&c, "");

        for (flag, value) in [
            ("-n", "0"),
            ("--temp", "0"),
            ("--top-k", "0"),
            ("--top-p", "1"),
            ("--min-p", "0"),
            ("--repeat-last-n", "0"),
            ("--adaptive-target", "0"),
            ("--top-n-sigma", "0"),
        ] {
            assert!(
                cmd.windows(2).any(|args| args == [flag, value]),
                "missing {flag} {value}"
            );
        }
    }

    #[test]
    fn advanced_sampling_dependents_follow_their_controller() {
        let mut disabled = cfg();
        disabled.xtc_threshold = 0.4;
        disabled.dynatemp_exp = 2.0;
        disabled.dry_base = 2.0;
        disabled.dry_allowed_length = 8;
        disabled.dry_penalty_last_n = 128;
        disabled.dry_sequence_breaker = "none".into();
        disabled.adaptive_decay = 0.5;
        let disabled_cmd = generate_command(&disabled, "");
        for omitted in [
            "--xtc-threshold",
            "--dynatemp-exp",
            "--dry-base",
            "--dry-allowed-length",
            "--dry-penalty-last-n",
            "--dry-sequence-breaker",
            "--adaptive-decay",
        ] {
            assert!(
                !disabled_cmd.iter().any(|arg| arg == omitted),
                "leaked disabled dependent {omitted}"
            );
        }

        let mut enabled = cfg();
        enabled.mirostat = 2;
        enabled.mirostat_lr = 0.2;
        enabled.mirostat_ent = 6.0;
        enabled.xtc_probability = 0.5;
        enabled.xtc_threshold = 0.2;
        enabled.dynatemp_range = 0.4;
        enabled.dynatemp_exp = 1.5;
        enabled.typical_p = 0.9;
        enabled.dry_multiplier = 0.8;
        enabled.dry_base = 2.0;
        enabled.dry_allowed_length = 4;
        enabled.dry_penalty_last_n = 96;
        enabled.dry_sequence_breaker = "none".into();
        enabled.adaptive_target = 0.4;
        enabled.adaptive_decay = 0.8;
        enabled.top_n_sigma = 1.2;
        let enabled_cmd = generate_command(&enabled, "");
        for (flag, value) in [
            ("--mirostat", "2"),
            ("--mirostat-lr", "0.2"),
            ("--mirostat-ent", "6"),
            ("--xtc-probability", "0.5"),
            ("--xtc-threshold", "0.2"),
            ("--dynatemp-range", "0.4"),
            ("--dynatemp-exp", "1.5"),
            ("--typical-p", "0.9"),
            ("--dry-multiplier", "0.8"),
            ("--dry-base", "2"),
            ("--dry-allowed-length", "4"),
            ("--dry-penalty-last-n", "96"),
            ("--dry-sequence-breaker", "none"),
            ("--adaptive-target", "0.4"),
            ("--adaptive-decay", "0.8"),
            ("--top-n-sigma", "1.2"),
        ] {
            assert!(
                enabled_cmd.windows(2).any(|args| args == [flag, value]),
                "missing enabled sampling option {flag} {value}"
            );
        }
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
    fn proxy_quit_prompts_even_when_keep_alive_is_enabled() {
        assert_eq!(
            super::decide_proxy_quit(true, true),
            super::ProxyQuitDecision::RequestConfirmation
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
