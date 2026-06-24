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
  state: "pending" | "active" | "done" | "error";
}

const INITIAL_STEPS: Step[] = [
  { id: "auth",    label: "Verifying Claude account",   state: "pending" },
  { id: "extract",  label: "Setting up farm directory",  state: "pending" },
  { id: "pull",     label: "Pulling latest images",      state: "pending" },
  { id: "start",    label: "Starting services",          state: "pending" },
  { id: "bridge",   label: "Starting host bridge",       state: "pending" },
  { id: "ready",    label: "Ready",                      state: "pending" },
];

const IMAGES = [
  "ghcr.io/kennydead/claude-agent-farm/agent:latest",
  "ghcr.io/kennydead/claude-agent-farm/dashboard-backend:latest",
  "ghcr.io/kennydead/claude-agent-farm/dashboard-frontend:latest",
];

const AGENT_TAGS = ["claudeagentfarm-coder", "claudeagentfarm-reviewer", "claudeagentfarm-planner", "claudeagentfarm-auditor"];

export default function StartupScreen({ onReady, onResetSetup }: Props) {
  const [steps, setSteps] = useState<Step[]>(INITIAL_STEPS);
  const [error, setError] = useState("");

  const set = (id: string, state: Step["state"]) =>
    setSteps((prev) => prev.map((s) => (s.id === id ? { ...s, state } : s)));

  useEffect(() => { run(); }, []);

  async function run() {
    if (import.meta.env.DEV) {
      for (const step of INITIAL_STEPS) {
        set(step.id, "active");
        await delay(600);
        set(step.id, "done");
      }
      setTimeout(onReady, 600);
      return;
    }

    try {
      // Verify Claude is authenticated before starting anything
      set("auth", "active");
      const isAuth = await invoke<boolean>("check_claude_auth");
      if (!isAuth) {
        set("auth", "error");
        setError("Claude account not authenticated. Please sign in again.");
        return;
      }
      set("auth", "done");

      // Extract bundled files → ~/farm/
      set("extract", "active");
      const farmDir = await invoke<string>("extract_resources");
      set("extract", "done");

      // Pull images + tag
      set("pull", "active");
      for (const image of IMAGES) {
        await invoke("run_command", { program: "docker", args: ["pull", image] });
      }
      for (const tag of AGENT_TAGS) {
        await invoke("run_command", {
          program: "docker",
          args: ["tag", IMAGES[0], tag],
        });
      }
      set("pull", "done");

      // Start services
      set("start", "active");
      await invoke("run_docker_compose", {
        subArgs: ["up", "-d", "dashboard-db", "dashboard-backend", "dashboard-frontend"],
      });
      set("start", "done");

      // Start host bridge (fire and forget)
      set("bridge", "active");
      invoke("run_detached", {
        program: "python3",
        args: [`${farmDir}/host_bridge.py`],
      }).catch(() => {});
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

  async function waitForDashboard(attempts = 40) {
    for (let i = 0; i < attempts; i++) {
      try {
        const res = await fetch("http://localhost:8090/health");
        if (res.ok) return;
      } catch {}
      await delay(1500);
    }
    throw new Error("Dashboard did not start in time. Check that Docker is running and try again.");
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
                <span className="step-label">{step.label}</span>
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
