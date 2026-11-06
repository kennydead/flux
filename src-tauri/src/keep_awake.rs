//! "Keep computer awake while the farm runs" (owner 2026-11-06; on the v2 backlog since 2026-08-21).
//! When the computer sleeps, Docker's VM suspends and an autonomous farm stalls. While the farm runs
//! (and the setting is on — default on), Flux holds a "don't idle-sleep" lock; it lets go when the
//! farm stops or Flux quits. Display sleep and closing the lid still work as usual.
//!  - macOS: `caffeinate -i -w <Flux pid>` — ends by itself when Flux ends, even after a crash.
//!  - Windows: a hidden PowerShell holding SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)
//!    while Flux's process lives (no extra crate; the state ends with that process).

use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

const SETTINGS: &str = "settings.json";

static HOLDER: Mutex<Option<Child>> = Mutex::new(None);

/// The tray's checkmark item, kept so the home-screen switch can update it.
pub struct TrayItem(pub tauri::menu::CheckMenuItem<tauri::Wry>);
static FARM_RUNNING: AtomicBool = AtomicBool::new(false);

fn settings_path() -> std::path::PathBuf {
    crate::docker_env::data_dir().join(SETTINGS)
}

fn read_settings() -> serde_json::Value {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}))
}

/// The setting (default on).
pub fn enabled() -> bool {
    read_settings()["keep_awake"].as_bool().unwrap_or(true)
}

fn save_enabled(on: bool) {
    let mut s = read_settings();
    s["keep_awake"] = serde_json::json!(on);
    let _ = std::fs::create_dir_all(crate::docker_env::data_dir());
    let _ = std::fs::write(settings_path(), serde_json::to_string_pretty(&s).unwrap_or_default());
}

fn spawn_holder() -> Option<Child> {
    let pid = std::process::id().to_string();
    #[cfg(target_os = "macos")]
    {
        crate::silent_command("/usr/bin/caffeinate").args(["-i", "-w", &pid]).spawn().ok()
    }
    #[cfg(target_os = "windows")]
    {
        let script = format!(
            "$k = Add-Type -MemberDefinition '[DllImport(\"kernel32.dll\")] public static extern uint SetThreadExecutionState(uint f);' -Name Power -Namespace FluxKeepAwake -PassThru; \
             [void]$k::SetThreadExecutionState(0x80000001); \
             while (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ Start-Sleep -Seconds 30 }}"
        );
        crate::silent_command("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
            .spawn()
            .ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = pid;
        None
    }
}

/// Hold or release the lock to match "farm running AND setting on".
fn apply() {
    let want = FARM_RUNNING.load(Ordering::SeqCst) && enabled();
    let Ok(mut holder) = HOLDER.lock() else { return };
    // A holder that died on its own (killed by the user, say) counts as released.
    if let Some(child) = holder.as_mut() {
        if matches!(child.try_wait(), Ok(Some(_))) {
            *holder = None;
        }
    }
    match (want, holder.is_some()) {
        (true, false) => *holder = spawn_holder(),
        (false, true) => {
            if let Some(mut child) = holder.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        _ => {}
    }
}

/// The farm started (or Flux found it running) / stopped.
pub fn set_farm_running(running: bool) {
    FARM_RUNNING.store(running, Ordering::SeqCst);
    apply();
}

/// Change the setting; returns the new value.
pub fn set_enabled(on: bool) -> bool {
    save_enabled(on);
    apply();
    on
}

#[tauri::command]
pub fn get_keep_awake() -> bool {
    enabled()
}

/// From the home screen's switch. The tray's checkmark follows through the "keep-awake-changed" event.
#[tauri::command]
pub fn set_keep_awake(app: tauri::AppHandle, enabled: bool) -> bool {
    use tauri::{Emitter, Manager};
    let on = set_enabled(enabled);
    if let Some(item) = app.try_state::<TrayItem>() {
        let _ = item.0.set_checked(on);
    }
    let _ = app.emit("keep-awake-changed", on);
    on
}

#[tauri::command]
pub fn farm_running_changed(running: bool) {
    set_farm_running(running);
}
