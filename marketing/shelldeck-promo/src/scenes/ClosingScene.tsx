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
import { palette } from "../theme";

export const ClosingScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  return (
    <AbsoluteFill
      style={{
        overflow: "hidden",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <LightBackdrop imageOpacity={0.2} />
      <Interactive.Div
        name="Closing card"
        style={{
          position: "relative",
          width: 1320,
          minHeight: 680,
          padding: "86px 110px",
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          textAlign: "center",
          borderRadius: 42,
          backgroundColor: "rgba(255,255,255,0.82)",
          border: `1px solid ${palette.border}`,
          boxShadow: "0 36px 110px rgba(31, 60, 86, 0.14)",
          backdropFilter: "blur(18px)",
          opacity: interpolate(frame, [0, 0.65 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          scale: interpolate(frame, [0, 0.85 * fps], [0.93, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.spring({ damping: 180 }),
            output: "perceptual-scale",
          }),
        }}
      >
        <Brand />
        <Interactive.H2
          name="Closing headline"
          style={{
            margin: "62px 0 22px",
            color: palette.ink,
            fontSize: 88,
            lineHeight: 1.02,
            fontWeight: 790,
            letterSpacing: -4,
          }}
        >
          Votre infrastructure,
          <br />
          <span style={{ color: palette.blue }}>au même endroit.</span>
        </Interactive.H2>
        <Interactive.P
          name="Closing subtitle"
          style={{
            margin: 0,
            color: palette.muted,
            fontSize: 32,
            lineHeight: 1.35,
          }}
        >
          Travaillez plus vite. Gardez le contrôle.
        </Interactive.P>
        <Interactive.Div
          name="Website call to action"
          style={{
            marginTop: 50,
            padding: "19px 34px",
            borderRadius: 16,
            color: "white",
            background: `linear-gradient(135deg, ${palette.blue}, ${palette.blueDeep})`,
            boxShadow: "0 18px 38px rgba(24, 139, 214, 0.26)",
            fontSize: 26,
            lineHeight: 1,
            fontWeight: 720,
            letterSpacing: 0.2,
            opacity: interpolate(frame, [1.15 * fps, 1.7 * fps], [0, 1], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            }),
            translate: interpolate(frame, [1.15 * fps, 1.7 * fps], ["0px 18px", "0px 0px"], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            }),
          }}
        >
          shelldeck.1clic.pro
        </Interactive.Div>
      </Interactive.Div>
    </AbsoluteFill>
  );
};
