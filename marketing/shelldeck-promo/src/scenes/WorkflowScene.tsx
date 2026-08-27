import {
  AbsoluteFill,
  Easing,
  Interactive,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";
import { Brand } from "../components/Brand";
import { LightBackdrop } from "../components/LightBackdrop";
import { ScreenshotCard } from "../components/ScreenshotCard";
import { palette } from "../theme";

export const WorkflowScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <LightBackdrop imageOpacity={0.1} />
      <Interactive.Div
        name="Workflow header"
        style={{
          position: "absolute",
          left: 112,
          right: 112,
          top: 72,
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <Brand compact />
        <Interactive.Div
          name="Workflow chapter"
          style={{
            padding: "13px 22px",
            borderRadius: 999,
            color: "#087c67",
            backgroundColor: "rgba(34, 191, 160, 0.11)",
            fontSize: 22,
            fontWeight: 720,
            letterSpacing: 1.2,
            textTransform: "uppercase",
          }}
        >
          02 · Automatisation
        </Interactive.Div>
      </Interactive.Div>

      <Interactive.H2
        name="Workflow headline"
        style={{
          position: "absolute",
          left: 112,
          top: 172,
          width: 1120,
          margin: 0,
          color: palette.ink,
          fontSize: 70,
          lineHeight: 1.04,
          fontWeight: 780,
          letterSpacing: -3,
          opacity: interpolate(frame, [0, 0.55 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [0, 0.55 * fps], ["0px 30px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        Des tunnels aux scripts,
        <br />
        <span style={{ color: palette.teal }}>tout reste à portée de main.</span>
      </Interactive.H2>

      <ScreenshotCard
        src="dev-tunnels.webp"
        name="ShellDeck port forwarding"
        style={{
          width: 1120,
          height: 630,
          left: 112,
          bottom: -48,
          zIndex: 1,
          opacity: interpolate(frame, [0.5 * fps, 1.2 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [0.5 * fps, 1.2 * fps], ["-70px 64px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          rotate: "-1.2deg",
        }}
      />

      <ScreenshotCard
        src="dev-scripts.webp"
        name="ShellDeck script runner"
        style={{
          width: 920,
          height: 518,
          right: 104,
          bottom: 12,
          zIndex: 2,
          opacity: interpolate(frame, [1.05 * fps, 1.75 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [1.05 * fps, 1.75 * fps], ["78px 72px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          rotate: "1.6deg",
        }}
      />

      <Interactive.Div
        name="Workflow metrics"
        style={{
          position: "absolute",
          right: 126,
          top: 208,
          zIndex: 3,
          display: "flex",
          gap: 14,
          opacity: interpolate(frame, [2.15 * fps, 2.7 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [2.15 * fps, 2.7 * fps], ["0px 26px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        {[
          ["2", "tunnels prêts", palette.blue],
          ["3", "scripts disponibles", palette.teal],
        ].map(([value, label, color]) => (
          <div
            key={label}
            style={{
              minWidth: 174,
              padding: "16px 20px",
              borderRadius: 18,
              backgroundColor: "rgba(255,255,255,0.95)",
              border: `1px solid ${palette.border}`,
              boxShadow: "0 14px 36px rgba(31, 60, 86, 0.12)",
            }}
          >
            <div style={{ color, fontSize: 34, lineHeight: 1, fontWeight: 780 }}>{value}</div>
            <div style={{ color: palette.muted, fontSize: 18, marginTop: 8 }}>{label}</div>
          </div>
        ))}
      </Interactive.Div>
    </AbsoluteFill>
  );
};
