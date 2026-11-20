// The two crate-level helpers machine_id.rs uses, then the real file itself.
use std::path::PathBuf;

pub fn silent_command(program: &str) -> std::process::Command {
    std::process::Command::new(program)
}

pub fn farm_dir() -> PathBuf {
    std::env::temp_dir().join("flux-machine-id-check")
}

#[path = "../../../src-tauri/src/machine_id.rs"]
pub mod machine_id;

#[cfg(test)]
mod check {
    #[test]
    fn writes_machine_json_like_flux_does() {
        crate::machine_id::write_machine_file();
        let raw = std::fs::read_to_string(crate::farm_dir().join("logs").join("machine.json"))
            .expect("machine.json written (no hardware id found?)");
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let h = v["machine_hash"].as_str().unwrap();
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!v["name"].as_str().unwrap().is_empty());
        // The hash is safe to show; it is what the licence server sees anyway.
        println!("platform={} name={} hash={}...", v["platform"], v["name"], &h[..12]);
    }
}
