//! This computer's identity for licence seats (2026-11-19). The farm runs in Docker and can't see the
//! hardware, so Flux AI writes `~/farm/logs/machine.json` at every start: a SHA-256 of the hardware id
//! (never the id itself), the computer's name (shown in the licence's list of computers) and the platform.
//! A farm folder copied to another computer gets that computer's id here, so it needs its own seat.

use sha2::{Digest, Sha256};

/// The hardware id: the Mac's IOPlatformUUID, Windows' MachineGuid, Linux's machine-id.
fn hardware_id() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let out = crate::silent_command("ioreg").args(["-rd1", "-c", "IOPlatformExpertDevice"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().find(|l| l.contains("\"IOPlatformUUID\""))?;
        let id = line.rsplit('"').nth(1)?.trim().to_string();
        return (!id.is_empty()).then_some(id);
    }
    #[cfg(target_os = "windows")]
    {
        let out = crate::silent_command("reg")
            .args(["query", r"HKLM\SOFTWARE\Microsoft\Cryptography", "/v", "MachineGuid"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().find(|l| l.contains("MachineGuid"))?;
        let id = line.split_whitespace().last()?.trim().to_string();
        return (!id.is_empty()).then_some(id);
    }
    #[allow(unreachable_code)]
    {
        let id = std::fs::read_to_string("/etc/machine-id").ok()?.trim().to_string();
        (!id.is_empty()).then_some(id)
    }
}

/// A name people recognise: "Marko's MacBook Pro", the Windows computer name, the hostname.
fn computer_name() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = crate::silent_command("scutil").args(["--get", "ComputerName"]).output() {
            let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    std::env::var("COMPUTERNAME").ok().filter(|s| !s.is_empty())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string()))
        .unwrap_or_else(|| "This computer".to_string())
}

fn platform() -> &'static str {
    if cfg!(target_os = "macos") { "macos" } else if cfg!(target_os = "windows") { "windows" } else { "linux" }
}

pub fn hash(id: &str) -> String {
    let digest = Sha256::digest(format!("flux-ai-seat:{id}").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Write `logs/machine.json` for the farm. Without a hardware id nothing is written and the farm uses a
/// random id of its own.
pub fn write_machine_file() {
    let Some(id) = hardware_id() else { return };
    let logs = crate::farm_dir().join("logs");
    if std::fs::create_dir_all(&logs).is_err() {
        return;
    }
    let body = serde_json::json!({ "machine_hash": hash(&id), "name": computer_name(), "platform": platform() });
    let tmp = logs.join("machine.json.tmp");
    if std::fs::write(&tmp, body.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, logs.join("machine.json"));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_hash_is_stable_and_hides_the_id() {
        let h = super::hash("ABC-123");
        assert_eq!(h.len(), 64);
        assert_eq!(h, super::hash("ABC-123"));
        assert!(!h.contains("ABC"));
        assert_ne!(h, super::hash("ABC-124"));
    }

    #[test]
    fn this_computer_has_a_hardware_id() {
        // Every machine Flux AI runs on (and CI) has one; without it seats fall back to a random id.
        assert!(super::hardware_id().is_some());
    }
}
