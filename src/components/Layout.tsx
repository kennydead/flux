import { ReactNode } from "react";
import "./Layout.css";

interface Props {
  children: ReactNode;
  narrow?: boolean;
}

export default function Layout({ children, narrow = true }: Props) {
  return (
    <div className="layout">
      <div className={`layout-inner ${narrow ? "layout-narrow" : ""}`}>
        <div className="layout-logo">
          <span className="logo-text">Flux</span>
        </div>
        {children}
      </div>
    </div>
  );
}
