import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Layout from "../components/Layout";
import Button from "../components/Button";
import "./Screen.css";

interface Props {
  onReady: () => void;
  onResetSetup: () => void;
}

interface Step {
  id: string;
  label: string;
  note?: string;
  state: "pending" | "active" | "done" | "error";
}

const INITIAL_STEPS: Step[] = [
  { id: "docker",  label: "Checking Docker",            state: "pending" },
  { id: "extract", label: "Setting up farm directory",  state: "pending" },
  { id: "pull",    label: "Pulling latest images",      state: "pending" },
  { id: "start",   label: "Starting services",          state: "pending" },
  { id: "bridge",  label: "Starting host bridge",       state: "pending" },
  { id: "ready",   label: "Ready",                      state: "pending" },
];


const AGENT_TAGS = ["claudeagentfarm-coder", "claudeagentfarm-reviewer", "claudeagentfarm-planner", "claudeagentfarm-auditor"];

export default function StartupScreen({ onReady, onResetSetup }: Props) {
  const [steps, setSteps] = useState<Step[]>(INITIAL_STEPS);
  const [error, setError] = useState("");

  const set = (id: string, state: Step["state"], note?: string) =>
    setSteps((prev) => prev.map((s) => (s.id === id ? { ...s, state, ...(note !== undefined ? { note } : {}) } : s)));

  useEffect(() => { run(); }, []);

  async function run() {
    if (import.meta.env.DEV) {
      for (const step of INITIAL_STEPS) {
        set(step.id, "active");
        await delay(500);
        set(step.id, "done");
      }
      setTimeout(onReady, 500);
      return;
    }

    try {
      // Resolve pinned image tags from Rust constant
      const version = await invoke<string>("get_farm_version");
      const reg = "ghcr.io/kennydead/claude-agent-farm";
      const images = [
        `${reg}/agent:${version}`,
        `${reg}/dashboard-backend:${version}`,
        `${reg}/dashboard-frontend:${version}`,
      ];

      // 1. Verify Docker is running
      set("docker", "active");
      const dockerRunning = await invoke<boolean>("check_docker_running");
      if (!dockerRunning) {
        set("docker", "error");
        setError("Docker is not running. Please start Docker Desktop and try again.");
        return;
      }
      set("docker", "done");

      // 2. Extract bundled files → ~/farm/
      set("extract", "active");
      await invoke<string>("extract_resources");
      set("extract", "done");

      // 3. Pull images + tag
      const imageNames = ["agent", "dashboard-backend", "dashboard-frontend"];
      let anyUpdated = false;
      for (let i = 0; i < images.length; i++) {
        set("pull", "active", `${imageNames[i]} (${i + 1}/${images.length}) — can take a few minutes after an update`);
        const updated = await invoke<boolean>("pull_image", { image: images[i] });
        if (updated) anyUpdated = true;
      }
      for (const tag of AGENT_TAGS) {
        await invoke("run_command", { program: "docker", args: ["tag", images[0], tag] });
      }
      set("pull", "done", anyUpdated ? "updated to latest version" : "");

      // Start services
      set("start", "active");
      await invoke("run_docker_compose", {
        subArgs: ["up", "-d", "dashboard-db", "dashboard-backend", "dashboard-frontend"],
      });
      set("start", "done");

      // Start host bridge (fire and forget)
      set("bridge", "active");
      invoke("start_host_bridge").catch(() => {});
      set("bridge", "done");

      // Wait for dashboard
      set("ready", "active");
      await waitForDashboard();
      set("ready", "done");

      setTimeout(onReady, 600);
    } catch (e: any) {
      const msg = String(e);
      setError(msg);
      setSteps((prev) =>
        prev.map((s) => (s.state === "active" ? { ...s, state: "error" } : s))
      );
    }
  }

  async function waitForDashboard(attempts = 80) {
    // Step 1: wait for backend (DB init can be slow on first run)
    let backendUp = false;
    for (let i = 0; i < attempts; i++) {
      backendUp = await invoke<boolean>("check_dashboard_health").catch(() => false);
      if (backendUp) break;
      await delay(2000);
    }
    if (!backendUp) throw new Error("Dashboard did not start in time. Check that Docker is running and try again.");

    // Step 2: wait for frontend (separate container, may lag behind backend)
    for (let i = 0; i < 20; i++) {
      const frontendUp = await invoke<boolean>("check_farm_running").catch(() => false);
      if (frontendUp) return;
      await delay(1500);
    }
    throw new Error("Dashboard frontend did not start in time. Check that Docker is running and try again.");
  }

  const hasError = steps.some((s) => s.state === "error");

  return (
    <Layout>
      <div className="screen">
        <div className="screen-header">
          <h1>Starting Flux</h1>
          <p className="screen-subtitle">Setting up your agent farm…</p>
        </div>

        <div className="screen-body">
          <div className="stepper">
            {steps.map((step) => (
              <div key={step.id} className={`step step-${step.state}`}>
                <div className="step-icon">
                  {step.state === "done"    && <span className="step-check">✓</span>}
                  {step.state === "active"  && <span className="step-pulse" />}
                  {step.state === "error"   && <span className="step-error-icon">✕</span>}
                  {step.state === "pending" && <span className="step-dot" />}
                </div>
                <span className="step-label">
                  {step.label}
                  {step.note && <span className="step-note"> — {step.note}</span>}
                </span>
              </div>
            ))}
          </div>

          {hasError && (
            <div className="startup-error">
              <p className="field-error">{error}</p>
            </div>
          )}
        </div>

        {hasError && (
          <div className="startup-actions">
            <Button onClick={run} fullWidth>Try Again</Button>
            <Button variant="ghost" onClick={onResetSetup} fullWidth>Back to Setup</Button>
          </div>
        )}
      </div>
    </Layout>
  );
}

function delay(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}
