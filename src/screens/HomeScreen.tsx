import Button from "../components/Button";
import appIcon from "../assets/app-icon.png";
import "./HomeScreen.css";

interface Props {
  onStart: () => void;
  isConfigured: boolean;
}

const FEATURES = [
  { icon: "⚡", label: "Multiple AI agents working in parallel" },
  { icon: "🐳", label: "Runs locally in Docker — your code stays private" },
  { icon: "🔧", label: "Built for real software projects" },
];

export default function HomeScreen({ onStart }: Props) {
  return (
    <div className="home">
      {/* Hero */}
      <div className="home-hero">
        <div className="home-glow" />
        <div className="home-icon">
          <img src={appIcon} alt="Flux" className="home-icon-img" />
        </div>

        <h1 className="home-title">Flux</h1>
        <p className="home-tagline">AI Agent Farm</p>
        <p className="home-desc">
          Spin up a team of AI agents that plan, code, and review your software —
          all running locally on your machine.
        </p>
      </div>

      {/* Features */}
      <ul className="home-features">
        {FEATURES.map((f) => (
          <li key={f.label} className="home-feature">
            <span className="home-feature-icon">{f.icon}</span>
            <span>{f.label}</span>
          </li>
        ))}
      </ul>

      {/* CTA */}
      <div className="home-cta">
        <Button onClick={onStart} fullWidth>
          Get Started
        </Button>
      </div>

      {/* Footer */}
      <footer className="home-footer">
        <span className="home-version">v0.1.0</span>
        <div className="home-links">
          <a className="home-link" href="#" onClick={(e) => e.preventDefault()}>Help</a>
          <span className="home-dot">·</span>
          <a className="home-link" href="#" onClick={(e) => e.preventDefault()}>Privacy</a>
          <span className="home-dot">·</span>
          <a className="home-link" href="#" onClick={(e) => e.preventDefault()}>Terms</a>
          <span className="home-dot">·</span>
          <a className="home-link" href="#" onClick={(e) => e.preventDefault()}>Website</a>
        </div>
      </footer>
    </div>
  );
}
