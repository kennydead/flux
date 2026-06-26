import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import Layout from "../components/Layout";
import Button from "../components/Button";
import "./Screen.css";

interface Props {
  onBack: () => void;
}

export default function RunningScreen({ onBack }: Props) {
  const [checking, setChecking] = useState(true);
  const [alive, setAlive] = useState(true);

  // Open dashboard in system browser on first load
  useEffect(() => {
    openUrl("http://localhost:5174");
  }, []);

  // Poll health so we can detect if farm stops
  useEffect(() => {
    const id = setInterval(async () => {
      const ok = await invoke<boolean>("check_farm_running").catch(() => false);
      setAlive(ok);
      setChecking(false);
    }, 5000);
    return () => clearInterval(id);
  }, []);

  return (
    <Layout>
      <div className="screen">
        <div className="screen-header">
          <h1>Farm Running</h1>
          <p className="screen-subtitle">
            Your agent farm is up. The dashboard opened in your browser.
          </p>
        </div>

        <div className="screen-body">
          <div className="docker-card">
            <div className="status-row">
              {checking ? (
                <span className="status-spinner" />
              ) : (
                <span className={alive ? "step-check" : "step-error-icon"}>
                  {alive ? "✓" : "✕"}
                </span>
              )}
              <span className="status-text">
                {checking ? "Checking…" : alive ? "Dashboard healthy" : "Dashboard not responding"}
              </span>
            </div>
          </div>
        </div>

        <Button onClick={() => openUrl("http://localhost:5174")} fullWidth>
          Open Dashboard
        </Button>
        <Button variant="ghost" onClick={onBack} fullWidth>
          Back
        </Button>
      </div>
    </Layout>
  );
}
