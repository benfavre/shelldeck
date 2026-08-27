import { Interactive } from "remotion";

export const RoleBadge: React.FC<{
  role: "Utilisateur" | "Support";
  compact?: boolean;
}> = ({ role, compact = false }) => {
  const user = role === "Utilisateur";
  const accent = user ? "#168ee0" : "#6d5ce7";

  return (
    <Interactive.Div
      name={`${role} role badge`}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: compact ? 10 : 14,
        padding: compact ? "10px 15px" : "13px 20px",
        borderRadius: 999,
        color: accent,
        background: `linear-gradient(135deg, ${accent}17, ${accent}0b)`,
        border: `1px solid ${accent}2b`,
        fontSize: compact ? 18 : 22,
        lineHeight: 1,
        fontWeight: 700,
        letterSpacing: 0.3,
        whiteSpace: "nowrap",
      }}
    >
      <span
        style={{
          display: "grid",
          placeItems: "center",
          width: compact ? 26 : 32,
          height: compact ? 26 : 32,
          borderRadius: 99,
          color: "white",
          background: accent,
          fontSize: compact ? 13 : 15,
          fontWeight: 800,
          boxShadow: `0 8px 20px ${accent}35`,
        }}
      >
        {user ? "U" : "S"}
      </span>
      Mode {role}
    </Interactive.Div>
  );
};
