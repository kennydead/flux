import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import "./PullProgress.css";

/** Payload of the Rust "farm-pull-progress" event (src-tauri/src/pull.rs). */
export interface PullProgressEvent {
  phase: "pulling" | "done";
  image: string;
  index: number;
  count: number;
  downloaded: number;
  total: number;
  determinate: boolean;
}

export interface PullProgressState {
  progress: PullProgressEvent | null;
  /** Seconds left, or null while the rate isn't known yet. */
  eta: number | null;
}

/** Live farm image download progress (both the startup pull and the dashboard's "Update farm"). */
export function usePullProgress(): PullProgressState {
  const [state, setState] = useState<PullProgressState>({ progress: null, eta: null });
  const samples = useRef<{ t: number; b: number }[]>([]);

  useEffect(() => {
    const unlisten = listen<PullProgressEvent>("farm-pull-progress", ({ payload }) => {
      if (payload.phase === "done") {
        samples.current = [];
        setState({ progress: null, eta: null });
        return;
      }
      const now = Date.now();
      const s = samples.current;
      if (s.length && payload.downloaded < s[s.length - 1].b) s.length = 0;
      s.push({ t: now, b: payload.downloaded });
      // Rate over the last ~20 s: steady enough for a rough ETA, quick to follow a change.
      while (s.length > 2 && now - s[0].t > 20_000) s.shift();
      let eta: number | null = null;
      const span = (now - s[0].t) / 1000;
      if (payload.determinate && span >= 3) {
        const rate = (payload.downloaded - s[0].b) / span;
        if (rate > 0) eta = Math.max(0, (payload.total - payload.downloaded) / rate);
      }
      setState({ progress: payload, eta });
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  return state;
}

function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(2)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  return `${Math.round(n / 1e3)} KB`;
}

function formatEta(sec: number): string {
  if (sec < 60) return "less than a minute left";
  const min = Math.round(sec / 60);
  return min === 1 ? "about 1 minute left" : `about ${min} minutes left`;
}

/** One overall bar. Renders nothing when progress can't be computed — callers keep their text. */
export function PullProgressBar({ state }: { state: PullProgressState }) {
  const p = state.progress;
  if (!p || !p.determinate || p.total <= 0) return null;
  const pct = Math.min(100, Math.max(0, (p.downloaded / p.total) * 100));
  return (
    <div className="pull-progress">
      <div className="pull-progress-track">
        <div className="pull-progress-bar" style={{ width: `${pct}%` }} />
      </div>
      <div className="pull-progress-meta">
        <span>
          {p.image ? `${p.image} (${p.index + 1}/${p.count})` : "Downloading"}
          {" · "}
          {formatBytes(p.downloaded)} of {formatBytes(p.total)}
        </span>
        {state.eta !== null && p.downloaded < p.total && <span>{formatEta(state.eta)}</span>}
      </div>
    </div>
  );
}
