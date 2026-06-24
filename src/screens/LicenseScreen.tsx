import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Layout from "../components/Layout";
import Button from "../components/Button";
import "./Screen.css";

interface Props {
  onVerified: () => void;
}

export default function LicenseScreen({ onVerified }: Props) {
  const [key, setKey] = useState("");
  const [loading, setLoading] = useState(false);

  async function verify() {
    if (!key.trim()) return;
    setLoading(true);
    await invoke("save_license_key", { key: key.trim() });
    onVerified();
  }

  return (
    <Layout>
      <div className="screen">
        <div className="screen-header">
          <h1>Welcome to Flux</h1>
          <p className="screen-subtitle">Enter your license key to get started.</p>
        </div>

        <div className="screen-body">
          <div className="field">
            <label className="field-label">License Key</label>
            <input
              className="field-input"
              type="text"
              placeholder="FLUX-XXXX-XXXX-XXXX"
              value={key}
              onChange={(e) => setKey(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && verify()}
              autoFocus
              spellCheck={false}
            />
          </div>
        </div>

        <Button onClick={verify} loading={loading} disabled={!key.trim()} fullWidth>
          Verify License
        </Button>

        <p className="screen-footnote">
          Don't have a key? Contact us at <span className="link">support@claudeagentfarm.com</span>
        </p>
      </div>
    </Layout>
  );
}
