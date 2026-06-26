import Layout from "../components/Layout";
import Button from "../components/Button";
import "./HomeScreen.css";

interface Props {
  onStart: () => void;
  isConfigured: boolean;
}

export default function HomeScreen({ onStart, isConfigured }: Props) {
  return (
    <Layout>
      <div className="home">
        <div className="home-logo">
          <div className="home-icon">
            <svg width="56" height="56" viewBox="0 0 56 56" fill="none">
              <polygon points="28,6 50,18 50,38 28,50 6,38 6,18" fill="url(#grad)" opacity="0.15" />
              <polygon points="28,10 46,20 46,36 28,46 10,36 10,20" fill="url(#grad)" opacity="0.3" />
              <polygon points="28,16 42,24 42,34 28,42 14,34 14,24" fill="url(#grad)" />
              <defs>
                <linearGradient id="grad" x1="6" y1="6" x2="50" y2="50" gradientUnits="userSpaceOnUse">
                  <stop stopColor="#4F8EF7" />
                  <stop offset="1" stopColor="#7C5CFC" />
                </linearGradient>
              </defs>
            </svg>
          </div>
          <h1 className="home-title">Flux</h1>
          <p className="home-subtitle">AI Agent Farm</p>
        </div>

        {isConfigured && (
          <div className="home-status">
            <div className="home-status-dot" />
            <span>Ready to start</span>
          </div>
        )}

        <Button onClick={onStart} fullWidth>
          {isConfigured ? "Start Farm" : "Get Started"}
        </Button>
      </div>
    </Layout>
  );
}
