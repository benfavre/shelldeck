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

export const TerminalScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <LightBackdrop imageOpacity={0.09} />
      <Interactive.Div
        name="Terminal scene header"
        style={{
          position: "absolute",
          left: 112,
          right: 112,
          top: 72,
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          opacity: interpolate(frame, [0, 0.55 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        <Brand compact />
        <Interactive.Div
          name="Terminal chapter"
          style={{
            padding: "13px 22px",
            borderRadius: 999,
            color: palette.blueDeep,
            backgroundColor: "rgba(24, 139, 214, 0.1)",
            fontSize: 22,
            fontWeight: 720,
            letterSpacing: 1.2,
            textTransform: "uppercase",
          }}
        >
          01 · Terminal
        </Interactive.Div>
      </Interactive.Div>

      <Interactive.H2
        name="Terminal headline"
        style={{
          position: "absolute",
          left: 112,
          top: 168,
          margin: 0,
          color: palette.ink,
          fontSize: 74,
          lineHeight: 1.04,
          fontWeight: 780,
          letterSpacing: -3,
          opacity: interpolate(frame, [0.25 * fps, 0.8 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [0.25 * fps, 0.8 * fps], ["0px 28px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        Connectez-vous.
        <br />
        <span style={{ color: palette.blue }}>Gardez le contexte.</span>
      </Interactive.H2>

      <ScreenshotCard
        src="dev-terminal.webp"
        name="ShellDeck terminal"
        style={{
          left: 112,
          right: 112,
          bottom: -38,
          height: 662,
          opacity: interpolate(frame, [0.6 * fps, 1.3 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          scale: interpolate(frame, [0.6 * fps, 4.8 * fps], [0.94, 1.025], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
            output: "perceptual-scale",
          }),
          translate: interpolate(frame, [0.6 * fps, 4.8 * fps], ["0px 68px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      />

      <Interactive.Div
        name="Live status"
        style={{
          position: "absolute",
          right: 150,
          bottom: 86,
          display: "flex",
          alignItems: "center",
          gap: 14,
          padding: "18px 24px",
          borderRadius: 18,
          color: palette.ink,
          backgroundColor: "rgba(255,255,255,0.94)",
          border: `1px solid ${palette.border}`,
          boxShadow: "0 18px 48px rgba(24, 52, 76, 0.2)",
          fontSize: 23,
          fontWeight: 680,
          opacity: interpolate(frame, [2 * fps, 2.5 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [2 * fps, 2.5 * fps], ["0px 24px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        <span
          style={{
            width: 12,
            height: 12,
            borderRadius: 99,
            backgroundColor: palette.green,
            boxShadow: `0 0 0 7px ${palette.green}20`,
          }}
        />
        Session active · prod-eu-west
      </Interactive.Div>
    </AbsoluteFill>
  );
};
