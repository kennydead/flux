import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import Layout from "../components/Layout";
import Button from "../components/Button";
import "./Screen.css";

interface Props {
  onReady: () => void;
}

type DockerState = "checking" | "no-wsl" | "not-found" | "not-running" | "ready";

async function checkDocker(): Promise<DockerState> {
  const wslOk = await invoke<boolean>("check_wsl_installed");
  if (!wslOk) return "no-wsl";

  try {
    const out = await invoke<string>("run_command", {
      program: "docker",
      args: ["info", "--format", "{{.ServerVersion}}"],
    });
    return out.trim() ? "ready" : "not-running";
  } catch (e: any) {
    const msg = String(e).toLowerCase();
    if (msg.includes("not found") || msg.includes("no such file")) return "not-found";
    return "not-running";
  }
}

export default function DockerScreen({ onReady }: Props) {
  const [state, setState] = useState<DockerState>("checking");

  useEffect(() => {
    let active = true;
    async function poll() {
      while (active) {
        const s = await checkDocker();
        if (!active) break;
        setState(s);
        if (s === "ready") { onReady(); return; }
        await new Promise((r) => setTimeout(r, 3000));
      }
    }
    poll();
    return () => { active = false; };
  }, []);

  return (
    <Layout>
      <div className="screen">
        <div className="screen-header">
          <h1>Docker Required</h1>
          <p className="screen-subtitle">
            Flux uses Docker to run your AI agents securely.
          </p>
        </div>

        <div className="screen-body">
          {state === "checking" && (
            <div className="status-row">
              <span className="status-spinner" />
              <span className="status-text">Checking for Docker…</span>
            </div>
          )}
          {state === "no-wsl" && (
            <div className="docker-card">
              <p><strong>Step 1:</strong> Install WSL (Windows Subsystem for Linux)</p>
              <p className="screen-footnote" style={{ marginTop: 8 }}>
                Open <strong>PowerShell as Administrator</strong> and run:
              </p>
              <code className="screen-code">wsl --install --no-distribution</code>
              <p className="screen-footnote" style={{ marginTop: 8 }}>
                Then <strong>restart your computer</strong> and reopen Flux.
              </p>
            </div>
          )}
          {state === "not-found" && (
            <div className="docker-card">
              <p><strong>Step 2:</strong> Install Docker Desktop</p>
              <Button
                variant="ghost"
                onClick={() => openUrl("https://www.docker.com/products/docker-desktop/")}
              >
                Download Docker Desktop →
              </Button>
              <p className="screen-footnote">After installing and starting Docker, Flux will continue automatically.</p>
            </div>
          )}
          {state === "not-running" && (
            <div className="docker-card">
              <p>Docker Desktop is installed but not running.</p>
              <p className="screen-footnote">Please open Docker Desktop, then wait — Flux will continue automatically.</p>
            </div>
          )}
          {state === "ready" && (
            <div className="status-row">
              <span className="status-check">✓</span>
              <span className="status-text">Docker is running</span>
            </div>
          )}
        </div>

        {(state === "not-found" || state === "not-running") && (
          <div className="status-row status-polling">
            <span className="status-spinner status-spinner-sm" />
            <span className="status-text-muted">Checking every 3 seconds…</span>
          </div>
        )}
      </div>
    </Layout>
  );
}
