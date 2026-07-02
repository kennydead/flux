import { useState } from "react";
import LicenseScreen from "./LicenseScreen";
import DockerScreen from "./DockerScreen";
import ProviderScreen from "./ProviderScreen";
import AuthScreen from "./AuthScreen";

export type SetupStep = "license" | "docker" | "provider" | "auth";

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
      return <DockerScreen onReady={() => setStep("provider")} />;
    case "provider":
      return <ProviderScreen onSelected={() => setStep("auth")} />;
    case "auth":
      return <AuthScreen onAuthenticated={onComplete} />;
  }
}
