import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import SetupFlow from "./screens/SetupFlow";
import { SetupStep } from "./screens/SetupFlow";
import HomeScreen from "./screens/HomeScreen";
import StartupScreen from "./screens/StartupScreen";
import RunningScreen from "./screens/RunningScreen";
import "./App.css";

type AppState = "loading" | "setup" | "home" | "startup" | "running";

async function isFarmRunning(): Promise<boolean> {
  try {
    return await invoke<boolean>("check_farm_running");
  } catch {
    return false;
  }
}

export default function App() {
  const [state, setState] = useState<AppState>("loading");
  const [initialStep] = useState<SetupStep>("license");
  const [showResetConfirm, setShowResetConfirm] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [showStopConfirm, setShowStopConfirm] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [showQuitConfirm, setShowQuitConfirm] = useState(false);
  const [quitting, setQuitting] = useState(false);
  const [showMigrateConfirm, setShowMigrateConfirm] = useState(false);
  const [migrating, setMigrating] = useState(false);

  useEffect(() => {
    const unlisten = listen("reset-requested", () => setShowResetConfirm(true));
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    const unlisten = listen("stop-requested", () => setShowStopConfirm(true));
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    const unlisten = listen("quit-requested", () => setShowQuitConfirm(true));
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    const unlisten = listen("migrate-requested", () => setShowMigrateConfirm(true));
    return () => { unlisten.then((f) => f()); };
  }, []);

  async function doQuit() {
    setQuitting(true);
    await invoke("confirm_quit");
  }

  async function doStop() {
    setStopping(true);
    await invoke("stop_farm");
    setShowStopConfirm(false);
    setStopping(false);
    setState("home");
  }

  async function doReset() {
    setResetting(true);
    await invoke("reset_setup");
    window.location.reload();
  }

  async function doMigrate() {
    setMigrating(true);
    await invoke("soft_reset");
    window.location.reload();
  }

  const [isConfigured, setIsConfigured] = useState(false);

  useEffect(() => {
    if (import.meta.env.DEV) { setIsConfigured(true); setState("home"); return; }
    async function init() {
      if (await isFarmRunning()) { setState("running"); return; }
      const hasLicense = await invoke<boolean>("check_license");
      if (!hasLicense) { setState("home"); return; }
      // Only mark as configured if setup was fully completed (including auth)
      const setupDone = await invoke<boolean>("check_setup_complete");
      if (setupDone) setIsConfigured(true);
      setState("home");
    }
    init();
  }, []);

  return (
    <>
      {state === "loading" && (
        <div className="app-loading">
          <span className="app-loading-spinner" />
        </div>
      )}
      {state === "home"    && <HomeScreen isConfigured={isConfigured} onStart={() => setState(isConfigured ? "startup" : "setup")} />}
      {state === "setup"   && <SetupFlow initialStep={initialStep} onComplete={() => { invoke("mark_setup_complete"); setIsConfigured(true); setState("startup"); }} />}
      {state === "startup" && <StartupScreen onReady={() => setState("running")} onResetSetup={() => setState("setup")} />}
      {state === "running" && <RunningScreen onBack={() => setState("home")} />}

      {showQuitConfirm && (
        <div className="reset-overlay">
          <div className="reset-dialog">
            {quitting ? (
              <>
                <h2>Stopping Farm…</h2>
                <p>Waiting for agents to shut down. This may take a moment.</p>
                <div className="reset-actions">
                  <span className="quit-spinner" />
                </div>
              </>
            ) : (
              <>
                <h2>Quit Flux?</h2>
                <p>This will stop the farm and all running agents before closing.</p>
                <div className="reset-actions">
                  <button className="reset-btn-cancel" onClick={() => setShowQuitConfirm(false)}>
                    Cancel
                  </button>
                  <button className="reset-btn-confirm" onClick={doQuit}>
                    Quit
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      {showStopConfirm && (
        <div className="reset-overlay">
          <div className="reset-dialog">
            <h2>Stop Farm?</h2>
            <p>
              This will stop all running agents and the dashboard.
              You can restart from the home screen.
            </p>
            <div className="reset-actions">
              <button
                className="reset-btn-cancel"
                onClick={() => setShowStopConfirm(false)}
                disabled={stopping}
              >
                Cancel
              </button>
              <button
                className="reset-btn-confirm"
                onClick={doStop}
                disabled={stopping}
              >
                {stopping ? "Stopping…" : "Stop Farm"}
              </button>
            </div>
          </div>
        </div>
      )}

      {showMigrateConfirm && (
        <div className="reset-overlay">
          <div className="reset-dialog">
            <h2>Migrate from Terminal Setup?</h2>
            <p>
              This will stop any running farm containers and remove the old farm directory.
              Your task history and AI credentials will be preserved.
              Flux will guide you through setup again.
            </p>
            <div className="reset-actions">
              <button
                className="reset-btn-cancel"
                onClick={() => setShowMigrateConfirm(false)}
                disabled={migrating}
              >
                Cancel
              </button>
              <button
                className="reset-btn-confirm"
                onClick={doMigrate}
                disabled={migrating}
              >
                {migrating ? "Migrating…" : "Migrate"}
              </button>
            </div>
          </div>
        </div>
      )}

      {showResetConfirm && (
        <div className="reset-overlay">
          <div className="reset-dialog">
            <h2>Reset Setup?</h2>
            <p>
              This will stop the farm, remove your license key, and sign out of Claude.
              You'll need to go through setup again.
            </p>
            <div className="reset-actions">
              <button
                className="reset-btn-cancel"
                onClick={() => setShowResetConfirm(false)}
                disabled={resetting}
              >
                Cancel
              </button>
              <button
                className="reset-btn-confirm"
                onClick={doReset}
                disabled={resetting}
              >
                {resetting ? "Resetting…" : "Reset Everything"}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
