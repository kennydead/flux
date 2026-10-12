import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./RunningScreen.css";

interface Props {
  onBack: () => void;
}

export default function RunningScreen({ onBack }: Props) {
  const [loaded, setLoaded] = useState(false);
  const [unreachable, setUnreachable] = useState(false);
  const [frameNonce, setFrameNonce] = useState(0);
  const failCount = useRef(0);
  const frameRef = useRef<HTMLIFrameElement>(null);
  const pendingPath = useRef<string | null>(null);

  // Desktop notifications: the dashboard (in the frame) hands up the farm notifications that need
  // the owner; Flux shows them as system notifications when its window is not in front. Clicking a
  // notification brings Flux forward — on that focus the dashboard opens the page the notification
  // is about.
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      if (e.origin !== "http://localhost:5174") return;
      const d = e.data as { source?: string; type?: string; title?: string; body?: string; path?: string } | null;
      if (!d || d.source !== "agent-farm" || d.type !== "desktop-notification") return;
      invoke<boolean>("desktop_notify", { title: d.title ?? "Agent Farm", body: d.body ?? "" })
        .then((shown) => { if (shown && d.path) pendingPath.current = d.path; })
        .catch(() => {});
    };
    const onFocus = () => {
      const path = pendingPath.current;
      if (!path) return;
      pendingPath.current = null;
      frameRef.current?.contentWindow?.postMessage({ source: "flux", type: "navigate", path }, "http://localhost:5174");
    };
    window.addEventListener("message", onMessage);
    window.addEventListener("focus", onFocus);
    return () => { window.removeEventListener("message", onMessage); window.removeEventListener("focus", onFocus); };
  }, []);

  // Poll farm health so a dead dashboard shows a message instead of a black window
  useEffect(() => {
    const timer = setInterval(async () => {
      const up = await invoke<boolean>("check_farm_running").catch(() => false);
      if (up) {
        failCount.current = 0;
        if (unreachable) {
          // Farm came back — reload the iframe
          setUnreachable(false);
          setLoaded(false);
          setFrameNonce((n) => n + 1);
        }
      } else {
        failCount.current += 1;
        if (failCount.current >= 3) setUnreachable(true);
      }
    }, 5000);
    return () => clearInterval(timer);
  }, [unreachable]);

  return (
    <div className="running-wrap">
      <iframe
        ref={frameRef}
        key={frameNonce}
        className="dashboard-frame"
        src="http://localhost:5174"
        title="Farm Dashboard"
        allow="clipboard-read; clipboard-write"
        onLoad={() => setLoaded(true)}
      />
      {(!loaded || unreachable) && (
        <div className="dashboard-overlay">
          {unreachable ? (
            <div className="dashboard-overlay-message">
              <h2>Dashboard not responding</h2>
              <p>The farm may be restarting or updating. It will reconnect automatically.</p>
              <button className="dashboard-overlay-btn" onClick={onBack}>
                Back to Home
              </button>
            </div>
          ) : (
            <div className="dashboard-overlay-message">
              <span className="dashboard-spinner" />
              <p>Loading dashboard…</p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
