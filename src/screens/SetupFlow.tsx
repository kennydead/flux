import { useState } from "react";
import LicenseScreen from "./LicenseScreen";
import DockerScreen from "./DockerScreen";

export type SetupStep = "license" | "docker";

interface Props {
  onComplete: () => void;
  initialStep?: SetupStep;
}

export default function SetupFlow({ onComplete, initialStep = "license" }: Props) {
  const [step, setStep] = useState<SetupStep>(initialStep);

  switch (step) {
    case "license":
      return <LicenseScreen onVerified={() => setStep("docker")} />;
    case "docker":
      return <DockerScreen onReady={onComplete} />;
  }
}
