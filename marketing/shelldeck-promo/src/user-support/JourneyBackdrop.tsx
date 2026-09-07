import {
  AbsoluteFill,
  Easing,
  Interactive,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

type JourneyTone = "user" | "support" | "blend";

const tones: Record<JourneyTone, { primary: string; secondary: string }> = {
  user: { primary: "#168ee0", secondary: "#35c8ad" },
  support: { primary: "#6d5ce7", secondary: "#2f9cea" },
  blend: { primary: "#168ee0", secondary: "#8065ec" },
};

export const JourneyBackdrop: React.FC<{ tone: JourneyTone }> = ({ tone }) => {
  const frame = useCurrentFrame();
  const { durationInFrames } = useVideoConfig();
  const colors = tones[tone];

  return (
    <AbsoluteFill style={{ overflow: "hidden", backgroundColor: "#f8fbff" }}>
      <AbsoluteFill
        style={{
          background:
            "linear-gradient(135deg, rgba(255,255,255,0.98), rgba(246,250,255,0.88) 48%, rgba(255,255,255,0.96))",
        }}
      />
      <Interactive.Div
        name="Primary color field"
        style={{
          position: "absolute",
          width: 820,
          height: 820,
          left: -260,
          top: -280,
          borderRadius: "44% 56% 62% 38% / 48% 40% 60% 52%",
          background: `radial-gradient(circle at 55% 55%, ${colors.primary}58, ${colors.primary}12 48%, transparent 72%)`,
          filter: "blur(20px)",
          opacity: 0.72,
          translate: interpolate(
            frame,
            [0, durationInFrames - 1],
            ["0px 0px", "100px 60px"],
            {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.4, 0, 0.2, 1),
            },
          ),
          rotate: interpolate(frame, [0, durationInFrames - 1], ["-8deg", "6deg"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          }),
        }}
      />
      <Interactive.Div
        name="Secondary color field"
        style={{
          position: "absolute",
          width: 920,
          height: 920,
          right: -280,
          bottom: -390,
          borderRadius: "62% 38% 44% 56% / 40% 54% 46% 60%",
          background: `radial-gradient(circle at 42% 38%, ${colors.secondary}54, ${colors.secondary}12 50%, transparent 73%)`,
          filter: "blur(24px)",
          opacity: 0.76,
          translate: interpolate(
            frame,
            [0, durationInFrames - 1],
            ["0px 0px", "-90px -48px"],
            {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.4, 0, 0.2, 1),
            },
          ),
          rotate: interpolate(frame, [0, durationInFrames - 1], ["5deg", "-7deg"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          }),
        }}
      />
      <AbsoluteFill className="us-micro-grid" style={{ opacity: 0.7 }} />
      <AbsoluteFill
        style={{
          background:
            "linear-gradient(90deg, rgba(255,255,255,0.34), transparent 24%, transparent 76%, rgba(255,255,255,0.36))",
        }}
      />
    </AbsoluteFill>
  );
};
