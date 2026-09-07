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
import { Pill } from "../components/Pill";
import { ScreenshotCard } from "../components/ScreenshotCard";
import { palette } from "../theme";

export const HeroScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <LightBackdrop imageOpacity={0.16} />
      <Interactive.Div
        name="Hero copy"
        style={{
          position: "absolute",
          left: 112,
          top: 126,
          width: 800,
          opacity: interpolate(frame, [0, 0.55 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [0, 0.7 * fps], ["0px 42px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        <Brand />
        <Interactive.H1
          name="Hero headline"
          style={{
            margin: "58px 0 26px",
            color: palette.ink,
            fontSize: 96,
            lineHeight: 0.98,
            fontWeight: 790,
            letterSpacing: -5,
          }}
        >
          Tous vos serveurs.
          <br />
          <span style={{ color: palette.blue }}>Un seul cockpit.</span>
        </Interactive.H1>
        <Interactive.P
          name="Hero subtitle"
          style={{
            margin: 0,
            width: 700,
            color: palette.muted,
            fontSize: 34,
            lineHeight: 1.35,
            fontWeight: 480,
          }}
        >
          SSH, terminaux, scripts et tunnels réunis dans une application native.
        </Interactive.P>
      </Interactive.Div>

      <ScreenshotCard
        src="shelldeck-hero.png"
        name="ShellDeck user dashboard"
        style={{
          width: 980,
          height: 551,
          right: -132,
          top: 246,
          opacity: interpolate(frame, [0.45 * fps, 1.25 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          scale: interpolate(frame, [0.45 * fps, 1.4 * fps], [0.9, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.spring({ damping: 180 }),
            output: "perceptual-scale",
          }),
          rotate: interpolate(frame, [0.45 * fps, 1.4 * fps], ["1.8deg", "0deg"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      />

      <Interactive.Div
        name="Feature pills"
        style={{
          position: "absolute",
          left: 112,
          bottom: 98,
          display: "flex",
          gap: 16,
          opacity: interpolate(frame, [1.25 * fps, 1.8 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [1.25 * fps, 1.8 * fps], ["0px 24px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        <Pill>SSH natif</Pill>
        <Pill accent="teal">Sessions persistantes</Pill>
        <Pill accent="amber">Multi-sites</Pill>
      </Interactive.Div>
    </AbsoluteFill>
  );
};
