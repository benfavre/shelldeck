import { Interactive } from "remotion";
import { palette } from "../theme";

export const Pill: React.FC<{
  children: React.ReactNode;
  accent?: "blue" | "teal" | "amber";
}> = ({ children, accent = "blue" }) => {
  const color =
    accent === "teal"
      ? palette.teal
      : accent === "amber"
        ? palette.amber
        : palette.blue;

  return (
    <Interactive.Div
      name={`Feature ${String(children)}`}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        padding: "14px 22px",
        borderRadius: 999,
        backgroundColor: "rgba(255,255,255,0.86)",
        border: `1px solid ${palette.border}`,
        color: palette.ink,
        fontSize: 24,
        lineHeight: 1,
        fontWeight: 650,
        boxShadow: "0 8px 24px rgba(31, 60, 86, 0.07)",
      }}
    >
      <span
        style={{
          width: 10,
          height: 10,
          borderRadius: 99,
          backgroundColor: color,
          boxShadow: `0 0 0 6px ${color}1f`,
        }}
      />
      {children}
    </Interactive.Div>
  );
};
