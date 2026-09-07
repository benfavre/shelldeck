import { Interactive } from "remotion";
import { Brand } from "../../components/Brand";
import { RoleBadge } from "../RoleBadge";

export const MobileHeader: React.FC<{
  role?: "Utilisateur" | "Support";
}> = ({ role }) => (
  <Interactive.Div
    name="Mobile header"
    style={{
      position: "absolute",
      left: 80,
      right: 80,
      top: 62,
      zIndex: 10,
      display: "flex",
      alignItems: "center",
      justifyContent: role ? "space-between" : "flex-start",
    }}
  >
    <Brand compact />
    {role ? <RoleBadge role={role} compact /> : null}
  </Interactive.Div>
);

export const MobileWindowChrome: React.FC<{ title: string }> = ({ title }) => (
  <Interactive.Div
    name={`${title} window chrome`}
    className="us-focus-chrome"
    style={{
      height: 64,
      display: "flex",
      alignItems: "center",
      gap: 10,
      padding: "0 24px",
    }}
  >
    <span className="us-window-dot" style={{ backgroundColor: "#ff786b" }} />
    <span className="us-window-dot" style={{ backgroundColor: "#ffc64f" }} />
    <span className="us-window-dot" style={{ backgroundColor: "#48c78e" }} />
    <span
      style={{
        marginLeft: 10,
        color: "#65758a",
        fontSize: 18,
        fontWeight: 700,
      }}
    >
      {title}
    </span>
  </Interactive.Div>
);

export const MobilePill: React.FC<{
  children: React.ReactNode;
  color?: string;
}> = ({ children, color = "#168ee0" }) => (
  <span
    style={{
      display: "inline-flex",
      alignItems: "center",
      padding: "9px 13px",
      borderRadius: 999,
      color,
      background: `${color}12`,
      border: `1px solid ${color}20`,
      fontSize: 15,
      fontWeight: 750,
      whiteSpace: "nowrap",
    }}
  >
    {children}
  </span>
);
