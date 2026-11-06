//! One "Flux update" for the user (owner 2026-11-06): Flux is two parts — this desktop app and the
//! farm's Docker images — but the user only cares whether there is an update. This module answers
//! that for both parts and installs it, so the dashboard (through the host bridge) and the tray menu
//! offer ONE update, reachable while the farm runs (before, the app update sat behind Stop Farm).
//!
//! Installing:
//!  - a new Flux app: download + install, then restart Flux. The farm keeps running; the restarted
//!    Flux runs its normal startup, which pulls the new images and installs them when the farm is
//!    idle (or at once when the user chose "Update now" — the flag file below).
//!  - images only: the existing farm update (lib.rs run_farm_update).

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// The Flux window shows "Updating Flux — it restarts in a moment" while this runs.
pub const APP_UPDATING_EVENT: &str = "flux-app-updating";
/// The Flux window shows a notice when the app update failed.
pub const APP_UPDATE_FAILED_EVENT: &str = "flux-app-update-failed";

const FRESH_FOR: Duration = Duration::from_secs(15 * 60);
const NOW_FLAG: &str = ".flux-update-now";
/// Set before restarting into a new Flux: the next launch runs the startup (pulls the new images)
/// even though the farm is running — the file form of HomeScreen's markUpdateFarmOnLaunch.
const AFTER_FLAG: &str = ".flux-after-update";

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Status {
    /// This Flux app's version.
    pub flux_version: String,
    /// A newer Flux app ("0.5.6"), or None.
    pub app_update: Option<String>,
    /// New farm images for the channel: Some(true/false), None = couldn't tell.
    pub images_update: Option<bool>,
    /// Either part has something new.
    pub update_available: bool,
}

static CACHE: Mutex<Option<(Status, Instant)>> = Mutex::new(None);
static APP_UPDATE_RUNNING: AtomicBool = AtomicBool::new(false);

fn combine(flux_version: String, app_update: Option<String>, images_update: Option<bool>) -> Status {
    let update_available = app_update.is_some() || images_update == Some(true);
    Status { flux_version, app_update, images_update, update_available }
}

async fn app_update_version(app: &AppHandle) -> Option<String> {
    let updater = app.updater().ok()?;
    match tokio::time::timeout(Duration::from_secs(15), updater.check()).await {
        Ok(Ok(Some(u))) => Some(u.version),
        _ => None,
    }
}

/// Both parts' update state; cached 15 minutes (the dashboard polls), `refresh` asks again now.
pub async fn status(app: &AppHandle, refresh: bool) -> Status {
    if !refresh {
        if let Some((s, at)) = CACHE.lock().ok().and_then(|g| g.clone()) {
            if at.elapsed() < FRESH_FOR {
                return s;
            }
        }
    }
    let version = app.package_info().version.to_string();
    let app_update = app_update_version(app).await;
    let images = crate::farm_image_refs(&crate::farm_channel());
    let images_update = crate::pull::images_outdated(images).await;
    let s = combine(version, app_update, images_update);
    if let Ok(mut g) = CACHE.lock() {
        *g = Some((s.clone(), Instant::now()));
    }
    s
}

fn flag_path(name: &str) -> std::path::PathBuf {
    crate::farm_dir().join("logs").join(name)
}

fn take_flag(name: &str) -> bool {
    let p = flag_path(name);
    let set = p.exists();
    let _ = std::fs::remove_file(p);
    set
}

/// Set before an app update the user asked to install "now": the restarted Flux then installs the
/// new images without waiting for the farm to be idle. Read once (and removed) at startup.
pub fn take_now_flag() -> bool {
    take_flag(NOW_FLAG)
}

/// Download + install the new Flux app, then restart Flux. Returns Ok(false) when there is no app
/// update (nothing to do), Ok(true) when the install started (Flux restarts on its own shortly).
/// The work runs in the background: the caller's reply goes out before Flux restarts.
pub async fn start_app_update(app: &AppHandle, now: bool) -> Result<bool, String> {
    if APP_UPDATE_RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(true);   // already installing
    }
    let update = match app.updater().map_err(|e| e.to_string())?.check().await {
        Ok(Some(u)) => u,
        Ok(None) => {
            APP_UPDATE_RUNNING.store(false, Ordering::SeqCst);
            return Ok(false);
        }
        Err(e) => {
            APP_UPDATE_RUNNING.store(false, Ordering::SeqCst);
            return Err(format!("Could not check for the Flux update: {e}"));
        }
    };
    let _ = app.emit(APP_UPDATING_EVENT, update.version.clone());
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        match update.download_and_install(|_, _| {}, || {}).await {
            Ok(()) => {
                let _ = std::fs::create_dir_all(crate::farm_dir().join("logs"));
                let _ = std::fs::write(flag_path(AFTER_FLAG), "1");
                if now {
                    let _ = std::fs::write(flag_path(NOW_FLAG), "1");
                }
                // Give the bridge reply / the window notice a moment, then restart into the new app.
                tokio::time::sleep(Duration::from_millis(800)).await;
                app2.restart();
            }
            Err(e) => {
                APP_UPDATE_RUNNING.store(false, Ordering::SeqCst);
                let _ = app2.emit(APP_UPDATE_FAILED_EVENT, e.to_string());
                eprintln!("[flux] app update failed: {e}");
            }
        }
    });
    Ok(true)
}

// ── Tauri commands (Flux window: tray "Check for Updates…" dialog, startup) ────────────────────

#[tauri::command]
pub async fn check_updates(app: AppHandle, refresh: bool) -> Status {
    status(&app, refresh).await
}

#[tauri::command]
pub async fn install_app_update(app: AppHandle, now: bool) -> Result<bool, String> {
    start_app_update(&app, now).await
}

#[tauri::command]
pub fn take_update_now_flag() -> bool {
    take_now_flag()
}

#[tauri::command]
pub fn take_after_update_flag() -> bool {
    take_flag(AFTER_FLAG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_part_makes_an_update() {
        assert!(!combine("0.5.5".into(), None, Some(false)).update_available);
        assert!(!combine("0.5.5".into(), None, None).update_available);
        assert!(combine("0.5.5".into(), Some("0.5.6".into()), Some(false)).update_available);
        assert!(combine("0.5.5".into(), None, Some(true)).update_available);
    }
}
