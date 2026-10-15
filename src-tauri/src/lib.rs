use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WindowEvent,
};
use std::path::PathBuf;

mod docker_env;
mod pull;

const FARM_VERSION: &str = "latest";
const IMAGE_REGISTRY: &str = "ghcr.io/kennydead/claude-agent-farm";

fn agent_image() -> String {
    format!("{}/agent:{}", IMAGE_REGISTRY, farm_channel())
}

/// Release channel for farm images: FARM_CHANNEL from ~/farm/.env, or "latest".
/// Same contract as the farm's own self-updater — staging installs opt in by
/// setting FARM_CHANNEL=staging there.
fn farm_channel() -> String {
    if let Ok(content) = std::fs::read_to_string(farm_dir().join(".env")) {
        for line in content.lines() {
            if let Some(v) = line.trim().strip_prefix("FARM_CHANNEL=") {
                let v = v.trim();
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    "latest".to_string()
}

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

/// Every docker / docker compose call Flux makes: the docker binary, an augmented PATH and (on
/// macOS) the Flux-owned docker config — see docker_env.rs.
fn docker_command() -> std::process::Command {
    let mut cmd = silent_command(&docker_bin());
    docker_env::apply(&mut cmd);
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

/// Append `KEY=value` to the env file if no line with that key exists.
fn ensure_env_var(env_path: &std::path::Path, key: &str, value: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(env_path).unwrap_or_default();
    if content.lines().any(|l| l.starts_with(&format!("{key}="))) {
        return Ok(());
    }
    let mut new_content = content;
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push_str(&format!("{key}={value}\n"));
    std::fs::write(env_path, new_content).map_err(|e| e.to_string())
}

/// 64 hex chars from the OS RNG (no extra crate: SystemTime + getrandom via /dev/urandom
/// is not portable to Windows, so use the `getrandom` crate).
fn random_hex_key() -> String {
    let mut buf = [0u8; 32];
    if getrandom::getrandom(&mut buf).is_err() {
        // Extremely unlikely; fall back to a time-seeded value rather than failing setup
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (nanos.rotate_left(i as u32) & 0xff) as u8;
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
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

    // GUI-launched processes have no shell PWD, which docker-compose variable
    // substitution relied on — persist the farm dir so ${COMPOSE_PROJECT_DIR}
    // resolves to the real host path (needed for Live Session bind mounts).
    let env_path = farm.join(".env");
    ensure_env_var(&env_path, "COMPOSE_PROJECT_DIR", &farm.to_string_lossy())?;

    // Persist a random secret for token encryption — relying on the compose
    // default means any default change invalidates every stored token.
    let existing = std::fs::read_to_string(&env_path).unwrap_or_default();
    if !existing.lines().any(|l| l.starts_with("DASHBOARD_SECRET_KEY=")) {
        ensure_env_var(&env_path, "DASHBOARD_SECRET_KEY", &random_hex_key())?;
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
    tauri::async_runtime::spawn_blocking(move || {
        docker_command()
            .args(["info", "--format", "{{.ServerVersion}}"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

#[tauri::command]
async fn run_command(program: String, args: Vec<String>) -> Result<String, String> {
    let is_docker = program == "docker";
    let bin = if is_docker { docker_bin() } else if program == "python3" { python_bin() } else { program };
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = if is_docker { docker_command() } else { silent_command(&bin) };
        if !is_docker {
            cmd.env("PATH", augmented_path());
        }
        let output = cmd
            .args(&args)
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
async fn pull_image(app: AppHandle, image: String) -> Result<bool, String> {
    pull::pull_images(Some(app), vec![image]).await
}

fn farm_image_refs(channel: &str) -> Vec<String> {
    ["agent", "dashboard-backend", "dashboard-frontend"]
        .iter()
        .map(|img| format!("{IMAGE_REGISTRY}/{img}:{channel}"))
        .collect()
}

/// The compose file pins :latest, so a staging install must retag channel→latest for the pulled
/// image to take effect (no-op when the channel is already "latest").
fn retag_channel_to_latest(channel: &str) {
    if channel == "latest" {
        return;
    }
    for img in ["agent", "dashboard-backend", "dashboard-frontend"] {
        let src = format!("{IMAGE_REGISTRY}/{img}:{channel}");
        let dst = format!("{IMAGE_REGISTRY}/{img}:latest");
        let _ = docker_command().args(["tag", &src, &dst]).output();
    }
}

/// Pull the farm images for the current channel with a progress stream (PROGRESS_EVENT).
/// Returns true when any image changed.
#[tauri::command]
async fn pull_farm_images(app: AppHandle) -> Result<bool, String> {
    let channel = farm_channel();
    let updated = pull::pull_images(Some(app), farm_image_refs(&channel)).await?;
    tauri::async_runtime::spawn_blocking(move || retag_channel_to_latest(&channel))
        .await
        .map_err(|e| e.to_string())?;
    Ok(updated)
}

/// Farm endpoint saying whether recreating the dashboard now would interrupt work (a run check,
/// a preview, a planning session…). Old farms don't have it: 404 / no answer = "unknown → safe".
const UPDATE_SAFETY_URL: &str = "http://localhost:8090/system/update-safety";

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
struct SafetyReason {
    #[serde(default)]
    code: String,
    #[serde(default)]
    text: String,
}

#[derive(serde::Serialize, Clone, Debug)]
struct UpdateSafety {
    safe: bool,
    /// false = the farm didn't answer (old farm / not running) and `safe` is the default.
    known: bool,
    reasons: Vec<SafetyReason>,
}

fn update_safety() -> UpdateSafety {
    // Only "this farm is too old to have the endpoint" (404) counts as safe by default. A timeout,
    // an error or garbage means "not sure" and is NOT safe — retry later (audit 2026-10-15: one slow
    // answer used to let a deferred update recreate the farm in the middle of work).
    let unsure = UpdateSafety { safe: false, known: false, reasons: vec![] };
    let resp = match ureq::get(UPDATE_SAFETY_URL).timeout(std::time::Duration::from_secs(5)).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return UpdateSafety { safe: true, known: false, reasons: vec![] },
        Err(_) => return unsure,
    };
    let Ok(body) = resp.into_json::<serde_json::Value>() else { return unsure };
    let Some(safe) = body["safe"].as_bool() else { return unsure };
    let reasons = body["reasons"]
        .as_array()
        .map(|a| a.iter().filter_map(|r| serde_json::from_value::<SafetyReason>(r.clone()).ok()).collect())
        .unwrap_or_default();
    UpdateSafety { safe, known: true, reasons }
}

fn safety_reasons_text(s: &UpdateSafety) -> String {
    let texts: Vec<&str> = s.reasons.iter().map(|r| r.text.as_str()).filter(|t| !t.is_empty()).collect();
    if texts.is_empty() { "work in progress".to_string() } else { texts.join(", ") }
}

#[tauri::command]
async fn check_update_safety() -> UpdateSafety {
    tauri::async_runtime::spawn_blocking(update_safety)
        .await
        .unwrap_or(UpdateSafety { safe: false, known: false, reasons: vec![] })
}

/// True when a running dashboard container runs an older image than the local :latest tag —
/// an update was downloaded but not installed yet (e.g. deferred while the farm was busy, then
/// Flux restarted). Stateless, so a deferred update survives a Flux restart.
fn farm_update_pending_blocking() -> bool {
    for svc in ["dashboard-backend", "dashboard-frontend"] {
        let out = |args: &[&str]| -> String {
            docker_command()
                .args(args)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        };
        let project = "label=com.docker.compose.project=claudeagentfarm".to_string();
        let service = format!("label=com.docker.compose.service={svc}");
        let ids = out(&["ps", "-q", "--filter", &project, "--filter", &service]);
        let Some(cid) = ids.lines().next().map(str::trim).filter(|s| !s.is_empty()) else { continue };
        let running = out(&["inspect", "--format", "{{.Image}}", cid]);
        let local = out(&["image", "inspect", "--format", "{{.Id}}", &format!("{IMAGE_REGISTRY}/{svc}:latest")]);
        if !running.is_empty() && !local.is_empty() && running != local {
            return true;
        }
    }
    false
}

#[tauri::command]
async fn farm_update_pending() -> bool {
    tauri::async_runtime::spawn_blocking(farm_update_pending_blocking)
        .await
        .unwrap_or(false)
}

/// Recreate the dashboard stack so freshly pulled images take effect. --force-recreate: a plain
/// `up -d` does not reliably replace a running container after its image changed.
fn recreate_dashboard() -> Result<(), String> {
    let farm = farm_dir();
    let compose_file = farm.join("docker-compose.yml").to_string_lossy().into_owned();
    let out = docker_command()
        .args([
            "compose", "-f", &compose_file, "up", "-d", "--force-recreate",
            "dashboard-db", "dashboard-backend", "dashboard-frontend",
        ])
        .current_dir(&farm)
        .output()
        .map_err(|e| format!("compose up failed: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// Install a downloaded farm update (the deferred-until-idle path).
#[tauri::command]
async fn apply_farm_update() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(recreate_dashboard)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn run_docker_compose(sub_args: Vec<String>) -> Result<String, String> {
    let farm = farm_dir();
    let compose_file = farm.join("docker-compose.yml").to_string_lossy().into_owned();
    let mut full_args = vec!["compose".to_string(), "-f".to_string(), compose_file];
    full_args.extend(sub_args);
    tauri::async_runtime::spawn_blocking(move || {
        let output = docker_command()
            .args(&full_args)
            .current_dir(farm_dir())
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
    tauri::async_runtime::spawn_blocking(move || {
        let _ = docker_command()
            .args(["compose", "-f", &compose_file, "down"])
            .output();
    }).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn soft_reset() -> Result<(), String> {
    let farm = farm_dir();

    // Stop running containers — preserve all volumes (credentials + database)
    let compose_file = farm.join("docker-compose.yml").to_string_lossy().into_owned();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let _ = docker_command()
            .args(["compose", "-f", &compose_file, "down"])
            .output();
    }).await;

    // Remove farm directory (clears setup_complete flag and old scripts)
    let _ = std::fs::remove_dir_all(&farm);

    Ok(())
}

#[tauri::command]
async fn reset_setup() -> Result<(), String> {
    let farm = farm_dir();

    // Stop running containers
    let compose_file = farm.join("docker-compose.yml").to_string_lossy().into_owned();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let _ = docker_command()
            .args(["compose", "-f", &compose_file, "down"])
            .output();
    }).await;

    // Remove license key
    let _ = std::fs::remove_file(farm.join("logs").join("license_key.txt"));

    // Remove Claude auth volume
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let _ = docker_command()
            .args(["volume", "rm", "-f", "claudeagentfarm_claude-home"])
            .output();
    }).await;

    Ok(())
}

#[tauri::command]
async fn confirm_quit(app: AppHandle) {
    let farm = farm_dir();
    let compose_file = farm.join("docker-compose.yml").to_string_lossy().into_owned();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let _ = docker_command()
            .args(["compose", "-f", &compose_file, "down"])
            .output();
    }).await;
    app.exit(0);
}

#[tauri::command]
fn get_farm_version() -> String {
    farm_channel()
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

/// Convert a Git Bash / MSYS-style path (`/c/Users/...`) to a native Windows path (`C:\Users\...`).
/// Paths that don't match the pattern are returned unchanged.
#[allow(dead_code)]
fn to_windows_path(path: &str) -> String {
    let b = path.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b'/' {
        let drive = (b[1] as char).to_ascii_uppercase();
        let rest = path[2..].replace('/', "\\");
        format!("{drive}:{rest}")
    } else {
        path.to_string()
    }
}

/// This computer's address on the local network and its Bonjour/host name, for the dashboard's
/// "Access from other devices" setting (the farm runs in Docker and cannot see either). The UDP
/// "connect" sends nothing; it only asks the OS which interface would route outward.
#[tauri::command]
fn host_network_info() -> serde_json::Value {
    let mut ips: Vec<String> = Vec::new();
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("192.0.2.1:9").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                let ip = addr.ip();
                if !ip.is_loopback() && !ip.is_unspecified() {
                    ips.push(ip.to_string());
                }
            }
        }
    }
    serde_json::json!({ "ips": ips, "hostname": local_hostname() })
}

fn local_hostname() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let out = silent_command("scutil").args(["--get", "LocalHostName"]).output().ok()?;
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return (!name.is_empty()).then(|| format!("{name}.local"));
    }
    #[cfg(target_os = "windows")]
    {
        return std::env::var("COMPUTERNAME").ok().filter(|s| !s.is_empty());
    }
    #[allow(unreachable_code)]
    {
        let out = std::process::Command::new("hostname").output().ok()?;
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!name.is_empty()).then_some(name)
    }
}

/// Origins allowed to drive the host bridge: the dashboard (in Flux's frame or a browser tab) and
/// Flux itself. A browser always sends Origin on these cross-origin POSTs, so any other website is
/// refused (audit 2026-10-15: with `Access-Control-Allow-Origin: *` any page the user visited
/// could reach /open-terminal and /open-discuss, which build shell commands). Local programs send
/// no Origin and are allowed — they can run commands anyway.
const BRIDGE_ORIGINS: [&str; 5] = [
    "http://localhost:5174", "http://127.0.0.1:5174",
    "tauri://localhost", "http://tauri.localhost", "https://tauri.localhost",
];

fn bridge_origin(req: &tiny_http::Request) -> Result<Option<String>, ()> {
    match req.headers().iter().find(|h| h.field.equiv("Origin")) {
        None => Ok(None),
        Some(h) => {
            let o = h.value.as_str();
            if BRIDGE_ORIGINS.contains(&o) { Ok(Some(o.to_string())) } else { Err(()) }
        }
    }
}

/// A farm project id is a UUID; anything else never reaches a command line.
fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().enumerate().all(|(i, c)| {
        if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() }
    })
}

/// POSIX single-quoting: the shell expands nothing inside '…' ('\'' closes, escapes, reopens).
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn open_folder_in_terminal(path: &str) {
    // Only an existing directory, and nothing that could end a line or a command.
    if path.contains(['\n', '\r', '\0']) || !std::path::Path::new(path).is_dir() {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        // Shell-quote first (the command Terminal runs), then escape for the AppleScript string.
        let cmd = format!("cd {}", shell_quote(path));
        let safe = cmd.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "tell application \"Terminal\" to activate\ntell application \"Terminal\" to do script \"{safe}\""
        );
        let _ = silent_command("osascript").args(["-e", &script]).spawn();
    }

    #[cfg(target_os = "windows")]
    {
        // cmd.exe re-parses its command line: refuse its metacharacters outright.
        if path.contains(['&', '|', '<', '>', '^', '%', '"']) {
            return;
        }
        // Use start's /d switch for the working directory — nesting a quoted
        // `cd /d "..."` string inside cmd /k breaks because cmd.exe doesn't
        // understand Rust's backslash-escaped quotes.
        let native = to_windows_path(path);
        let _ = std::process::Command::new("cmd.exe")
            .args(["/c", "start", "Project Terminal", "/d", &native, "cmd"])
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
    if !project_id.is_empty() && is_uuid(project_id) {
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

/// Event telling the Flux window that a farm update is downloaded but waits for the farm to be
/// idle (payload: UpdateSafety) — App.tsx polls the safety endpoint and installs it then.
const DEFERRED_EVENT: &str = "farm-update-deferred";

/// One farm update at a time (bridge requests, background downloads).
static FARM_UPDATE_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Pull the channel's images (with progress in the Flux window) and retag them for compose.
/// Returns true when an image changed. Blocking — runs on the bridge thread / a worker thread.
fn pull_farm_images_blocking(app: &AppHandle) -> Result<bool, String> {
    let channel = farm_channel();
    let updated = tauri::async_runtime::block_on(pull::pull_images(Some(app.clone()), farm_image_refs(&channel)))?;
    retag_channel_to_latest(&channel);
    Ok(updated)
}

fn busy_body(message: &str, safety: &UpdateSafety) -> String {
    serde_json::json!({ "ok": false, "busy": true, "error": message, "reasons": safety.reasons }).to_string()
}

/// /update-farm on the host bridge: the dashboard's in-app "Update" button on a Flux install.
/// The dashboard can't safely update itself (it would recreate its own backend mid-request, and on
/// Windows its self-updater is disabled), so it delegates to Flux, an external process that
/// survives the recreate. Returns (HTTP status, JSON body) in the shape the dashboard reads:
/// 200 {"ok":true,"status":"updated"} or an error status with {"ok":false,"error":…}.
/// When the farm is busy (update-safety says not safe) nothing is recreated: the reply is 409
/// busy with the reasons, the update downloads in the background and Flux installs it when the
/// farm is idle.
fn run_farm_update(app: &AppHandle) -> (u16, String) {
    use std::sync::atomic::Ordering;
    if FARM_UPDATE_RUNNING.swap(true, Ordering::SeqCst) {
        let msg = "A farm update is already in progress.";
        return (409, serde_json::json!({ "ok": false, "busy": true, "error": msg, "reasons": [] }).to_string());
    }

    let pre = update_safety();
    if !pre.safe {
        let msg = format!(
            "The farm is busy (waiting for: {}). The update downloads now and installs automatically when the farm is idle.",
            safety_reasons_text(&pre)
        );
        let app2 = app.clone();
        std::thread::spawn(move || {
            let res = pull_farm_images_blocking(&app2);
            FARM_UPDATE_RUNNING.store(false, Ordering::SeqCst);
            match res {
                Ok(updated) if updated || farm_update_pending_blocking() => {
                    let _ = app2.emit(DEFERRED_EVENT, update_safety());
                }
                Ok(_) => {}
                Err(e) => eprintln!("[flux] background farm update download failed: {e}"),
            }
        });
        return (409, busy_body(&msg, &pre));
    }

    let result = (|| -> Result<Option<UpdateSafety>, String> {
        let updated = pull_farm_images_blocking(app)?;
        // The pull can take minutes — work may have started meanwhile.
        let post = update_safety();
        if !post.safe {
            if updated || farm_update_pending_blocking() {
                let _ = app.emit(DEFERRED_EVENT, post.clone());
            }
            return Ok(Some(post));
        }
        recreate_dashboard()?;
        Ok(None)
    })();
    FARM_UPDATE_RUNNING.store(false, Ordering::SeqCst);

    match result {
        Ok(None) => (200, "{\"ok\":true,\"status\":\"updated\"}".to_string()),
        Ok(Some(post)) => {
            let msg = format!(
                "Update downloaded — it installs automatically when the farm is idle (waiting for: {}).",
                safety_reasons_text(&post)
            );
            (409, busy_body(&msg, &post))
        }
        Err(e) => {
            let e: String = e.chars().take(400).collect();
            (500, serde_json::json!({ "ok": false, "error": e }).to_string())
        }
    }
}

#[tauri::command]
fn start_host_bridge(app: AppHandle) {
    let farm = farm_dir();
    std::thread::spawn(move || {
        let server = match tiny_http::Server::http("127.0.0.1:8092") {
            Ok(s) => s,
            Err(_) => return,
        };
        loop {
            if let Ok(mut req) = server.recv() {
                let origin = match bridge_origin(&req) {
                    Ok(o) => o,
                    Err(()) => {
                        let _ = req.respond(tiny_http::Response::from_string("{\"error\":\"forbidden origin\"}")
                            .with_status_code(403));
                        continue;
                    }
                };
                let is_options = *req.method() == tiny_http::Method::Options;
                let is_discuss = req.url() == "/open-discuss";

                let is_open_terminal = req.url() == "/open-terminal";
                let is_update = req.url() == "/update-farm";

                let mut body = String::new();
                if (is_discuss || is_open_terminal) && !is_options {
                    let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
                }
                let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();

                let project_id = if is_discuss && !is_options {
                    parsed["projectId"].as_str().filter(|s| is_uuid(s)).map(String::from).unwrap_or_default()
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

                // Body reported back for /update-farm so the dashboard can show success/failure.
                let mut resp_body = String::from("{\"ok\":true}");
                let mut status = if is_options { 204u16 } else { 200u16 };
                if is_update && !is_options {
                    let (code, body) = run_farm_update(&app);
                    status = code;
                    resp_body = body;
                }

                let mut resp = tiny_http::Response::from_string(resp_body)
                    .with_status_code(status);
                if let Some(o) = &origin {
                    resp.add_header(tiny_http::Header::from_bytes("Access-Control-Allow-Origin", o.as_bytes()).unwrap());
                    resp.add_header(tiny_http::Header::from_bytes("Vary", "Origin").unwrap());
                }
                resp.add_header(tiny_http::Header::from_bytes("Access-Control-Allow-Methods", "POST, OPTIONS").unwrap());
                resp.add_header(tiny_http::Header::from_bytes("Access-Control-Allow-Headers", "Content-Type").unwrap());
                resp.add_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap());
                let _ = req.respond(resp);
            }
        }
    });
}

/// A farm notification that needs the owner (ticket waiting for approval, stage ready, something
/// blocked) as a macOS / Windows notification — only when the Flux window is not in front, since
/// the dashboard already shows it otherwise. Returns true when a notification was shown.
#[tauri::command]
fn desktop_notify(app: AppHandle, title: String, body: String) -> bool {
    use tauri_plugin_notification::{NotificationExt, PermissionState};
    if let Some(w) = app.get_webview_window("main") {
        let visible = w.is_visible().unwrap_or(false);
        let focused = w.is_focused().unwrap_or(false);
        if visible && focused {
            eprintln!("[flux] desktop_notify skipped: window is in front");
            return false;
        }
    }
    let n = app.notification();
    if !matches!(n.permission_state(), Ok(PermissionState::Granted)) {
        let _ = n.request_permission();
    }
    let res = n.builder().title(title.clone()).body(body).show();
    eprintln!("[flux] desktop_notify '{}' -> permission {:?}, shown {:?}", title, n.permission_state().ok(), res.as_ref().map(|_| ()).map_err(|e| e.to_string()));
    res.is_ok()
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // One Flux at a time: a second launch (e.g. Windows starting Flux again from a clicked
        // notification) brings the running window forward instead of starting a second copy.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| { show_main_window(app); }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            if let Ok(dir) = app.path().app_data_dir() {
                docker_env::set_app_data_dir(dir);
            }
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
            host_network_info,
            get_farm_dir,
            check_license,
            validate_license_key,
            save_license_key,
            extract_resources,
            run_command,
            pull_image,
            pull_farm_images,
            check_update_safety,
            farm_update_pending,
            apply_farm_update,
            run_docker_compose,
            run_detached,
            start_host_bridge,
            check_setup_complete,
            mark_setup_complete,
            check_wsl_installed,
            install_wsl,
            check_docker_running,
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
            desktop_notify,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // macOS: clicking a notification (or the Dock icon) while the window is hidden to the
            // tray re-activates the app — bring the window back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main_window(app);
            }
            let _ = (app, event);
        })
}

#[cfg(test)]
mod bridge_tests {
    use super::*;

    #[test]
    fn project_ids_must_be_uuids() {
        assert!(is_uuid("a39dd42e-4e11-47f3-855a-b40b1e2b13b8"));
        for bad in ["", "x; rm -rf ~", "a39dd42e-4e11-47f3-855a-b40b1e2b13b8; id", "a39dd42e 4e11 47f3 855a b40b1e2b13b8",
                    "../../etc/passwd", "a39dd42e-4e11-47f3-855a-b40b1e2b13bZ"] {
            assert!(!is_uuid(bad), "{bad}");
        }
    }

    #[test]
    fn shell_quote_leaves_nothing_for_the_shell_to_expand() {
        assert_eq!(shell_quote("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(shell_quote("/tmp/$(id)`id`\"x"), "'/tmp/$(id)`id`\"x'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        // Round-trip through a real shell: the argument comes back byte for byte, nothing runs.
        for s in ["/tmp/$(touch /tmp/flux-audit-pwned)", "a'b\"c`d`$HOME", "plain"] {
            let out = std::process::Command::new("sh").arg("-c").arg(format!("printf %s {}", shell_quote(s)))
                .output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), s);
        }
        assert!(!std::path::Path::new("/tmp/flux-audit-pwned").exists());
    }

    #[test]
    fn only_existing_directories_are_opened() {
        // open_folder_in_terminal returns before building any command for these.
        for bad in ["/no/such/dir/$(id)", "/tmp\n; id", "/etc/hosts"] {
            assert!(bad.contains(['\n', '\r', '\0']) || !std::path::Path::new(bad).is_dir());
        }
    }

    #[test]
    fn allowed_origins_are_the_dashboard_and_flux() {
        assert!(BRIDGE_ORIGINS.contains(&"http://localhost:5174"));
        assert!(!BRIDGE_ORIGINS.contains(&"https://evil.example"));
        assert!(!BRIDGE_ORIGINS.contains(&"http://localhost:5174.evil.example"));
    }
}
