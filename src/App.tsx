import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import SetupFlow from "./screens/SetupFlow";
import { SetupStep } from "./screens/SetupFlow";
import HomeScreen from "./screens/HomeScreen";
import StartupScreen from "./screens/StartupScreen";
import RunningScreen from "./screens/RunningScreen";
import { PullProgressBar, usePullProgress } from "./components/PullProgress";
import { reasonsText, takeUpdateFarmOnLaunch, type SafetyReason, type UpdateSafety } from "./farmUpdate";
import "./App.css";

/** How often a downloaded-but-deferred farm update re-asks the farm whether it is idle. */
const SAFETY_POLL_MS = 30_000;

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

  // A farm update that was downloaded while the farm was busy: installed once it is idle.
  const [updatePending, setUpdatePending] = useState(false);
  const [waitingFor, setWaitingFor] = useState<SafetyReason[]>([]);
  const [installingUpdate, setInstallingUpdate] = useState(false);
  const [updateError, setUpdateError] = useState("");
  const [noticeHidden, setNoticeHidden] = useState(false);
  const [dashboardNonce, setDashboardNonce] = useState(0);
  const pullProgress = usePullProgress();

  function deferUpdate(safety: UpdateSafety) {
    setWaitingFor(safety.reasons);
    setUpdateError("");
    setNoticeHidden(false);
    setUpdatePending(true);
  }

  // The dashboard's "Update farm" (host bridge) downloaded an update while the farm was busy.
  useEffect(() => {
    const unlisten = listen<UpdateSafety>("farm-update-deferred", (e) => deferUpdate(e.payload));
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    if (!updatePending) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      // Farm stopped meanwhile: nothing to install into (the next start uses the new images).
      const up = await invoke<boolean>("check_dashboard_health").catch(() => false);
      if (cancelled) return;
      if (!up) { setUpdatePending(false); return; }
      const safety = await invoke<UpdateSafety>("check_update_safety")
        .catch(() => ({ safe: false, known: false, reasons: [] } as UpdateSafety));
      if (cancelled) return;
      if (!safety.safe) {
        setWaitingFor(safety.reasons);
        timer = setTimeout(tick, SAFETY_POLL_MS);
        return;
      }
      setInstallingUpdate(true);
      setNoticeHidden(false);
      try {
        await invoke("apply_farm_update");
        // Wait for the recreated dashboard, then reload the frame onto the new version.
        for (let i = 0; i < 60; i++) {
          if (await invoke<boolean>("check_farm_running").catch(() => false)) break;
          await new Promise((r) => setTimeout(r, 2000));
        }
        setDashboardNonce((n) => n + 1);
        setUpdatePending(false);
      } catch (e) {
        setUpdateError(String(e));
        setUpdatePending(false);
      } finally {
        setInstallingUpdate(false);
      }
    };
    tick();
    return () => { cancelled = true; if (timer) clearTimeout(timer); };
  }, [updatePending]);

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
      const afterFluxUpdate = takeUpdateFarmOnLaunch();
      if (await isFarmRunning()) {
        // A running farm means setup was done: without this, stopping the farm sent "Get started"
        // back through license + Docker setup (the key was still saved).
        setIsConfigured(true);
        // Flux just updated itself: update the farm too (StartupScreen keeps a busy farm running
        // and defers the install until it is idle).
        if (afterFluxUpdate) { setIsConfigured(true); setState("startup"); return; }
        // The farm survived a Flux restart/crash; the host bridge (Update farm, terminals) lives
        // in Flux and must be started for this session too (audit 2026-10-15).
        invoke("start_host_bridge").catch(() => {});
        setState("running");
        // An update downloaded earlier but never installed (deferred, then Flux restarted).
        if (await invoke<boolean>("farm_update_pending").catch(() => false)) {
          deferUpdate({ safe: false, known: false, reasons: [] });
        }
        return;
      }
      const hasLicense = await invoke<boolean>("check_license");
      if (!hasLicense) { setState("home"); return; }
      // Only mark as configured if setup was fully completed
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
      {state === "startup" && (
        <StartupScreen
          onReady={(deferred) => { if (deferred) deferUpdate(deferred); setState("running"); }}
          onResetSetup={() => setState("setup")}
        />
      )}
      {state === "running" && <RunningScreen key={dashboardNonce} onBack={() => setState("home")} />}

      {/* Farm update notices — small and non-blocking, over the dashboard */}
      {state !== "startup" && pullProgress.progress && (
        <div className="farm-update-notice">
          <span>Downloading the farm update…</span>
          <PullProgressBar state={pullProgress} />
        </div>
      )}
      {state !== "startup" && !pullProgress.progress && !noticeHidden && (installingUpdate || updatePending || updateError) && (
        <div className="farm-update-notice">
          <span>
            {installingUpdate
              ? "Installing the farm update — the dashboard reloads in a moment…"
              : updateError
                ? `The farm update could not be installed: ${updateError.slice(0, 200)}`
                : `Farm update downloaded — it installs when the farm is idle${waitingFor.length ? ` (waiting for: ${reasonsText(waitingFor)})` : ""}.`}
          </span>
          {!installingUpdate && (
            <button
              className="farm-update-notice-close"
              aria-label="Hide"
              onClick={() => { setNoticeHidden(true); setUpdateError(""); }}
            >
              ×
            </button>
          )}
        </div>
      )}

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
              This will stop the farm and remove your license key.
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
