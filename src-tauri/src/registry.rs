//! Login for the farm's private images on ghcr.io (owner 2026-11-06).
//!
//! The farm's images are going private so only licensed installs can download them. The license
//! server hands a valid license a read-only login (`registry: {username, token}` in its answer); Flux
//! asks for it with the key saved at first start (~/farm/logs/license_key.txt), then:
//!  - passes it to its Engine API pulls (bollard), which never read docker's config;
//!  - runs `docker login ghcr.io` with Flux's isolated DOCKER_CONFIG (docker_command), so the CLI
//!    fallback and every `docker compose` call can pull too.
//! No key, no login in the answer (an older license server), or the server unreachable → None: pulls
//! stay anonymous, exactly as before, and a login from an earlier run is still in the isolated config.
//! The user never sees or types any of this.

use std::io::Write;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const HOST: &str = "ghcr.io";
const FRESH_FOR: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, PartialEq)]
pub struct Login {
    pub username: String,
    pub token: String,
}

static CACHE: Mutex<Option<(Login, Instant)>> = Mutex::new(None);

/// The `registry` part of a license-server answer, when it carries a usable login.
pub fn from_answer(body: &serde_json::Value) -> Option<Login> {
    let r = body.get("registry")?;
    let username = r.get("username")?.as_str()?.trim().to_string();
    let token = r.get("token")?.as_str()?.trim().to_string();
    if username.is_empty() || token.is_empty() || body.get("valid").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    Some(Login { username, token })
}

fn license_key() -> Option<String> {
    let key = std::fs::read_to_string(crate::farm_dir().join("logs").join("license_key.txt")).ok()?;
    let key = key.trim().to_string();
    (!key.is_empty()).then_some(key)
}

fn ask_license_server(key: &str) -> Option<Login> {
    let res = ureq::post(crate::LICENSE_ENDPOINT)
        .timeout(Duration::from_secs(8))
        .send_json(serde_json::json!({ "key": key }))
        .ok()?;
    let body: serde_json::Value = res.into_json().ok()?;
    from_answer(&body)
}

/// `docker login ghcr.io` in Flux's isolated docker config; the token goes on stdin, never argv.
fn docker_login(login: &Login) -> bool {
    let child = crate::docker_command()
        .args(["login", HOST, "-u", &login.username, "--password-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else { return false };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(login.token.as_bytes());
    }
    match child.wait_with_output() {
        Ok(out) if out.status.success() => true,
        Ok(out) => {
            eprintln!("[flux] docker login to {HOST} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
            false
        }
        Err(e) => {
            eprintln!("[flux] docker login to {HOST} could not run: {e}");
            false
        }
    }
}

/// The login for this session (asked at most every 30 minutes), with the docker CLI logged in too.
/// Blocking: call it from spawn_blocking.
pub fn ensure() -> Option<Login> {
    if let Ok(guard) = CACHE.lock() {
        if let Some((login, at)) = guard.as_ref() {
            if at.elapsed() < FRESH_FOR {
                return Some(login.clone());
            }
        }
    }
    let login = ask_license_server(&license_key()?)?;
    docker_login(&login);
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some((login.clone(), Instant::now()));
    }
    Some(login)
}

/// "Basic …" header value for the ghcr.io token endpoint.
pub fn basic_auth(login: &Login) -> String {
    format!("Basic {}", base64(format!("{}:{}", login.username, login.token).as_bytes()))
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_answer_with_a_login() {
        let body = serde_json::json!({"valid": true, "registry": {"host": "ghcr.io", "username": "kennydead", "token": "ghp_x"}});
        assert_eq!(from_answer(&body), Some(Login { username: "kennydead".into(), token: "ghp_x".into() }));
    }

    #[test]
    fn no_login_from_an_old_server_or_an_invalid_license() {
        assert_eq!(from_answer(&serde_json::json!({"valid": true})), None);
        let invalid = serde_json::json!({"valid": false, "registry": {"username": "u", "token": "t"}});
        assert_eq!(from_answer(&invalid), None);
        let empty = serde_json::json!({"valid": true, "registry": {"username": "", "token": "t"}});
        assert_eq!(from_answer(&empty), None);
    }

    #[test]
    fn basic_auth_is_standard_base64() {
        let l = Login { username: "Aladdin".into(), token: "open sesame".into() };
        assert_eq!(basic_auth(&l), "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
    }
}
