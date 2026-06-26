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
  const [initialStep, setInitialStep] = useState<SetupStep>("license");
  const [showResetConfirm, setShowResetConfirm] = useState(false);
  const [resetting, setResetting] = useState(false);

  useEffect(() => {
    const unlisten = listen("reset-requested", () => setShowResetConfirm(true));
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    const unlisten = listen("farm-stopped", () => setState("home"));
    return () => { unlisten.then((f) => f()); };
  }, []);

  async function doReset() {
    setResetting(true);
    await invoke("reset_setup");
    window.location.reload();
  }

  const [isConfigured, setIsConfigured] = useState(false);

  useEffect(() => {
    if (import.meta.env.DEV) { setIsConfigured(true); setState("home"); return; }
    async function init() {
      if (await isFarmRunning()) { setState("running"); return; }
      const hasLicense = await invoke<boolean>("check_license");
      if (!hasLicense) { setState("home"); return; }
      const isAuth = await invoke<boolean>("check_claude_auth");
      if (!isAuth) { setInitialStep("docker"); setState("home"); return; }
      setIsConfigured(true);
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
      {state === "setup"   && <SetupFlow initialStep={initialStep} onComplete={() => { setIsConfigured(true); setState("home"); }} />}
      {state === "startup" && <StartupScreen onReady={() => setState("running")} onResetSetup={() => setState("setup")} />}
      {state === "running" && <RunningScreen onBack={() => setState("home")} />}

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
