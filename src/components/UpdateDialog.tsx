import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { reasonsText, type UpdateSafety } from "../farmUpdate";

/** updates.rs Status: one "Flux update" over the app and the farm images. */
export interface FluxUpdateStatus {
  flux_version: string;
  app_update: string | null;
  images_update: boolean | null;
  update_available: boolean;
}

type Phase = "checking" | "ready" | "working" | "done" | "error";

/**
 * Tray → "Check for Updates…": the same one update the dashboard offers. A new Flux app installs
 * and Flux restarts (the farm keeps running; the restarted Flux brings the new images). Images only
 * → the farm update. Busy farm → "Update when idle" by default, "Update now anyway" deliberately.
 */
export default function UpdateDialog({ onClose, onFarmUpdated }: { onClose: () => void; onFarmUpdated: () => void }) {
  const [phase, setPhase] = useState<Phase>("checking");
  const [status, setStatus] = useState<FluxUpdateStatus | null>(null);
  const [safety, setSafety] = useState<UpdateSafety | null>(null);
  const [farmUp, setFarmUp] = useState(false);
  const [message, setMessage] = useState("");

  useEffect(() => {
    (async () => {
      try {
        const [st, up] = await Promise.all([
          invoke<FluxUpdateStatus>("check_updates", { refresh: true }),
          invoke<boolean>("check_dashboard_health").catch(() => false),
        ]);
        setStatus(st);
        setFarmUp(up);
        if (up && st.update_available) {
          setSafety(await invoke<UpdateSafety>("check_update_safety").catch(() => null));
        }
        setPhase("ready");
      } catch (e) {
        setMessage(String(e));
        setPhase("error");
      }
    })();
  }, []);

  async function install(now: boolean) {
    if (!status) return;
    setPhase("working");
    try {
      if (status.app_update) {
        // Flux restarts on its own in a moment; the notice in App.tsx takes over.
        await invoke<boolean>("install_app_update", { now });
        setMessage(`Installing Flux ${status.app_update} — Flux restarts in a moment. The farm keeps running.`);
        setPhase("done");
        return;
      }
      const busy = await invoke<string | null>("update_farm_now", { now });
      if (busy) {
        setMessage(busy);
      } else {
        setMessage("Updated. The dashboard reloads.");
        onFarmUpdated();
      }
      setPhase("done");
    } catch (e) {
      setMessage(String(e).slice(0, 300));
      setPhase("error");
    }
  }

  const unsafe = !!safety && !safety.safe;
  const what = status?.app_update
    ? `Flux ${status.app_update} is available (you have ${status.flux_version}).`
    : "A new version of the farm is available.";

  return (
    <div className="reset-overlay">
      <div className="reset-dialog">
        <h2>Flux updates</h2>
        {phase === "checking" && <p>Checking for updates…</p>}
        {phase === "error" && <p>Could not check or install the update: {message}</p>}
        {(phase === "done" || phase === "working") && <p>{phase === "working" ? "Updating…" : message}</p>}
        {phase === "ready" && status && !status.update_available && (
          <p>Flux is up to date (version {status.flux_version}).{status.images_update === null ? " The farm's version couldn't be checked right now." : ""}</p>
        )}
        {phase === "ready" && status?.update_available && !farmUp && !status.app_update && (
          <p>A new version of the farm is available. It installs when you start the farm.</p>
        )}
        {phase === "ready" && status?.update_available && (farmUp || status.app_update) && (
          <>
            <p>{what}</p>
            {unsafe && (
              <p>
                The farm is busy right now ({reasonsText(safety!.reasons)}). Updating when it's idle interrupts nothing;
                agents keep working either way.
              </p>
            )}
          </>
        )}
        <div className="reset-actions">
          {phase === "ready" && status?.update_available && (farmUp || status.app_update) ? (
            unsafe ? (
              <>
                <button className="reset-btn-cancel" onClick={onClose}>Later</button>
                <button className="reset-btn-cancel" onClick={() => install(true)}>Update now anyway</button>
                <button className="reset-btn-confirm" onClick={() => install(false)}>Update when idle</button>
              </>
            ) : (
              <>
                <button className="reset-btn-cancel" onClick={onClose}>Later</button>
                <button className="reset-btn-confirm" onClick={() => install(false)}>Update now</button>
              </>
            )
          ) : (
            <button className="reset-btn-confirm" disabled={phase === "checking" || phase === "working"} onClick={onClose}>
              Close
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
