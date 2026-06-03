#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod models;
mod utils;
mod commands;

use std::collections::HashMap;
use std::sync::Mutex;
use tauri::Manager;
use crate::models::{AppState, WindowState};
use crate::commands::scanner::{scan_models, get_models, delete_model_file, open_model_folder, read_gguf_metadata, scan_engines, get_engines, delete_engine, open_engine_folder};
use crate::commands::config::{save_config, load_config, save_window_state, load_window_state, resolve_path};
use crate::commands::server::{generate_server_command, start_server, stop_server, open_browser, test_connection, check_port};
use crate::commands::download::{browse_modelscope, download_modelscope_files, cancel_file_download, pause_file_download, cancel_and_cleanup_download};

fn main() {
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
                                    });
                                global.running = s.running.lock().unwrap().clone();
                                let _ = std::fs::write(&path, serde_json::to_string_pretty(&global).unwrap_or_default());
                            }
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
            Ok(())
        })
        .manage(AppState {
            models: Mutex::new(default_models),
            engines: Mutex::new(default_engines),
            instances: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            config_dir: Mutex::new(config_dir),
            cancel_flags: Mutex::new(HashMap::new()),
            pause_flags: Mutex::new(HashMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            scan_models, get_models, delete_model_file, open_model_folder, read_gguf_metadata,
            scan_engines, get_engines, delete_engine, open_engine_folder,
            generate_server_command, start_server, stop_server, open_browser,
            save_config, load_config,
            browse_modelscope, download_modelscope_files,
            cancel_file_download, pause_file_download, cancel_and_cleanup_download,
            test_connection, check_port,
            save_window_state, load_window_state,
            resolve_path,
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
