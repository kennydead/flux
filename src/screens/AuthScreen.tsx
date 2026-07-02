import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import Layout from "../components/Layout";
import Button from "../components/Button";
import "./Screen.css";

interface Props {
  onAuthenticated: () => void;
}

type AuthState = "checking" | "needed" | "waiting-url" | "waiting-code" | "authenticated" | "error";

export default function AuthScreen({ onAuthenticated }: Props) {
  const [state, setState] = useState<AuthState>("checking");
  const [loginUrl, setLoginUrl] = useState("");
  const [code, setCode] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (import.meta.env.DEV) { setState("needed"); return; }
    invoke<boolean>("check_claude_auth").then((ok) => {
      if (ok) onAuthenticated();
      else setState("needed");
    });
  }, []);

  async function startLogin() {
    setState("waiting-url");
    setError("");
    if (import.meta.env.DEV) {
      setLoginUrl("https://claude.ai/oauth/authorize?dev=true");
      setState("waiting-code");
      return;
    }
    try {
      const url = await invoke<string>("start_claude_auth");
      setLoginUrl(url);
      setState("waiting-code");
    } catch (e: any) {
      setError(String(e));
      setState("error");
    }
  }

  async function submitCode() {
    if (!code.trim()) return;
    setLoading(true);
    setError("");
    if (import.meta.env.DEV) { onAuthenticated(); return; }
    try {
      await invoke("complete_claude_auth", { code: code.trim() });
      onAuthenticated();
    } catch (e: any) {
      setError(String(e));
      setState("error");
      setLoading(false);
    }
  }

  return (
    <Layout>
      <div className="screen">
        <div className="screen-header">
          <h1>Sign in to Claude</h1>
          <p className="screen-subtitle">
            Your agents need a Claude account to operate.
          </p>
        </div>

        <div className="screen-body">
          {state === "checking" && (
            <div className="status-row">
              <span className="status-spinner" />
              <span className="status-text">Checking authentication…</span>
            </div>
          )}

          {state === "needed" && (
            <div className="auth-card">
              <p>You'll need to sign in with your Claude account. A login URL will appear — open it in your browser, then paste the code back here.</p>
            </div>
          )}

          {state === "waiting-url" && (
            <div className="status-row">
              <span className="status-spinner" />
              <span className="status-text">Preparing login…</span>
            </div>
          )}

          {state === "waiting-code" && (
            <div className="auth-flow">
              <div className="auth-url-box">
                <p className="auth-url-label">Open this URL in your browser</p>
                <button className="auth-url" onClick={() => openUrl(loginUrl)}>
                  {loginUrl}
                </button>
                <Button variant="ghost" onClick={() => openUrl(loginUrl)}>
                  Open in Browser
                </Button>
              </div>
              <div className="field">
                <label className="field-label">Paste the code from your browser</label>
                <input
                  className="field-input"
                  type="text"
                  placeholder="Paste code here…"
                  value={code}
                  onChange={(e) => setCode(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && submitCode()}
                  autoFocus
                />
              </div>
            </div>
          )}

          {state === "error" && (
            <p className="field-error">{error}</p>
          )}
        </div>

        {state === "needed" && (
          <Button onClick={startLogin} fullWidth>Get Login URL</Button>
        )}
        {state === "waiting-code" && (
          <Button onClick={submitCode} loading={loading} disabled={!code.trim()} fullWidth>
            Confirm Sign In
          </Button>
        )}
        {state === "error" && (
          <Button onClick={() => setState("needed")} fullWidth>Try Again</Button>
        )}
      </div>
    </Layout>
  );
}
