import { CanvasImage, Interactive, staticFile } from "remotion";
import { palette } from "../theme";

export const Brand: React.FC<{ compact?: boolean }> = ({ compact = false }) => {
  const iconSize = compact ? 52 : 76;

  return (
    <Interactive.Div
      name="ShellDeck brand"
      style={{ display: "flex", alignItems: "center", gap: compact ? 16 : 22 }}
    >
      <CanvasImage
        name="ShellDeck logo"
        src={staticFile("assets/shelldeck-light.png")}
        width={128}
        height={128}
        fit="contain"
        style={{ width: iconSize, height: iconSize, borderRadius: compact ? 13 : 18 }}
      />
      <Interactive.Div
        name="ShellDeck wordmark"
        style={{
          color: palette.ink,
          fontSize: compact ? 36 : 50,
          lineHeight: 1,
          fontWeight: 760,
          letterSpacing: -1.5,
        }}
      >
        Shell<span style={{ color: palette.blue }}>Deck</span>
      </Interactive.Div>
    </Interactive.Div>
  );
};
