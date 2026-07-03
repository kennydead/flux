import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import Button from "../components/Button";
import appIcon from "../assets/app-icon.png";
import "./HomeScreen.css";

interface Props {
  onStart: () => void;
  isConfigured: boolean;
}

const FEATURES = [
  { icon: "⚡", label: "Multiple AI agents working in parallel" },
  { icon: "🐳", label: "Runs locally in Docker — your code stays private" },
  { icon: "🔧", label: "Built for real software projects" },
];

export default function HomeScreen({ onStart }: Props) {
  const [autostart, setAutostart] = useState(false);
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState(0);
  const [checkingUpdate, setCheckingUpdate] = useState(false);

  useEffect(() => {
    if (!import.meta.env.DEV) {
      invoke<boolean>("get_autostart").then(setAutostart).catch(() => {});
    }

    const checkUpdate = async () => {
      try {
        const update = await check();
        if (update?.available) setUpdateVersion(update.version);
      } catch {}
    };

    checkUpdate();
    const interval = setInterval(checkUpdate, 60 * 60 * 1000);
    return () => clearInterval(interval);
  }, []);

  async function manualCheckUpdate() {
    setCheckingUpdate(true);
    try {
      const update = await check();
      if (update?.available) setUpdateVersion(update.version);
      else setUpdateVersion(null);
    } catch {}
    setCheckingUpdate(false);
  }


  async function installUpdate() {
    try {
      setDownloading(true);
      const update = await check();
      if (!update?.available) return;
      let downloaded = 0;
      let total = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") total = event.data.contentLength ?? 0;
        if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          if (total > 0) setDownloadProgress(Math.round((downloaded / total) * 100));
        }
        if (event.event === "Finished") setDownloadProgress(100);
      });
      await relaunch();
    } catch {
      setDownloading(false);
    }
  }

  function toggleAutostart(e: React.ChangeEvent<HTMLInputElement>) {
    const val = e.target.checked;
    setAutostart(val);
    invoke("set_autostart", { enabled: val }).catch(() => setAutostart(!val));
  }

  return (
    <div className="home">
      {/* Update banner */}
      {updateVersion && (
        <div className="home-update-banner">
          {downloading ? (
            <>
              <span>{downloadProgress < 100 ? `Downloading… ${downloadProgress}%` : "Installing…"}</span>
              <div className="home-update-progress">
                <div className="home-update-progress-bar" style={{ width: `${downloadProgress}%` }} />
              </div>
            </>
          ) : (
            <>
              <span>Update available: {updateVersion}</span>
              <button className="home-update-link" onClick={installUpdate}>
                Install & Restart →
              </button>
            </>
          )}
        </div>
      )}

      {/* Hero */}
      <div className="home-hero">
        <div className="home-glow" />
        <div className="home-icon">
          <img src={appIcon} alt="Flux" className="home-icon-img" />
        </div>

        <h1 className="home-title">Flux</h1>
        <p className="home-tagline">AI Agent Farm</p>
        <p className="home-desc">
          Spin up a team of AI agents that plan, code, and review your software —
          all running locally on your machine.
        </p>
      </div>

      {/* Features */}
      <ul className="home-features">
        {FEATURES.map((f) => (
          <li key={f.label} className="home-feature">
            <span className="home-feature-icon">{f.icon}</span>
            <span>{f.label}</span>
          </li>
        ))}
      </ul>

      {/* CTA */}
      <div className="home-cta">
        <Button onClick={onStart} fullWidth>
          Get Started
        </Button>
      </div>

      {/* Auto-start toggle */}
      {!import.meta.env.DEV && (
        <label className="home-autostart">
          <input
            type="checkbox"
            checked={autostart}
            onChange={toggleAutostart}
          />
          Launch Flux at login
        </label>
      )}

      {/* Footer */}
      <footer className="home-footer">
        <div className="home-version-row">
          <span className="home-version">v0.3.3</span>
          <button className="home-check-update" onClick={manualCheckUpdate} disabled={checkingUpdate}>
            {checkingUpdate ? "Checking…" : "Check for updates"}
          </button>
        </div>
        <div className="home-links">
          <a className="home-link" href="#" onClick={(e) => e.preventDefault()}>Help</a>
          <span className="home-dot">·</span>
          <a className="home-link" href="#" onClick={(e) => e.preventDefault()}>Privacy</a>
          <span className="home-dot">·</span>
          <a className="home-link" href="#" onClick={(e) => e.preventDefault()}>Terms</a>
          <span className="home-dot">·</span>
          <a className="home-link" href="#" onClick={(e) => e.preventDefault()}>Website</a>
        </div>
      </footer>
    </div>
  );
}
