import "./RunningScreen.css";

interface Props {
  onBack: () => void;
}

export default function RunningScreen({ onBack: _onBack }: Props) {
  return (
    <iframe
      className="dashboard-frame"
      src="http://localhost:5174"
      title="Farm Dashboard"
      allow="clipboard-read; clipboard-write"
    />
  );
}
