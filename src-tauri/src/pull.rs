//! Farm image pulls with a real download progress bar.
//!
//! Pulls go through the Docker Engine API (bollard), not the `docker pull` CLI:
//!  - the API streams per-layer byte progress (`progressDetail.current/total`); the CLI's non-TTY
//!    output only has status lines, no bytes;
//!  - the API talks to the daemon socket / named pipe directly and sends no registry credentials
//!    (the images are public), so no credential helper ever runs — no macOS privacy prompt;
//!  - it works the same on macOS (unix socket) and Windows (named pipe).
//! The daemon only reports a layer's size once that layer starts downloading (3 at a time), so
//! the overall total comes from the registry manifests (ghcr anonymous token, a few small GETs)
//! fetched up front. If the manifests can't be read, the bar is marked indeterminate and the UI
//! shows its step text instead. If the Engine API can't be reached at all, the pull falls back to
//! the CLI (with the isolated docker config) — the old behaviour, without a bar.

use bollard::query_parameters::CreateImageOptionsBuilder;
use bollard::{Docker, API_DEFAULT_VERSION};
use futures_util::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

pub const PROGRESS_EVENT: &str = "farm-pull-progress";

#[derive(Serialize, Clone, Debug)]
pub struct PullProgress {
    /// "pulling" while images download, "done" when the whole pull is over.
    pub phase: &'static str,
    /// Short image name ("agent", "dashboard-backend", …).
    pub image: String,
    pub index: usize,
    pub count: usize,
    /// Bytes downloaded so far / bytes to download, over ALL images of this pull.
    pub downloaded: u64,
    pub total: u64,
    /// false = the total isn't known: show text, not a bar.
    pub determinate: bool,
}

/// What the registry says an image (for the daemon's platform) is made of.
#[derive(Clone, Debug, Default)]
struct RemoteImage {
    /// Digest of the tag's manifest / index — matches a local RepoDigest when up to date.
    digest: Option<String>,
    /// (short layer id as the daemon reports it, compressed size)
    layers: Vec<(String, u64)>,
}

#[derive(Default, Clone, Debug)]
struct LayerState {
    size: Option<u64>,
    done: u64,
    exists: bool,
}

/// Split "host/repo/name:tag" → ("host/repo/name", "tag"); no tag → "latest".
fn split_ref(image: &str) -> (String, String) {
    let slash = image.rfind('/').map(|i| i + 1).unwrap_or(0);
    match image[slash..].rfind(':') {
        Some(i) => (image[..slash + i].to_string(), image[slash + i + 1..].to_string()),
        None => (image.to_string(), "latest".to_string()),
    }
}

fn short_name(image: &str) -> String {
    let (name, _) = split_ref(image);
    name.rsplit('/').next().unwrap_or(&name).to_string()
}

/// Layer sizes from ghcr.io (anonymous pull token — the farm images are public). Blocking.
fn fetch_remote(image: &str, os: &str, arch: &str) -> Option<RemoteImage> {
    let (name, tag) = split_ref(image);
    let repo = name.strip_prefix("ghcr.io/")?;
    let timeout = Duration::from_secs(8);
    let token: serde_json::Value = ureq::get(&format!("https://ghcr.io/token?scope=repository:{repo}:pull"))
        .timeout(timeout)
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let auth = format!("Bearer {}", token["token"].as_str()?);
    let accept = "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json";
    let get = |reference: &str| -> Option<(Option<String>, serde_json::Value)> {
        let resp = ureq::get(&format!("https://ghcr.io/v2/{repo}/manifests/{reference}"))
            .set("Authorization", &auth)
            .set("Accept", accept)
            .timeout(timeout)
            .call()
            .ok()?;
        let digest = resp.header("docker-content-digest").map(String::from);
        Some((digest, resp.into_json().ok()?))
    };
    let (digest, mut manifest) = get(&tag)?;
    if let Some(list) = manifest["manifests"].as_array() {
        let pick = list.iter().find(|m| {
            m["platform"]["os"].as_str() == Some(os) && m["platform"]["architecture"].as_str() == Some(arch)
        })?;
        let (_, platform_manifest) = get(pick["digest"].as_str()?)?;
        manifest = platform_manifest;
    }
    let layers = manifest["layers"]
        .as_array()?
        .iter()
        .filter_map(|l| {
            let d = l["digest"].as_str()?;
            let hex = d.split(':').nth(1)?;
            Some((hex.chars().take(12).collect::<String>(), l["size"].as_u64()?))
        })
        .collect();
    Some(RemoteImage { digest, layers })
}

/// Overall progress over every image in one pull.
struct Tracker {
    app: Option<AppHandle>,
    count: usize,
    /// Expected download size per image (None = unknown); Some(0) = already up to date.
    expected: Vec<Option<u64>>,
    /// Bytes downloaded by images that already finished.
    finished: u64,
    current: usize,
    layers: HashMap<String, LayerState>,
    /// Layers we have no size for yet (not in the manifest, not downloading yet).
    current_known: bool,
    last_emit: Option<Instant>,
}

impl Tracker {
    fn start_image(&mut self, index: usize, remote: Option<&RemoteImage>) {
        self.current = index;
        self.layers.clear();
        self.current_known = remote.is_some();
        // Up to date: the daemon reports no layer statuses — don't count its layers at all.
        if self.expected[index] == Some(0) {
            return;
        }
        if let Some(r) = remote {
            for (id, size) in &r.layers {
                self.layers.insert(id.clone(), LayerState { size: Some(*size), ..Default::default() });
            }
        }
    }

    fn update(&mut self, id: &str, status: &str, current: Option<i64>, total: Option<i64>) {
        const LAYER_STATUSES: [&str; 8] = [
            "Pulling fs layer", "Waiting", "Downloading", "Verifying Checksum",
            "Download complete", "Extracting", "Pull complete", "Already exists",
        ];
        if id.is_empty() || !LAYER_STATUSES.iter().any(|s| status.starts_with(s)) {
            return;
        }
        let layer = self.layers.entry(id.to_string()).or_default();
        match status {
            s if s.starts_with("Already exists") => layer.exists = true,
            s if s.starts_with("Downloading") => {
                if layer.size.is_none() {
                    layer.size = total.filter(|t| *t > 0).map(|t| t as u64);
                }
                if let Some(c) = current {
                    layer.done = (c.max(0) as u64).min(layer.size.unwrap_or(u64::MAX));
                }
            }
            s if s.starts_with("Verifying Checksum")
                || s.starts_with("Download complete")
                || s.starts_with("Extracting")
                || s.starts_with("Pull complete") =>
            {
                if let Some(size) = layer.size {
                    layer.done = size;
                }
            }
            _ => {}
        }
    }

    fn finish_image(&mut self) {
        let (done, _, _) = self.current_totals();
        self.finished += done;
        self.expected[self.current] = Some(done);
        self.layers.clear();
        self.current_known = true;
    }

    /// (downloaded, total, known) for the current image.
    fn current_totals(&self) -> (u64, u64, bool) {
        let mut done = 0;
        let mut total = 0;
        let mut known = self.current_known;
        for l in self.layers.values().filter(|l| !l.exists) {
            done += l.done;
            match l.size {
                Some(s) => total += s,
                None => known = false,
            }
        }
        (done, total.max(done), known)
    }

    fn emit(&mut self, phase: &'static str, image: &str, force: bool) {
        let Some(app) = &self.app else { return };
        if !force && self.last_emit.map(|t| t.elapsed() < Duration::from_millis(250)).unwrap_or(false) {
            return;
        }
        self.last_emit = Some(Instant::now());
        let (cur_done, cur_total, cur_known) = self.current_totals();
        let mut total = self.finished + cur_total;
        let mut determinate = cur_known;
        for e in self.expected.iter().skip(self.current + 1) {
            match e {
                Some(v) => total += v,
                None => determinate = false,
            }
        }
        let downloaded = (self.finished + cur_done).min(total);
        let _ = app.emit(
            PROGRESS_EVENT,
            PullProgress {
                phase,
                image: image.to_string(),
                index: self.current,
                count: self.count,
                downloaded,
                total,
                determinate: determinate && total > 0,
            },
        );
    }
}

async fn connect() -> Result<Docker, String> {
    let env = tauri::async_runtime::spawn_blocking(crate::docker_env::get)
        .await
        .map_err(|e| e.to_string())?;
    let endpoint = env.cli_host.or(env.endpoint);
    let docker = match endpoint.as_deref() {
        #[cfg(unix)]
        Some(h) if h.starts_with("unix://") => Docker::connect_with_unix(h, 120, API_DEFAULT_VERSION),
        #[cfg(windows)]
        Some(h) if h.starts_with("npipe://") => Docker::connect_with_named_pipe(h, 120, API_DEFAULT_VERSION),
        Some(other) => return Err(format!("unsupported docker endpoint {other}")),
        None => Docker::connect_with_local_defaults(),
    }
    .map_err(|e| e.to_string())?;
    // Docker Desktop may run an older API than bollard's default — negotiate down.
    docker.negotiate_version().await.map_err(|e| e.to_string())
}

/// One image through the Engine API. Returns whether the local image changed.
async fn pull_api(docker: &Docker, image: &str, tracker: &mut Tracker) -> Result<bool, String> {
    let before = docker.inspect_image(image).await.ok().and_then(|i| i.id);
    let (name, tag) = split_ref(image);
    let opts = CreateImageOptionsBuilder::default().from_image(&name).tag(&tag).build();
    let mut stream = docker.create_image(Some(opts), None, None);
    let short = short_name(image);
    let mut newer = false;
    while let Some(item) = stream.next().await {
        let info = item.map_err(|e| e.to_string())?;
        let status = info.status.unwrap_or_default();
        if status.contains("Downloaded newer image") {
            newer = true;
        }
        let (cur, tot) = info
            .progress_detail
            .map(|p| (p.current, p.total))
            .unwrap_or((None, None));
        tracker.update(info.id.as_deref().unwrap_or(""), &status, cur, tot);
        tracker.emit("pulling", &short, false);
    }
    let after = docker.inspect_image(image).await.ok().and_then(|i| i.id);
    Ok(match (before, after) {
        (Some(b), Some(a)) => a != b,
        (None, Some(_)) => true,
        _ => newer,
    })
}

/// The old way: `docker pull` through the CLI (isolated config). Blocking.
fn pull_cli(image: &str) -> Result<bool, String> {
    let output = crate::docker_command()
        .args(["pull", image])
        .output()
        .map_err(|e| format!("Failed to pull {image}: {e}"))?;
    if output.status.success() {
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(combined.contains("Downloaded newer image") || combined.contains("Pull complete"))
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

/// Pull `images` in order, emitting one overall progress stream. Returns true when any image
/// changed locally.
pub async fn pull_images(app: Option<AppHandle>, images: Vec<String>) -> Result<bool, String> {
    let docker = match connect().await {
        Ok(d) => Some(d),
        Err(e) => {
            eprintln!("[flux] Engine API unavailable ({e}); pulling with the docker CLI");
            None
        }
    };

    // Expected download sizes, from the registry, for the daemon's platform.
    let mut remotes: Vec<Option<RemoteImage>> = vec![None; images.len()];
    let mut expected: Vec<Option<u64>> = vec![None; images.len()];
    if let Some(d) = &docker {
        if let Ok(v) = d.version().await {
            let os = v.os.unwrap_or_else(|| "linux".into());
            let arch = v.arch.unwrap_or_else(|| "amd64".into());
            for (i, img) in images.iter().enumerate() {
                let (img2, os2, arch2) = (img.clone(), os.clone(), arch.clone());
                let remote = tauri::async_runtime::spawn_blocking(move || fetch_remote(&img2, &os2, &arch2))
                    .await
                    .ok()
                    .flatten();
                if let Some(r) = &remote {
                    // Already have exactly this tag → nothing to download for it.
                    let local = d.inspect_image(img).await.ok();
                    let up_to_date = match (&r.digest, local.and_then(|l| l.repo_digests)) {
                        (Some(dg), Some(rds)) => rds.iter().any(|rd| rd.ends_with(&format!("@{dg}"))),
                        _ => false,
                    };
                    expected[i] = Some(if up_to_date { 0 } else { r.layers.iter().map(|(_, s)| s).sum() });
                }
                remotes[i] = remote;
            }
        }
    }

    let mut tracker = Tracker {
        app,
        count: images.len(),
        expected,
        finished: 0,
        current: 0,
        layers: HashMap::new(),
        current_known: false,
        last_emit: None,
    };

    let mut any_updated = false;
    for (i, img) in images.iter().enumerate() {
        let short = short_name(img);
        tracker.start_image(i, remotes[i].as_ref());
        tracker.emit("pulling", &short, true);
        let api_result = match &docker {
            Some(d) => Some(pull_api(d, img, &mut tracker).await),
            None => None,
        };
        let updated = match api_result {
            Some(Ok(u)) => u,
            other => {
                if let Some(Err(e)) = other {
                    eprintln!("[flux] Engine API pull of {img} failed ({e}); retrying with the docker CLI");
                }
                // No byte progress from the CLI: this image's share becomes unknown.
                tracker.current_known = false;
                tracker.emit("pulling", &short, true);
                let img2 = img.clone();
                tauri::async_runtime::spawn_blocking(move || pull_cli(&img2))
                    .await
                    .map_err(|e| e.to_string())??
            }
        };
        any_updated |= updated;
        tracker.finish_image();
        tracker.emit("pulling", &short, true);
    }
    tracker.emit("done", "", true);
    Ok(any_updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_refs() {
        assert_eq!(
            split_ref("ghcr.io/kennydead/claude-agent-farm/agent:staging"),
            ("ghcr.io/kennydead/claude-agent-farm/agent".into(), "staging".into())
        );
        assert_eq!(split_ref("localhost:5000/x"), ("localhost:5000/x".into(), "latest".into()));
        assert_eq!(short_name("ghcr.io/a/b/dashboard-backend:latest"), "dashboard-backend");
    }

    fn tracker(expected: Vec<Option<u64>>) -> Tracker {
        Tracker {
            app: None, count: expected.len(), expected, finished: 0, current: 0,
            layers: HashMap::new(), current_known: false, last_emit: None,
        }
    }

    #[test]
    fn tracks_bytes_and_existing_layers() {
        let remote = RemoteImage { digest: None, layers: vec![("aaa".into(), 100), ("bbb".into(), 50)] };
        let mut t = tracker(vec![Some(150), Some(0)]);
        t.start_image(0, Some(&remote));
        assert_eq!(t.current_totals(), (0, 150, true));
        t.update("bbb", "Already exists", None, None);
        t.update("aaa", "Downloading", Some(40), Some(100));
        assert_eq!(t.current_totals(), (40, 100, true));
        t.update("aaa", "Pull complete", None, None);
        assert_eq!(t.current_totals(), (100, 100, true));
        t.update("latest", "Pulling from kennydead/x", None, None);
        assert_eq!(t.layers.len(), 2);
        t.finish_image();
        assert_eq!(t.finished, 100);
        // up-to-date second image: contributes nothing
        t.start_image(1, Some(&remote));
        assert_eq!(t.current_totals(), (0, 0, true));
    }

    #[test]
    fn unknown_layer_makes_it_indeterminate() {
        let mut t = tracker(vec![None]);
        t.start_image(0, None);
        t.update("ccc", "Pulling fs layer", None, None);
        assert!(!t.current_totals().2);
    }

    /// Read-only check against the local Docker daemon: connect + negotiate + inspect.
    #[test]
    #[ignore]
    fn engine_api_reachable() {
        tauri::async_runtime::block_on(async {
            let d = connect().await.expect("connect");
            let v = d.version().await.expect("version");
            eprintln!("daemon {:?}/{:?} api {:?}", v.os, v.arch, v.api_version);
            let r = tauri::async_runtime::spawn_blocking(|| {
                fetch_remote("ghcr.io/kennydead/claude-agent-farm/agent:latest", "linux", "arm64")
            }).await.unwrap().expect("manifest");
            eprintln!("agent:latest layers {} total {} MB", r.layers.len(), r.layers.iter().map(|l| l.1).sum::<u64>() / 1_000_000);
        });
    }
}
