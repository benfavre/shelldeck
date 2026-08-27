import {
  AbsoluteFill,
  Easing,
  Interactive,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";
import { Brand } from "../../components/Brand";
import { palette } from "../../theme";
import { JourneyBackdrop } from "../JourneyBackdrop";
import { RoleBadge } from "../RoleBadge";

export const MobileOutcomeScene: React.FC = () => {
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
      <JourneyBackdrop tone="blend" />
      <Interactive.Div
        name="Mobile outcome card"
        className="us-glass"
        style={{
          position: "relative",
          width: 920,
          height: 1120,
          padding: "80px 64px",
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          textAlign: "center",
          borderRadius: 44,
          opacity: interpolate(frame, [0, 20], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          }),
          scale: interpolate(frame, [0, 26], [0.95, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.spring({ damping: 180 }),
            output: "perceptual-scale",
          }),
        }}
      >
        <Brand compact />

        <Interactive.Div
          name="Mobile outcome flow"
          style={{
            width: 760,
            height: 90,
            marginTop: 78,
            display: "grid",
            gridTemplateColumns: "max-content 1fr 70px 1fr max-content",
            columnGap: 10,
            alignItems: "center",
          }}
        >
          <RoleBadge role="Utilisateur" compact />
          <div
            style={{
              height: 3,
              borderRadius: 99,
              background: "linear-gradient(90deg,#168ee0,#35c8ad)",
              scale: `${interpolate(frame, [0.7 * fps, 1.25 * fps], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })} 1`,
              transformOrigin: "left center",
            }}
          />
          <Interactive.Div
            name="Mobile resolved check"
            style={{
              display: "grid",
              placeItems: "center",
              width: 70,
              height: 70,
              borderRadius: 99,
              color: "white",
              background:
                "conic-gradient(from 210deg,#168ee0,#35c8ad,#8065ec,#168ee0)",
              border: "7px solid rgba(255,255,255,.94)",
              boxShadow: "0 18px 38px rgba(73,103,184,.26)",
              fontSize: 31,
              fontWeight: 900,
              opacity: interpolate(frame, [1.15 * fps, 1.7 * fps], [0, 1], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
              }),
              scale: interpolate(frame, [1.15 * fps, 1.85 * fps], [0.6, 1], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
                easing: Easing.spring({ damping: 150 }),
                output: "perceptual-scale",
              }),
            }}
          >
            ✓
          </Interactive.Div>
          <div
            style={{
              height: 3,
              borderRadius: 99,
              background: "linear-gradient(90deg,#35c8ad,#8065ec)",
              scale: `${interpolate(frame, [1.2 * fps, 1.75 * fps], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })} 1`,
              transformOrigin: "left center",
            }}
          />
          <RoleBadge role="Support" compact />
        </Interactive.Div>

        <Interactive.H2
          name="Mobile outcome headline"
          style={{
            margin: "90px 0 28px",
            color: palette.ink,
            fontSize: 82,
            lineHeight: 1,
            fontWeight: 790,
            letterSpacing: -3.8,
          }}
        >
          Du besoin à la résolution.
          <br />
          <span
            style={{
              background: "linear-gradient(90deg,#168ee0,#6d5ce7)",
              backgroundClip: "text",
              color: "transparent",
            }}
          >
            Sans perdre le fil.
          </span>
        </Interactive.H2>
        <Interactive.P
          name="Mobile outcome subtitle"
          style={{
            margin: 0,
            color: palette.muted,
            fontSize: 31,
            lineHeight: 1.45,
          }}
        >
          ShellDeck réunit vos utilisateurs et votre équipe support.
        </Interactive.P>
        <Interactive.Div
          name="Mobile outcome website"
          style={{
            marginTop: 56,
            padding: "20px 32px",
            borderRadius: 17,
            color: "white",
            background: "linear-gradient(100deg,#168ee0,#6d5ce7)",
            boxShadow: "0 18px 42px rgba(65,103,199,.28)",
            fontSize: 25,
            fontWeight: 760,
            opacity: interpolate(frame, [2 * fps, 2.55 * fps], [0, 1], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
            }),
            translate: interpolate(
              frame,
              [2 * fps, 2.55 * fps],
              ["0px 18px", "0px 0px"],
              {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
                easing: Easing.bezier(0.16, 1, 0.3, 1),
              },
            ),
          }}
        >
          shelldeck.1clic.pro
        </Interactive.Div>
      </Interactive.Div>
    </AbsoluteFill>
  );
};
