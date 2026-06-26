import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
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

  useEffect(() => {
    if (import.meta.env.DEV) return;
    invoke<boolean>("get_autostart").then(setAutostart).catch(() => {});

    const checkUpdate = () =>
      invoke<string | null>("check_for_update").then(setUpdateVersion).catch(() => {});

    checkUpdate();
    const interval = setInterval(checkUpdate, 60 * 60 * 1000); // every hour
    return () => clearInterval(interval);
  }, []);

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
          <span>Update available: {updateVersion}</span>
          <a
            className="home-update-link"
            href="#"
            onClick={(e) => {
              e.preventDefault();
              openUrl("https://github.com/kennydead/claude-agent-farm-dist/releases/latest").catch(() => {});
            }}
          >
            Download →
          </a>
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
        <span className="home-version">v0.1.1</span>
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
