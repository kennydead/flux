use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WindowEvent,
};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;

const FARM_VERSION: &str = "latest";
const IMAGE_REGISTRY: &str = "ghcr.io/kennydead/claude-agent-farm";

fn agent_image() -> String {
    format!("{}/agent:{}", IMAGE_REGISTRY, FARM_VERSION)
}

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
    #[cfg(target_os = "windows")]
    for cmd in &["python3", "python"] {
        let ok = silent_command(cmd)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok { return cmd.to_string(); }
    }
    "python3".to_string()
}

// Suppress the black console window on Windows for all subprocess calls
fn silent_command(program: &str) -> std::process::Command {
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
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
        silent_command(&docker)
            .args([
                "run", "--rm", "--platform", "linux/amd64",
                "--entrypoint", "",
                "-v", "claudeagentfarm_claude-home:/home/agent",
                agent_image().as_str(), "claude", "auth", "status", "--json",
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
    let mut child = silent_command(&docker)
        .args([
            "run", "--rm", "-i", "--platform", "linux/amd64",
            "--entrypoint", "",
            "-v", "claudeagentfarm_claude-home:/home/agent",
            agent_image().as_str(), "claude", "auth", "login",
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
    let out = silent_command(&docker)
        .args([
            "run", "--rm", "--platform", "linux/amd64",
            "--entrypoint", "",
            "-v", "claudeagentfarm_claude-home:/home/agent",
            agent_image().as_str(), "claude", "auth", "status", "--json",
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

#[tauri::command]
fn check_setup_complete() -> bool {
    farm_dir().join("logs").join("setup_complete").exists()
}

#[tauri::command]
fn mark_setup_complete() -> Result<(), String> {
    let logs = farm_dir().join("logs");
    std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    std::fs::write(logs.join("setup_complete"), "1").map_err(|e| e.to_string())
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
        std::fs::write(farm.join("config.yml"), "\
github:\n\
  repo:\n\
  base_branch: main\n\
agent:\n\
  workspace_dir: ./planning-workspace\n\
dashboard:\n\
  base_url: http://localhost:8090\n\
  project_id:\n\
").map_err(|e| e.to_string())?;
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
async fn check_wsl_installed() -> bool {
    #[cfg(not(target_os = "windows"))]
    return true;

    #[cfg(target_os = "windows")]
    return tauri::async_runtime::spawn_blocking(|| {
        silent_command("wsl")
            .args(["--status"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false);
}

#[tauri::command]
fn install_wsl() {
    // Opens an elevated PowerShell that runs wsl --install --no-distribution
    #[cfg(target_os = "windows")]
    let _ = silent_command("powershell")
        .args([
            "-Command",
            "Start-Process powershell -ArgumentList '-NoExit','-Command','wsl --install --no-distribution' -Verb RunAs",
        ])
        .spawn();
}

#[tauri::command]
async fn check_docker_running() -> bool {
    let bin = docker_bin();
    tauri::async_runtime::spawn_blocking(move || {
        silent_command(&bin)
            .args(["info", "--format", "{{.ServerVersion}}"])
            .env("PATH", augmented_path())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

#[tauri::command]
async fn run_command(program: String, args: Vec<String>) -> Result<String, String> {
    let bin = if program == "docker" { docker_bin() } else if program == "python3" { python_bin() } else { program };
    tauri::async_runtime::spawn_blocking(move || {
        let output = silent_command(&bin)
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

/// Returns true if a new image was downloaded, false if already up to date.
#[tauri::command]
async fn pull_image(image: String) -> Result<bool, String> {
    let bin = docker_bin();
    tauri::async_runtime::spawn_blocking(move || {
        let output = silent_command(&bin)
            .args(["pull", &image])
            .env("PATH", augmented_path())
            .output()
            .map_err(|e| format!("Failed to pull {image}: {e}"))?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{stdout}{stderr}");
            Ok(combined.contains("Downloaded newer image") || combined.contains("Pull complete"))
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
        let output = silent_command(&docker)
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
async fn stop_farm() -> Result<(), String> {
    let farm = farm_dir();
    let compose_file = farm.join("docker-compose.yml").to_string_lossy().into_owned();
    let docker = docker_bin();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = silent_command(&docker)
            .args(["compose", "-f", &compose_file, "down"])
            .env("PATH", augmented_path())
            .output();
    }).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn soft_reset() -> Result<(), String> {
    let farm = farm_dir();
    let docker = docker_bin();

    // Stop running containers — preserve all volumes (credentials + database)
    let compose_file = farm.join("docker-compose.yml").to_string_lossy().into_owned();
    let _ = tauri::async_runtime::spawn_blocking({
        let docker = docker.clone();
        move || {
            let _ = silent_command(&docker)
                .args(["compose", "-f", &compose_file, "down"])
                .env("PATH", augmented_path())
                .output();
        }
    }).await;

    // Remove farm directory (clears setup_complete flag and old scripts)
    let _ = std::fs::remove_dir_all(&farm);

    Ok(())
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
            let _ = silent_command(&docker)
                .args(["compose", "-f", &compose_file, "down"])
                .env("PATH", augmented_path())
                .output();
        }
    }).await;

    // Remove license key
    let _ = std::fs::remove_file(farm.join("logs").join("license_key.txt"));

    // Remove Claude auth volume
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let _ = silent_command(&docker)
            .args(["volume", "rm", "-f", "claudeagentfarm_claude-home"])
            .env("PATH", augmented_path())
            .output();
    }).await;

    Ok(())
}

#[tauri::command]
async fn confirm_quit(app: AppHandle) {
    let farm = farm_dir();
    let compose_file = farm.join("docker-compose.yml").to_string_lossy().into_owned();
    let docker = docker_bin();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let _ = silent_command(&docker)
            .args(["compose", "-f", &compose_file, "down"])
            .env("PATH", augmented_path())
            .output();
    }).await;
    app.exit(0);
}

#[tauri::command]
fn get_farm_version() -> &'static str {
    FARM_VERSION
}

#[tauri::command]
async fn check_for_update() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| {
        let url = "https://api.github.com/repos/kennydead/flux/releases/latest";
        let resp = ureq::get(url)
            .set("User-Agent", "flux-app")
            .timeout(std::time::Duration::from_secs(8))
            .call()
            .ok()?;
        let json: serde_json::Value = resp.into_json().ok()?;
        let latest = json["tag_name"].as_str()?.to_string();
        if latest != FARM_VERSION { Some(latest) } else { None }
    })
    .await
    .unwrap_or(None)
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    if enabled { mgr.enable() } else { mgr.disable() }
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn run_detached(program: String, args: Vec<String>) -> Result<(), String> {
    let bin = if program == "python3" { python_bin() } else { program };
    silent_command(&bin)
        .args(&args)
        .spawn()
        .map_err(|e| format!("Failed to spawn {bin}: {e}"))?;
    Ok(())
}

fn open_folder_in_terminal(path: &str) {
    #[cfg(target_os = "macos")]
    {
        let safe = path.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "tell application \"Terminal\" to activate\ntell application \"Terminal\" to do script \"cd \\\"{safe}\\\"\""
        );
        let _ = silent_command("osascript").args(["-e", &script]).spawn();
    }

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd.exe")
            .args(["/c", "start", "Project Terminal", "cmd", "/k", &format!("cd /d \"{}\"", path)])
            .spawn();
    }
}

fn open_discuss_terminal(farm: &std::path::Path, project_id: &str) {
    let image = agent_image();
    let workspace = farm.join("planning-workspace");
    let _ = std::fs::create_dir_all(&workspace);

    let mut docker_args: Vec<String> = vec![
        "docker".into(), "run".into(), "-it".into(), "--rm".into(),
        "--platform".into(), "linux/amd64".into(),
        "--network".into(), "claudeagentfarm_default".into(),
        "-e".into(), "DASHBOARD_URL=http://dashboard-backend:8090".into(),
        "-v".into(), "claudeagentfarm_claude-home:/home/agent".into(),
        "-v".into(), format!("{}:/planning-workspace", workspace.to_string_lossy()),
        "-v".into(), format!("{}:/app/config.yml", farm.join("config.yml").to_string_lossy()),
    ];
    if !project_id.is_empty() {
        docker_args.push("-e".into());
        docker_args.push(format!("DASHBOARD_PROJECT_ID={project_id}"));
    }
    docker_args.push(image);
    docker_args.push("python".into());
    docker_args.push("-m".into());
    docker_args.push("agent.discuss".into());

    #[cfg(target_os = "macos")]
    {
        let cmd = docker_args.iter().map(|a| {
            if a.contains(' ') || a.contains(':') { format!("\"{a}\"") } else { a.clone() }
        }).collect::<Vec<_>>().join(" ");
        let safe = cmd.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "tell application \"Terminal\" to activate\ntell application \"Terminal\" to do script \"{safe}\""
        );
        let _ = silent_command("osascript").args(["-e", &script]).spawn();
    }

    #[cfg(target_os = "windows")]
    {
        let bat_path = farm.join("discuss.bat");
        let line = docker_args.join(" ");
        let bat_content = format!("@echo off\n{line}\npause\n");
        let ps1_content = format!("Invoke-Expression '{}'\nRead-Host 'Press Enter to close'", line.replace('\'', "''"));
        let bat = bat_path.to_string_lossy().into_owned();
        let ps1_path = farm.join("discuss.ps1");
        let _ = std::fs::write(&bat_path, bat_content);
        let _ = std::fs::write(&ps1_path, ps1_content);
        let ps1 = ps1_path.to_string_lossy().into_owned();
        // Use a visible window — do NOT use silent_command here
        let opened = std::process::Command::new("wt.exe")
            .args(["powershell.exe", "-NoExit", "-File", &ps1])
            .spawn()
            .is_ok()
            || std::process::Command::new("powershell.exe")
                .args(["-NoExit", "-File", &ps1])
                .spawn()
                .is_ok();
        if !opened {
            let _ = std::process::Command::new("cmd.exe")
                .args(["/c", "start", "Farm Discuss", "cmd", "/k", &bat])
                .spawn();
        }
    }
}

#[tauri::command]
fn start_host_bridge() {
    let farm = farm_dir();
    std::thread::spawn(move || {
        let server = match tiny_http::Server::http("127.0.0.1:8092") {
            Ok(s) => s,
            Err(_) => return,
        };
        loop {
            if let Ok(mut req) = server.recv() {
                let is_options = *req.method() == tiny_http::Method::Options;
                let is_discuss = req.url() == "/open-discuss";

                let is_open_terminal = req.url() == "/open-terminal";

                let mut body = String::new();
                if (is_discuss || is_open_terminal) && !is_options {
                    let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
                }
                let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();

                let project_id = if is_discuss && !is_options {
                    parsed["projectId"].as_str().map(String::from).unwrap_or_default()
                } else {
                    String::new()
                };

                if is_discuss && !is_options {
                    open_discuss_terminal(&farm, &project_id);
                }

                if is_open_terminal && !is_options {
                    if let Some(path) = parsed["path"].as_str() {
                        open_folder_in_terminal(path);
                    }
                }

                let status = if is_options { 204u16 } else { 200u16 };
                let mut resp = tiny_http::Response::from_string("{\"ok\":true}")
                    .with_status_code(status);
                resp.add_header(tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap());
                resp.add_header(tiny_http::Header::from_bytes("Access-Control-Allow-Methods", "POST, OPTIONS").unwrap());
                resp.add_header(tiny_http::Header::from_bytes("Access-Control-Allow-Headers", "Content-Type").unwrap());
                resp.add_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap());
                let _ = req.respond(resp);
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AuthState::new(None))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let show    = MenuItem::with_id(app, "show",    "Show Flux",             true, None::<&str>)?;
            let stop    = MenuItem::with_id(app, "stop",    "Stop Farm",              true, None::<&str>)?;
            let migrate = MenuItem::with_id(app, "migrate", "Migrate from Terminal…", true, None::<&str>)?;
            let reset   = MenuItem::with_id(app, "reset",   "Reset Setup…",           true, None::<&str>)?;
            let quit    = MenuItem::with_id(app, "quit",    "Quit",                   true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &stop, &migrate, &reset, &quit])?;

            // macOS: monochrome template icon (OS adapts to menu bar colour)
            // Windows: use default coloured app icon
            #[cfg(target_os = "macos")]
            let (tray_icon, as_template) = {
                let icon = app.path().resource_dir()
                    .map(|d| d.join("tray-icon.png"))
                    .ok()
                    .filter(|p| p.exists())
                    .and_then(|p| tauri::image::Image::from_path(p).ok())
                    .unwrap_or_else(|| app.default_window_icon().unwrap().clone());
                (icon, true)
            };
            #[cfg(not(target_os = "macos"))]
            let (tray_icon, as_template) = (app.default_window_icon().unwrap().clone(), false);

            TrayIconBuilder::new()
                .menu(&menu)
                .icon(tray_icon)
                .icon_as_template(as_template)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.unminimize();
                            let _ = w.set_always_on_top(true);
                            let _ = w.show();
                            let _ = w.set_focus();
                            let _ = w.set_always_on_top(false);
                            #[cfg(target_os = "macos")]
                            let _ = app.show();
                        }
                    }
                    "stop" => {
                        // Ask frontend to confirm before stopping
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.unminimize();
                            let _ = w.set_always_on_top(true);
                            let _ = w.show();
                            let _ = w.set_focus();
                            let _ = w.set_always_on_top(false);
                            #[cfg(target_os = "macos")]
                            let _ = app.show();
                            let _ = w.emit("stop-requested", ());
                        }
                    }
                    "migrate" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.unminimize();
                            let _ = w.set_always_on_top(true);
                            let _ = w.show();
                            let _ = w.set_focus();
                            let _ = w.set_always_on_top(false);
                            #[cfg(target_os = "macos")]
                            let _ = app.show();
                            let _ = w.emit("migrate-requested", ());
                        }
                    }
                    "reset" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                            let _ = w.emit("reset-requested", ());
                        }
                    }
                    "quit" => {
                        // Bring window forward and let frontend confirm
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.unminimize();
                            let _ = w.set_always_on_top(true);
                            let _ = w.show();
                            let _ = w.set_focus();
                            let _ = w.set_always_on_top(false);
                            #[cfg(target_os = "macos")]
                            let _ = app.show();
                            let _ = w.emit("quit-requested", ());
                        }
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
            pull_image,
            run_docker_compose,
            run_detached,
            start_host_bridge,
            check_setup_complete,
            mark_setup_complete,
            check_wsl_installed,
            install_wsl,
            check_docker_running,
            check_claude_auth,
            start_claude_auth,
            complete_claude_auth,
            check_farm_running,
            check_dashboard_health,
            stop_farm,
            soft_reset,
            reset_setup,
            get_autostart,
            set_autostart,
            confirm_quit,
            get_farm_version,
            check_for_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application")
}
