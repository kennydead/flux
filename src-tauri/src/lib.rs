use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WindowEvent,
};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;

const AGENT_IMAGE: &str = "ghcr.io/kennydead/claude-agent-farm/agent:latest";

struct AuthSession {
    stdin: Option<std::process::ChildStdin>,
    child: std::process::Child,
}

type AuthState = Mutex<Option<AuthSession>>;

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

// Rust-side HTTP checks bypass webview CORS restrictions
#[tauri::command]
async fn check_farm_running() -> bool {
    // Check the frontend (5174) — what the iframe actually loads
    tauri::async_runtime::spawn_blocking(|| {
        ureq::get("http://localhost:5174")
            .timeout(std::time::Duration::from_secs(3))
            .call()
            .map(|r| r.status() == 200)
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

#[tauri::command]
async fn check_dashboard_health() -> bool {
    tauri::async_runtime::spawn_blocking(|| {
        ureq::get("http://localhost:8090/health")
            .timeout(std::time::Duration::from_secs(3))
            .call()
            .map(|r| r.status() == 200)
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

#[tauri::command]
async fn check_claude_auth() -> bool {
    let docker = docker_bin();
    let result = tauri::async_runtime::spawn_blocking(move || {
        std::process::Command::new(&docker)
            .args([
                "run", "--rm", "--platform", "linux/amd64",
                "--entrypoint", "",
                "-v", "claudeagentfarm_claude-home:/home/agent",
                AGENT_IMAGE, "claude", "auth", "status", "--json",
            ])
            .env("PATH", augmented_path())
            .output()
    })
    .await;
    match result {
        Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout).contains("\"loggedIn\": true"),
        _ => false,
    }
}

#[tauri::command]
fn start_claude_auth(app: AppHandle) -> Result<String, String> {
    let docker = docker_bin();
    let mut child = std::process::Command::new(&docker)
        .args([
            "run", "--rm", "-i", "--platform", "linux/amd64",
            "--entrypoint", "",
            "-v", "claudeagentfarm_claude-home:/home/agent",
            AGENT_IMAGE, "claude", "auth", "login",
        ])
        .env("PATH", augmented_path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start auth: {e}"))?;

    let stdout = child.stdout.take().ok_or("No stdout")?;
    let stderr = child.stderr.take().ok_or("No stderr")?;
    let stdin = child.stdin.take().ok_or("No stdin")?;

    // Read both streams in a background thread — keep pipes open so the process
    // doesn't get SIGPIPE when it writes the success message after code submission.
    // The channel carries every line; we look for the URL from the main thread.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let tx2 = tx.clone();

    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().flatten() { let _ = tx.send(line); }
    });
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().flatten() { let _ = tx2.send(line); }
    });

    // Wait up to 30 s for a URL to appear on either stream
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut url = String::new();
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(line) => {
                if let Some(u) = line.split_whitespace().find(|s| s.starts_with("https://")) {
                    url = u.to_string();
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => continue,
        }
    }

    if url.is_empty() {
        let _ = child.kill();
        return Err("No login URL found. Try running setup.sh manually.".to_string());
    }

    let state = app.state::<AuthState>();
    *state.lock().unwrap() = Some(AuthSession { stdin: Some(stdin), child });

    Ok(url)
}

#[tauri::command]
fn complete_claude_auth(app: AppHandle, code: String) -> Result<(), String> {
    let state = app.state::<AuthState>();
    let mut lock = state.lock().unwrap();
    let session = lock.as_mut().ok_or("No active auth session")?;

    if let Some(ref mut stdin) = session.stdin {
        writeln!(stdin, "{}", code.trim()).map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())?;
    }
    drop(session.stdin.take()); // close stdin → signals EOF to the process

    // Wait for process to exit — stdout/stderr are still being drained by the threads above
    let _ = session.child.wait();
    *lock = None;
    drop(lock);

    // Verify auth actually succeeded rather than trusting the exit code
    let docker = docker_bin();
    let out = std::process::Command::new(&docker)
        .args([
            "run", "--rm", "--platform", "linux/amd64",
            "--entrypoint", "",
            "-v", "claudeagentfarm_claude-home:/home/agent",
            AGENT_IMAGE, "claude", "auth", "status", "--json",
        ])
        .env("PATH", augmented_path())
        .output()
        .map_err(|e| e.to_string())?;

    if String::from_utf8_lossy(&out.stdout).contains("\"loggedIn\": true") {
        Ok(())
    } else {
        Err("Authentication did not complete. Please try again.".to_string())
    }
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
            Ok(res) => {
                let body: serde_json::Value = res.into_json()
                    .unwrap_or(serde_json::json!({ "valid": false }));
                Ok(body["valid"].as_bool().unwrap_or(false))
            }
            Err(ureq::Error::Status(_, _)) => Ok(false),
            Err(e) => Err(format!("Could not verify license key. Check your internet connection.\n\nDetails: {e}")),
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
async fn reset_setup() -> Result<(), String> {
    let farm = farm_dir();
    let docker = docker_bin();

    // Stop running containers
    let compose_file = farm.join("docker-compose.yml").to_string_lossy().into_owned();
    let _ = tauri::async_runtime::spawn_blocking({
        let docker = docker.clone();
        move || {
            let _ = std::process::Command::new(&docker)
                .args(["compose", "-f", &compose_file, "down"])
                .env("PATH", augmented_path())
                .output();
        }
    }).await;

    // Remove license key
    let _ = std::fs::remove_file(farm.join("logs").join("license_key.txt"));

    // Remove Claude auth volume
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let _ = std::process::Command::new(&docker)
            .args(["volume", "rm", "-f", "claudeagentfarm_claude-home"])
            .env("PATH", augmented_path())
            .output();
    }).await;

    Ok(())
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
        .manage(AuthState::new(None))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let show  = MenuItem::with_id(app, "show",  "Open Flux",      true, None::<&str>)?;
            let stop  = MenuItem::with_id(app, "stop",  "Stop Farm",       true, None::<&str>)?;
            let reset = MenuItem::with_id(app, "reset", "Reset Setup…",    true, None::<&str>)?;
            let quit  = MenuItem::with_id(app, "quit",  "Quit",            true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &stop, &reset, &quit])?;

            TrayIconBuilder::new()
                .menu(&menu)
                .icon(app.default_window_icon().unwrap().clone())
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                            #[cfg(target_os = "macos")]
                            let _ = app.show(); // activates the app in the macOS dock
                        }
                    }
                    "stop" => {
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
                                .env("PATH", augmented_path())
                                .output();
                            // Notify frontend so it can show a stopped state
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                                #[cfg(target_os = "macos")]
                                let _ = app.show();
                                let _ = w.emit("farm-stopped", ());
                            }
                        });
                    }
                    "reset" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                            let _ = w.emit("reset-requested", ());
                        }
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
            check_claude_auth,
            start_claude_auth,
            complete_claude_auth,
            check_farm_running,
            check_dashboard_health,
            reset_setup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application")
}
