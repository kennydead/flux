//! Login for the farm's private images on ghcr.io (owner 2026-11-06).
//!
//! The farm's images are going private so only licensed installs can download them. The license
//! server hands a valid license a read-only login (`registry: {username, token}` in its answer); Flux
//! asks for it with the key saved at first start (~/farm/logs/license_key.txt), then:
//!  - passes it to its Engine API pulls (bollard), which never read docker's config;
//!  - writes it into Flux's ISOLATED docker config (`auths`, an owner-only file — no `docker login`,
//!    so no keychain / credential manager and no prompt), so the CLI fallback and `docker compose`
//!    can pull too. The user's own docker config is never touched (docker_env::set_registry_auth).
//! The last good login is saved (owner-only) and used while the license server can't be reached (for
//! up to SAVED_FOR); an explicit "license not valid" answer removes it, and so do no key and no login
//! in a valid answer (an older server, or images public again) → pulls stay anonymous, exactly as
//! before. A login ghcr.io rejects (rotated) is asked for again, then dropped for an anonymous pull
//! (pull.rs). The user never sees or types any of this.
//! (Audit 2026-11-06: `docker login` went to the macOS keychain and to the user's own config on
//! Windows, the isolated config's rewrite on every start wiped it, and it had no timeout.)
//!
//! Since 2026-11-23 (owner: a download login per licence) Flux asks for its licence's OWN login (`registry: 2`).
//! A new licence server answers with our registry (`host`, a mirror of ghcr.io): images are downloaded from there
//! and also given their usual ghcr.io names (pull.rs), so nothing else changes. An older server's answer still
//! means the shared ghcr.io login.

use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const HOST: &str = "ghcr.io";
/// Our registry, the only other place Flux downloads from (owner 2026-11-23): a licence answer naming any other
/// host is ignored, so even a fake licence server can't send downloads elsewhere. Moving the registry = a Flux release.
pub const OUR_REGISTRY: &str = "registry.flux-ai.si";
const FRESH_FOR: Duration = Duration::from_secs(30 * 60);
const SAVED: &str = "registry-login.json";
/// How long a saved login stands in for the license server (a blocked server must not keep a revoked
/// install pulling forever).
const SAVED_FOR: Duration = Duration::from_secs(14 * 24 * 3600);

#[derive(Clone, PartialEq)]
pub struct Login {
    pub username: String,
    pub token: String,
    /// Where the login works: HOST itself, or our registry (a mirror of HOST).
    pub host: String,
}

impl Login {
    /// Where to download a ghcr.io image from: our registry's copy when the login is for it, else the image itself.
    pub fn source(&self, image: &str) -> String {
        match image.strip_prefix("ghcr.io/") {
            Some(rest) if self.host != HOST => format!("{}/{rest}", self.host),
            _ => image.to_string(),
        }
    }
}

/// A registry host name we accept from the licence server ("name.tld" or "name:port").
fn plausible_host(h: &str) -> bool {
    !h.is_empty() && h.len() <= 253 && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':')
        && !h.starts_with('.') && !h.starts_with('-')
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Login {{ username: {:?}, token: <redacted>, host: {:?} }}", self.username, self.host)
    }
}

/// What the license server said about the registry login.
#[derive(Debug, PartialEq)]
pub enum Answer {
    Login(Login),
    /// Valid, but no login in the answer (an older server): anonymous pulls.
    NoLogin,
    /// The license is not valid (revoked, expired, unknown): forget any saved login.
    Invalid,
}

static CACHE: Mutex<Option<(Login, Instant)>> = Mutex::new(None);
/// Whether the last question reached the licence server (audit 2026-11-23: a refused pull while it was down must
/// not throw the saved login away — our registry can't check logins then either).
static REACHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn server_reached() -> bool {
    REACHED.load(std::sync::atomic::Ordering::Relaxed)
}

/// The `registry` part of a license-server answer, when it carries a usable login.
#[cfg(test)]
pub fn from_answer(body: &serde_json::Value) -> Option<Login> {
    match classify(body) {
        Answer::Login(l) => Some(l),
        _ => None,
    }
}

pub fn classify(body: &serde_json::Value) -> Answer {
    if body.get("valid").and_then(|v| v.as_bool()) != Some(true) {
        return Answer::Invalid;
    }
    let get = |k: &str| body.get("registry").and_then(|r| r.get(k)).and_then(|v| v.as_str()).map(|v| v.trim().to_string());
    let reg = body.get("registry");
    let own = reg.and_then(|r| r.get("v")).and_then(|v| v.as_u64()) == Some(2)
        && reg.and_then(|r| r.get("mirror_of")).and_then(|v| v.as_str()) == Some(HOST);
    let host = match get("host") {
        Some(h) if own && h == OUR_REGISTRY => h,
        _ => HOST.to_string(),
    };
    match (get("username"), get("token")) {
        (Some(username), Some(token)) if !username.is_empty() && !token.is_empty() => Answer::Login(Login { username, token, host }),
        _ => Answer::NoLogin,
    }
}

fn license_key() -> Option<String> {
    let key = std::fs::read_to_string(crate::farm_dir().join("logs").join("license_key.txt")).ok()?;
    let key = key.trim().to_string();
    (!key.is_empty()).then_some(key)
}

/// None = the server could not be reached, answered an HTTP error, or something unreadable (keep what we have).
fn ask_license_server(key: &str) -> Option<Answer> {
    let res = match ureq::post(crate::LICENSE_ENDPOINT)
        .timeout(Duration::from_secs(8))
        .send_json(serde_json::json!({ "key": key, "registry": 2 }))
    {
        Ok(r) => r,
        // Any HTTP error is the server's problem, not a verdict: a misdeployed function once answered
        // 401 to every install (2026-07-06). Only an explicit `valid: false` revokes the saved login.
        Err(_) => return None,
    };
    let body: serde_json::Value = res.into_json().ok()?;
    Some(classify(&body))
}

fn saved_path() -> std::path::PathBuf {
    crate::docker_env::data_dir().join(SAVED)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The host of the saved login whatever its age (cleanup: an old login must go from the docker config too).
fn saved_host_any_age() -> Option<String> {
    let raw = std::fs::read_to_string(saved_path()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("host").and_then(|h| h.as_str()).filter(|h| plausible_host(h)).map(String::from)
}

fn load_saved() -> Option<Login> {
    let raw = std::fs::read_to_string(saved_path()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let saved_at = v.get("saved_at")?.as_u64()?;
    if now_secs().saturating_sub(saved_at) > SAVED_FOR.as_secs() {
        return None;
    }
    let (u, t) = (v.get("username")?.as_str()?, v.get("token")?.as_str()?);
    let host = v.get("host").and_then(|h| h.as_str()).filter(|h| *h == OUR_REGISTRY).unwrap_or(HOST);
    (!u.is_empty() && !t.is_empty()).then(|| Login { username: u.into(), token: t.into(), host: host.into() })
}

fn save(login: &Login) {
    let dir = crate::docker_env::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let body = serde_json::json!({ "username": login.username, "token": login.token, "host": login.host, "saved_at": now_secs() }).to_string();
    if let Err(e) = crate::docker_env::write_private(&saved_path(), &body) {
        eprintln!("[flux] could not save the registry login: {e}");
    }
}

/// Remove the login everywhere (saved file, isolated docker config, this session). Also on reset.
pub fn forget() {
    let mut hosts = vec![HOST.to_string()];
    if let Some(h) = saved_host_any_age() {
        hosts.push(h);
    }
    if let Ok(mut g) = CACHE.lock() {
        if let Some((l, _)) = g.take() {
            hosts.push(l.host);
        }
    }
    let _ = std::fs::remove_file(saved_path());
    hosts.dedup();
    for h in hosts {
        crate::docker_env::set_registry_auth(&h, None);
    }
}

fn remember(login: &Login) {
    // A login that moved to another host (ghcr.io → our registry) leaves nothing behind at the old one.
    let old = saved_host_any_age().filter(|h| *h != login.host);
    save(login);
    if let Some(h) = old {
        crate::docker_env::set_registry_auth(&h, None);
    }
    crate::docker_env::set_registry_auth(&login.host, Some(&auth_b64(login)));
    if let Ok(mut g) = CACHE.lock() {
        *g = Some((login.clone(), Instant::now()));
    }
}

/// The login for this session (asked at most every 30 minutes), also written into Flux's isolated
/// docker config. Blocking: call it from spawn_blocking.
pub fn ensure() -> Option<Login> {
    if let Ok(guard) = CACHE.lock() {
        if let Some((login, at)) = guard.as_ref() {
            if at.elapsed() < FRESH_FOR {
                return Some(login.clone());
            }
        }
    }
    let Some(key) = license_key() else {
        forget();   // no license on this machine (reset) → no login
        return None;
    };
    let answer = ask_license_server(&key);
    REACHED.store(answer.is_some(), std::sync::atomic::Ordering::Relaxed);
    match answer {
        Some(Answer::Login(login)) => {
            remember(&login);
            Some(login)
        }
        Some(Answer::Invalid) => {
            forget();
            None
        }
        // Valid but no login handed out (older server, or images public again): stop sending the old one.
        Some(Answer::NoLogin) => {
            forget();
            None
        }
        // Unreachable: keep updating with the last good login (only an explicit "not valid" revokes it).
        // Past its age it goes everywhere, the isolated docker config included.
        None => {
            let saved = load_saved();
            if saved.is_none() {
                forget();
            }
            saved
        }
    }
}

/// Ask the license server again now (ghcr.io rejected the login we had — e.g. a rotated token).
pub fn refresh() -> Option<Login> {
    if let Ok(mut g) = CACHE.lock() {
        *g = None;
    }
    ensure()
}

/// base64("user:token"), the form docker configs store.
pub fn auth_b64(login: &Login) -> String {
    base64(format!("{}:{}", login.username, login.token).as_bytes())
}

/// "Basic …" header value for the ghcr.io token endpoint.
pub fn basic_auth(login: &Login) -> String {
    format!("Basic {}", auth_b64(login))
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
        assert_eq!(from_answer(&body), Some(Login { username: "kennydead".into(), token: "ghp_x".into(), host: "ghcr.io".into() }));
    }

    #[test]
    fn answers_are_told_apart() {
        assert_eq!(classify(&serde_json::json!({"valid": false})), Answer::Invalid);
        assert_eq!(classify(&serde_json::json!({"valid": "yes"})), Answer::Invalid);
        assert_eq!(classify(&serde_json::json!({})), Answer::Invalid);
        assert_eq!(classify(&serde_json::json!({"valid": true})), Answer::NoLogin);
        assert_eq!(classify(&serde_json::json!({"valid": true, "registry": {"username": 5, "token": "t"}})), Answer::NoLogin);
    }

    #[test]
    fn the_token_never_prints() {
        let l = Login { username: "kennydead".into(), token: "ghp_secret".into(), host: HOST.into() };
        let shown = format!("{l:?} {:?}", Answer::Login(l.clone()));
        assert!(!shown.contains("ghp_secret") && shown.contains("kennydead"));
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
    fn a_licence_server_with_our_registry_gives_its_host() {
        let body = serde_json::json!({"valid": true, "registry": {"v": 2, "host": "registry.flux-ai.si", "mirror_of": "ghcr.io",
                                                                  "username": "lic-0123", "token": "t"}});
        let l = from_answer(&body).unwrap();
        assert_eq!(l.host, "registry.flux-ai.si");
        assert_eq!(l.source("ghcr.io/kennydead/claude-agent-farm/agent:latest"), "registry.flux-ai.si/kennydead/claude-agent-farm/agent:latest");
        assert_eq!(l.source("postgres:16-alpine"), "postgres:16-alpine");
    }

    #[test]
    fn only_a_proper_answer_moves_the_downloads() {
        for reg in [
            serde_json::json!({"host": "registry.flux-ai.si", "username": "u", "token": "t"}),                       // no v: 2
            serde_json::json!({"v": 2, "host": "registry.flux-ai.si", "mirror_of": "docker.io", "username": "u", "token": "t"}),
            serde_json::json!({"v": 2, "host": "evil.example/../x", "mirror_of": "ghcr.io", "username": "u", "token": "t"}),
            serde_json::json!({"v": 2, "host": "registry.evil.example", "mirror_of": "ghcr.io", "username": "u", "token": "t"}),
        ] {
            let l = from_answer(&serde_json::json!({"valid": true, "registry": reg})).unwrap();
            assert_eq!(l.host, HOST);
            assert_eq!(l.source("ghcr.io/kennydead/claude-agent-farm/agent:latest"), "ghcr.io/kennydead/claude-agent-farm/agent:latest");
        }
    }

    #[test]
    fn basic_auth_is_standard_base64() {
        let l = Login { username: "Aladdin".into(), token: "open sesame".into(), host: HOST.into() };
        assert_eq!(basic_auth(&l), "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
    }
}
