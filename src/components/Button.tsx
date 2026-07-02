import { ReactNode } from "react";
import "./Button.css";

interface Props {
  onClick?: () => void;
  disabled?: boolean;
  loading?: boolean;
  variant?: "primary" | "ghost";
  children: ReactNode;
  fullWidth?: boolean;
}

export default function Button({
  onClick,
  disabled,
  loading,
  variant = "primary",
  children,
  fullWidth,
}: Props) {
  return (
    <button
      className={`btn btn-${variant} ${fullWidth ? "btn-full" : ""}`}
      onClick={onClick}
      disabled={disabled || loading}
    >
      {loading && <span className="btn-spinner" />}
      {children}
    </button>
  );
}
