//! The environment every docker / docker compose call Flux makes runs with.
//!
//! macOS: the owner's ~/.docker/config.json usually has `"credsStore": "desktop"`, so the docker
//! CLI runs docker-credential-desktop on pulls, which reads Docker Desktop's group container and
//! makes macOS ask "Flux would like to access data from other apps" — every time, because Flux is
//! not signed. Flux needs no credential helper: on macOS its docker calls run with DOCKER_CONFIG
//! pointing at a Flux-owned directory whose config.json lists where the CLI plugins (compose) live,
//! plus the farm images' read-only login as a plain owner-only `auths` entry (registry.rs). Without the user's config the CLI would fall back
//! to the default context (/var/run/docker.sock), so DOCKER_HOST is set to the endpoint of the
//! user's current context (resolved once, read-only, with their own config).
//!
//! Anything unexpected (no docker yet, a non-socket context, compose not found with the isolated
//! config) falls back to the old behaviour: the user's own config, nothing overridden.
//! Windows keeps the user's config (no prompt there; the default named pipe just works).

#[allow(unused_imports)]
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static RESOLVED: Mutex<Option<DockerEnv>> = Mutex::new(None);

/// Called once from setup() with the app's data dir (where the isolated config lives).
pub fn set_app_data_dir(dir: PathBuf) {
    let _ = APP_DATA_DIR.set(dir);
}

#[derive(Clone, Default, Debug)]
pub struct DockerEnv {
    /// Flux-owned DOCKER_CONFIG directory; None = the user's own config (old behaviour).
    pub config_dir: Option<PathBuf>,
    /// DOCKER_HOST to set for the CLI alongside the isolated config.
    pub cli_host: Option<String>,
    /// The daemon endpoint the user's docker CLI talks to (also used by the Engine API client).
    pub endpoint: Option<String>,
}

/// The resolved environment. A failed resolution (docker not installed / not runnable yet) is not
/// cached, so a Docker installed during setup is picked up without restarting Flux.
pub fn get() -> DockerEnv {
    if let Some(env) = RESOLVED.lock().ok().and_then(|g| g.clone()) {
        return env;
    }
    let (env, cacheable) = resolve();
    if cacheable {
        if let Ok(mut g) = RESOLVED.lock() {
            *g = Some(env.clone());
        }
    }
    env
}

/// Flux's own data directory (the isolated config and the saved registry login live under it).
pub fn data_dir() -> PathBuf {
    APP_DATA_DIR.get().cloned().unwrap_or_else(|| crate::farm_dir().join(".flux"))
}

/// Write a file only its owner can read (it may carry the registry login).
pub fn write_private(path: &Path, content: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // A temp name of its own per write (two pulls can save the login at once), created fresh so
        // no leftover with wider permissions ever holds the token.
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_extension(format!("tmp-{}-{n}", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        f.write_all(content.as_bytes())?;
        drop(f);
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, content)   // under the user's profile: only that user can read it
    }
}

/// Put (Some) or remove (None) the ghcr.io login in Flux's ISOLATED docker config, so the CLI fallback
/// and `docker compose` can pull the private images. Never touches the user's own docker config: with
/// no isolated config (Windows, or macOS when isolation is off) it does nothing here — the Engine API
/// pulls carry the login themselves and the CLI fallback gets a one-off config (temp_login_config).
pub fn set_registry_auth(host: &str, auth_b64: Option<&str>) {
    let Some(dir) = get().config_dir else { return };
    set_auth_in(&dir.join("config.json"), host, auth_b64);
}

fn set_auth_in(path: &Path, host: &str, auth_b64: Option<&str>) {
    // An unreadable config is left alone (rewriting it as `{auths}` would drop the compose plugin dirs);
    // isolate() rebuilds it at the next start.
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    let Some(mut cfg) = serde_json::from_str::<serde_json::Value>(&raw).ok().filter(|v| v.is_object()) else {
        eprintln!("[flux] the isolated docker config can't be read; registry login not written");
        return;
    };
    match auth_b64 {
        Some(a) => {
            if !cfg["auths"].is_object() {
                cfg["auths"] = serde_json::json!({});
            }
            cfg["auths"][host] = serde_json::json!({ "auth": a });
        }
        None => {
            if let Some(o) = cfg.get_mut("auths").and_then(|x| x.as_object_mut()) {
                o.remove(host);
                if o.is_empty() {
                    cfg.as_object_mut().map(|m| m.remove("auths"));
                }
            }
        }
    }
    if let Ok(content) = serde_json::to_string_pretty(&cfg) {
        if content == raw {
            return;   // nothing changed (forget() runs on every pull without a license)
        }
        if let Err(e) = write_private(path, &content) {
            eprintln!("[flux] could not save the registry login in the docker config: {e}");
        }
    }
}

/// A one-off DOCKER_CONFIG holding only the registry login, for a CLI pull when there is no isolated
/// config (Windows; macOS with isolation off). The caller removes the directory afterwards. No
/// `docker login`: the user's own config and credential store stay untouched.
pub fn temp_login_config(host: &str, auth_b64: &str) -> Option<PathBuf> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = data_dir().join(format!("pull-login-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let cfg = serde_json::json!({ "auths": { host: { "auth": auth_b64 } } });
    match write_private(&dir.join("config.json"), &cfg.to_string()) {
        Ok(()) => Some(dir),
        Err(_) => {
            let _ = std::fs::remove_dir_all(&dir);
            None
        }
    }
}

/// Remove one-off login configs a killed Flux left behind (they hold the registry login). Startup.
pub fn sweep_temp_login_configs() {
    if let Ok(entries) = std::fs::read_dir(data_dir()) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("pull-login-") {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

/// The isolated config.json: the plugin dirs, plus the `auths` already in the file (registry.rs writes
/// them; rewriting the file on every start used to wipe the login).
fn merged_config(cfg_path: &Path, plugin_dirs: &[String]) -> serde_json::Value {
    let mut cfg = serde_json::json!({ "cliPluginsExtraDirs": plugin_dirs });
    if let Some(auths) = std::fs::read_to_string(cfg_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|old| old.get("auths").cloned())
        .filter(|a| a.as_object().map(|o| !o.is_empty()).unwrap_or(false))
    {
        cfg["auths"] = auths;
    }
    cfg
}

/// Apply PATH + the isolated config/host to a docker command.
pub fn apply(cmd: &mut Command) {
    cmd.env("PATH", crate::augmented_path());
    let env = get();
    if let Some(dir) = &env.config_dir {
        cmd.env("DOCKER_CONFIG", dir);
        // A DOCKER_CONTEXT inherited from the environment would name a context that the
        // isolated config doesn't have.
        cmd.env_remove("DOCKER_CONTEXT");
        if let Some(host) = &env.cli_host {
            cmd.env("DOCKER_HOST", host);
        }
    }
}

#[allow(dead_code)]
fn home() -> PathBuf {
    PathBuf::from(
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string()),
    )
}

/// Endpoint of the user's current docker context, read with their own config (context metadata
/// only — no credential helper is involved). Err = the docker CLI could not be run at all.
fn context_endpoint(bin: &str) -> Result<Option<String>, ()> {
    let out = crate::silent_command(bin)
        .args(["context", "inspect", "--format", "{{.Endpoints.docker.Host}}"])
        .env("PATH", crate::augmented_path())
        .output()
        .map_err(|_| ())?;
    if !out.status.success() {
        return Ok(None);
    }
    let host = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(if host.is_empty() { None } else { Some(host) })
}

fn resolve() -> (DockerEnv, bool) {
    let bin = crate::docker_bin();
    let env_host = std::env::var("DOCKER_HOST").ok().filter(|s| !s.is_empty());
    let endpoint = match &env_host {
        Some(h) => Some(h.clone()),
        None => match context_endpoint(&bin) {
            Ok(h) => h,
            Err(()) => return (DockerEnv::default(), false),
        },
    };
    #[allow(unused_mut)]
    let mut env = DockerEnv { endpoint: endpoint.clone(), ..Default::default() };

    #[cfg(target_os = "macos")]
    if let Some((dir, host)) = isolate(&bin, endpoint.as_deref()) {
        env.config_dir = Some(dir);
        env.cli_host = host;
    }
    (env, true)
}

#[cfg(target_os = "macos")]
fn isolate(bin: &str, endpoint: Option<&str>) -> Option<(PathBuf, Option<String>)> {
    // Only plain unix-socket endpoints: tcp/ssh contexts may carry TLS material stored under the
    // user's ~/.docker, which the isolated config would lose.
    let host = match endpoint {
        Some(h) if h.starts_with("unix://") => Some(h.to_string()),
        Some(_) => return None,
        None => {
            if Path::new("/var/run/docker.sock").exists() {
                None
            } else {
                let sock = home().join(".docker").join("run").join("docker.sock");
                if sock.exists() { Some(format!("unix://{}", sock.to_string_lossy())) } else { None }
            }
        }
    };

    let base = APP_DATA_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| crate::farm_dir().join(".flux"));
    let dir = base.join("docker-config");
    std::fs::create_dir_all(&dir).ok()?;

    // Where the CLI finds `docker compose`: Docker Desktop symlinks its plugins into
    // ~/.docker/cli-plugins, which an isolated DOCKER_CONFIG no longer covers.
    let mut plugin_dirs: Vec<String> = Vec::new();
    if let Ok(raw) = std::fs::read_to_string(home().join(".docker").join("config.json")) {
        if let Ok(user_cfg) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(arr) = user_cfg["cliPluginsExtraDirs"].as_array() {
                plugin_dirs.extend(arr.iter().filter_map(|v| v.as_str().map(String::from)));
            }
        }
    }
    plugin_dirs.push(home().join(".docker").join("cli-plugins").to_string_lossy().into_owned());
    plugin_dirs.push("/Applications/Docker.app/Contents/Resources/cli-plugins".to_string());
    let mut seen = std::collections::HashSet::new();
    plugin_dirs.retain(|d| Path::new(d).is_dir() && seen.insert(d.clone()));

    // Keep the registry login. A non-empty `auths` also stops the CLI from picking a platform
    // credential store (keychain) for this config — the login stays a plain owner-only file.
    let cfg_path = dir.join("config.json");
    let cfg = merged_config(&cfg_path, &plugin_dirs);
    let content = serde_json::to_string_pretty(&cfg).ok()?;
    if std::fs::read_to_string(&cfg_path).ok().as_deref() != Some(content.as_str()) {
        write_private(&cfg_path, &content).ok()?;
    }

    // Prove compose still resolves with the isolated config; otherwise keep the old behaviour.
    let mut check = crate::silent_command(bin);
    check
        .args(["compose", "version"])
        .env("PATH", crate::augmented_path())
        .env("DOCKER_CONFIG", &dir)
        .env_remove("DOCKER_CONTEXT");
    if let Some(h) = &host {
        check.env("DOCKER_HOST", h);
    }
    let ok = check.output().map(|o| o.status.success()).unwrap_or(false);
    if !ok {
        eprintln!("[flux] isolated docker config: compose not found, using the user's config");
        return None;
    }
    Some((dir, host))
}

#[cfg(test)]
mod tests {
    /// Run with FLUX_TEST_APP_DATA=<scratch dir> cargo test -- --ignored docker_env
    #[test]
    #[ignore]
    fn resolves_isolated_env() {
        let dir = std::env::var("FLUX_TEST_APP_DATA").expect("FLUX_TEST_APP_DATA");
        super::set_app_data_dir(dir.into());
        let env = super::get();
        eprintln!("{env:?}");
        let mut cmd = crate::docker_command();
        let out = cmd.args(["compose", "version"]).output().unwrap();
        assert!(out.status.success());
        let out = crate::docker_command().args(["info", "--format", "{{.ServerVersion}}"]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
}

#[cfg(test)]
mod registry_auth_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("flux-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// The isolated config's rewrite on every start must keep the registry login (audit 2026-11-06).
    #[test]
    fn rewriting_keeps_the_auths() {
        let dir = scratch("merge");
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"cliPluginsExtraDirs":["/old"],"auths":{"ghcr.io":{"auth":"dTp0"}}}"#).unwrap();
        let cfg = merged_config(&path, &["/new".to_string()]);
        assert_eq!(cfg["auths"]["ghcr.io"]["auth"], "dTp0");
        assert_eq!(cfg["cliPluginsExtraDirs"][0], "/new");
        std::fs::write(&path, "not json").unwrap();
        assert!(merged_config(&path, &[]).get("auths").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_login_is_added_and_removed_keeping_the_rest() {
        let dir = scratch("auth");
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"cliPluginsExtraDirs":["/p"],"auths":[]}"#).unwrap();   // malformed auths
        set_auth_in(&path, "ghcr.io", Some("dTp0"));
        let cfg = read(&path);
        assert_eq!(cfg["auths"]["ghcr.io"]["auth"], "dTp0");
        assert_eq!(cfg["cliPluginsExtraDirs"][0], "/p");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        set_auth_in(&path, "ghcr.io", None);
        let cfg = read(&path);
        assert!(cfg.get("auths").is_none() && cfg["cliPluginsExtraDirs"][0] == "/p");
        // an unreadable config is not replaced (it would lose the plugin dirs)
        std::fs::write(&path, "garbage").unwrap();
        set_auth_in(&path, "ghcr.io", Some("dTp0"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "garbage");
        // no temp files left behind
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_leftover_loose_temp_never_holds_the_token() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("tmp");
        let path = dir.join("f.json");
        write_private(&path, "secret").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
