use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};
use std::path::PathBuf;

fn farm_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("farm")
}

// GUI apps on Mac inherit a minimal PATH — augment it so credential helpers are found
fn augmented_path() -> String {
    let current = std::env::var("PATH").unwrap_or_default();
    let extras = "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin";
    if current.is_empty() { extras.to_string() } else { format!("{extras}:{current}") }
}

// On Mac, GUI apps may not inherit PATH — check known Docker Desktop locations
fn docker_bin() -> String {
    #[cfg(target_os = "macos")]
    for path in &["/usr/local/bin/docker", "/opt/homebrew/bin/docker"] {
        if std::path::Path::new(path).exists() {
            return path.to_string();
        }
    }
    "docker".to_string()
}

fn python_bin() -> String {
    #[cfg(target_os = "macos")]
    for path in &["/usr/bin/python3", "/usr/local/bin/python3", "/opt/homebrew/bin/python3"] {
        if std::path::Path::new(path).exists() {
            return path.to_string();
        }
    }
    "python3".to_string()
}

#[tauri::command]
fn get_farm_dir() -> String {
    farm_dir().to_string_lossy().into_owned()
}

#[tauri::command]
fn check_license() -> bool {
    farm_dir().join("logs").join("license_key.txt").exists()
}

const LICENSE_ENDPOINT: &str = "https://oywppqdqfcypawrthfox.supabase.co/functions/v1/validate-license";

#[tauri::command]
async fn validate_license_key(key: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        match ureq::post(LICENSE_ENDPOINT)
            .timeout(std::time::Duration::from_secs(8))
            .send_json(serde_json::json!({ "key": key }))
        {
            Ok(res) => Ok(res.status() == 200),
            // Server explicitly rejected the key
            Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => Ok(false),
            // Server unreachable — fail open, backend will validate
            Err(_) => Ok(true),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn save_license_key(key: String) -> Result<(), String> {
    let logs = farm_dir().join("logs");
    std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    std::fs::write(logs.join("license_key.txt"), key.trim()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn extract_resources(app: AppHandle) -> Result<String, String> {
    let farm = farm_dir();
    std::fs::create_dir_all(farm.join("logs")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(farm.join("planning-workspace")).map_err(|e| e.to_string())?;

    if !farm.join(".env").exists() {
        std::fs::write(farm.join(".env"), "").map_err(|e| e.to_string())?;
    }
    if !farm.join("config.yml").exists() {
        std::fs::write(farm.join("config.yml"), "").map_err(|e| e.to_string())?;
    }

    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    for filename in &["docker-compose.yml", "host_bridge.py"] {
        let src = resource_dir.join(filename);
        let dst = farm.join(filename);
        if src.exists() {
            std::fs::copy(&src, &dst)
                .map_err(|e| format!("Failed to copy {filename}: {e}"))?;
        }
    }

    Ok(farm.to_string_lossy().into_owned())
}

#[tauri::command]
async fn run_command(program: String, args: Vec<String>) -> Result<String, String> {
    let bin = if program == "docker" { docker_bin() } else if program == "python3" { python_bin() } else { program };
    tauri::async_runtime::spawn_blocking(move || {
        let output = std::process::Command::new(&bin)
            .args(&args)
            .env("PATH", augmented_path())
            .output()
            .map_err(|e| format!("Failed to run {bin}: {e}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn run_docker_compose(sub_args: Vec<String>) -> Result<String, String> {
    let farm = farm_dir();
    let compose_file = farm.join("docker-compose.yml").to_string_lossy().into_owned();
    let mut full_args = vec!["compose".to_string(), "-f".to_string(), compose_file];
    full_args.extend(sub_args);
    let docker = docker_bin();
    tauri::async_runtime::spawn_blocking(move || {
        let output = std::process::Command::new(&docker)
            .args(&full_args)
            .current_dir(farm_dir())
            .env("PATH", augmented_path())
            .output()
            .map_err(|e| format!("Failed to run docker compose: {e}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn run_detached(program: String, args: Vec<String>) -> Result<(), String> {
    let bin = if program == "python3" { python_bin() } else { program };
    std::process::Command::new(&bin)
        .args(&args)
        .spawn()
        .map_err(|e| format!("Failed to spawn {bin}: {e}"))?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let show = MenuItem::with_id(app, "show", "Open Flux", true, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "Stop Farm", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &stop, &quit])?;

            TrayIconBuilder::new()
                .menu(&menu)
                .icon(app.default_window_icon().unwrap().clone())
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "stop" => {
                        tauri::async_runtime::spawn(async {
                            let farm = farm_dir();
                            let compose_file = farm.join("docker-compose.yml")
                                .to_string_lossy()
                                .into_owned();
                            let docker = docker_bin();
                            let _ = std::process::Command::new(&docker)
                                .args(["compose", "-f", &compose_file, "down"])
                                .current_dir(&farm)
                                .output();
                        });
                    }
                    "quit" => {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let farm = farm_dir();
                            let compose_file = farm.join("docker-compose.yml")
                                .to_string_lossy()
                                .into_owned();
                            let docker = docker_bin();
                            let _ = std::process::Command::new(&docker)
                                .args(["compose", "-f", &compose_file, "down"])
                                .current_dir(&farm)
                                .output();
                            app.exit(0);
                        });
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                window.hide().unwrap();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_farm_dir,
            check_license,
            validate_license_key,
            save_license_key,
            extract_resources,
            run_command,
            run_docker_compose,
            run_detached,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application")
}
