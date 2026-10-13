import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
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
  const pendingPath = useRef<{ path: string; at: number } | null>(null);

  // Desktop notifications: the dashboard (in the frame) hands up the farm notifications that need
  // the owner; Flux shows them as system notifications when its window is not in front. Clicking a
  // notification brings Flux forward — on that focus the dashboard opens the page the notification
  // is about.
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      if (e.origin !== "http://localhost:5174" || e.source !== frameRef.current?.contentWindow) return;
      const d = e.data as { source?: string; type?: string; title?: string; body?: string; path?: string; url?: string } | null;
      if (!d || d.source !== "agent-farm") return;
      if (d.type === "open-url") {
        // The webview ignores the frame's target="_blank" links / window.open: the dashboard hands
        // external links here and they open in the default browser (http/https only).
        if (typeof d.url === "string" && /^https?:\/\//i.test(d.url)) openUrl(d.url).catch(() => {});
        return;
      }
      if (d.type !== "desktop-notification") return;
      invoke<boolean>("desktop_notify", { title: d.title ?? "Agent Farm", body: d.body ?? "" })
        .then((shown) => { if (shown && d.path) pendingPath.current = { path: d.path, at: Date.now() }; })
        .catch(() => {});
    };
    // Desktop notifications can't report a click, so "focus soon after a notification" stands in
    // for it: only within a minute, and only once — a later alt-tab never moves the dashboard.
    const onFocus = () => {
      const p = pendingPath.current;
      pendingPath.current = null;
      if (!p || Date.now() - p.at > 60_000) return;
      frameRef.current?.contentWindow?.postMessage({ source: "flux", type: "navigate", path: p.path }, "http://localhost:5174");
    };
    window.addEventListener("message", onMessage);
    // The OS window's focus (the page's own focus event may not fire when focus goes to the frame).
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    getCurrentWindow().onFocusChanged(({ payload: focused }) => { if (focused) onFocus(); })
      .then((u) => { if (cancelled) u(); else unlisten = u; })
      .catch(() => { if (!cancelled) window.addEventListener("focus", onFocus); });
    return () => {
      cancelled = true;
      window.removeEventListener("message", onMessage);
      window.removeEventListener("focus", onFocus);
      unlisten?.();
    };
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
