//! The environment every docker / docker compose call Flux makes runs with.
//!
//! macOS: the owner's ~/.docker/config.json usually has `"credsStore": "desktop"`, so the docker
//! CLI runs docker-credential-desktop on pulls, which reads Docker Desktop's group container and
//! makes macOS ask "Flux would like to access data from other apps" — every time, because Flux is
//! not signed. The farm images are public, so Flux needs no credentials at all: on macOS its
//! docker calls run with DOCKER_CONFIG pointing at a Flux-owned directory whose config.json only
//! lists where the CLI plugins (compose) live. Without the user's config the CLI would fall back
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

    let cfg = serde_json::json!({ "cliPluginsExtraDirs": plugin_dirs });
    let cfg_path = dir.join("config.json");
    let content = serde_json::to_string_pretty(&cfg).ok()?;
    if std::fs::read_to_string(&cfg_path).ok().as_deref() != Some(content.as_str()) {
        std::fs::write(&cfg_path, content).ok()?;
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
