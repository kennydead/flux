import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import SetupFlow from "./screens/SetupFlow";
import { SetupStep } from "./screens/SetupFlow";
import StartupScreen from "./screens/StartupScreen";
import RunningScreen from "./screens/RunningScreen";
import "./App.css";

type AppState = "loading" | "setup" | "startup" | "running";

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

  useEffect(() => {
    const unlisten = listen("reset-requested", async () => {
      const confirmed = window.confirm(
        "This will stop the farm, remove your license key, and sign out of Claude.\n\nContinue?"
      );
      if (!confirmed) return;
      await invoke("reset_setup");
      window.location.reload();
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    if (import.meta.env.DEV) { setState("setup"); return; }
    async function init() {
      if (await isFarmRunning()) { setState("running"); return; }
      const hasLicense = await invoke<boolean>("check_license");
      if (!hasLicense) { setState("setup"); return; }
      // License exists — check if Claude is also authenticated
      const isAuth = await invoke<boolean>("check_claude_auth");
      if (isAuth) { setState("startup"); return; }
      // License saved but not authenticated — skip license step only
      setInitialStep("docker");
      setState("setup");
    }
    init();
  }, []);

  if (state === "loading") return (
    <div className="app-loading">
      <span className="app-loading-spinner" />
    </div>
  );

  if (state === "running") return <RunningScreen onBack={() => setState("startup")} />;
  if (state === "startup") return <StartupScreen onReady={() => setState("running")} onResetSetup={() => setState("setup")} />;
  return <SetupFlow initialStep={initialStep} onComplete={() => setState("startup")} />;
}
