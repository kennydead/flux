import { useState } from "react";
import Layout from "../components/Layout";
import Button from "../components/Button";
import "./Screen.css";

interface Props {
  onSelected: () => void;
}

const PROVIDERS = [
  { id: "claude", name: "Claude", vendor: "Anthropic", available: true },
  { id: "openai", name: "OpenAI / Codex", vendor: "OpenAI", available: false },
  { id: "gemini", name: "Gemini", vendor: "Google", available: false },
];

export default function ProviderScreen({ onSelected }: Props) {
  const [selected, setSelected] = useState("claude");

  return (
    <Layout>
      <div className="screen">
        <div className="screen-header">
          <h1>AI Provider</h1>
          <p className="screen-subtitle">Choose which AI model powers your agents.</p>
        </div>

        <div className="screen-body">
          <div className="provider-list">
            {PROVIDERS.map((p) => (
              <button
                key={p.id}
                className={`provider-card ${selected === p.id ? "provider-card-active" : ""} ${!p.available ? "provider-card-disabled" : ""}`}
                onClick={() => p.available && setSelected(p.id)}
                disabled={!p.available}
              >
                <div className="provider-info">
                  <span className="provider-name">{p.name}</span>
                  <span className="provider-vendor">{p.vendor}</span>
                </div>
                {!p.available && <span className="provider-badge">Coming soon</span>}
                {p.available && selected === p.id && <span className="provider-check">✓</span>}
              </button>
            ))}
          </div>
        </div>

        <Button onClick={onSelected} fullWidth>
          Continue with {PROVIDERS.find((p) => p.id === selected)?.name}
        </Button>
      </div>
    </Layout>
  );
}
