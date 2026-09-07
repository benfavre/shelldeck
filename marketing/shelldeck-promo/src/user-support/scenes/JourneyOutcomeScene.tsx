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

export const JourneyOutcomeScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  return (
    <AbsoluteFill style={{ overflow: "hidden", alignItems: "center", justifyContent: "center" }}>
      <JourneyBackdrop tone="blend" />
      <Interactive.Div
        name="Outcome glass card"
        className="us-glass"
        style={{
          position: "relative",
          width: 1400,
          minHeight: 790,
          padding: "76px 108px",
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          textAlign: "center",
          borderRadius: 46,
          opacity: interpolate(frame, [0, 0.65 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          scale: interpolate(frame, [0, 0.85 * fps], [0.94, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.spring({ damping: 180 }),
            output: "perceptual-scale",
          }),
        }}
      >
        <Brand compact />

        <Interactive.Div
          name="Role resolution flow"
          style={{
            position: "relative",
            marginTop: 58,
            width: 760,
            height: 86,
            display: "grid",
            gridTemplateColumns: "max-content 1fr 70px 1fr max-content",
            columnGap: 10,
            alignItems: "center",
          }}
        >
          <RoleBadge role="Utilisateur" compact />
          <Interactive.Div
            name="User to resolution line"
            style={{
              height: 3,
              borderRadius: 99,
              background: "linear-gradient(90deg, #168ee0, #35c8ad)",
              scale: `${interpolate(frame, [0.7 * fps, 1.25 * fps], [0, 1], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
                easing: Easing.bezier(0.16, 1, 0.3, 1),
              })} 1`,
              transformOrigin: "left center",
              boxShadow: "0 0 20px rgba(109,92,231,0.32)",
            }}
          />
          <Interactive.Div
            name="Resolved check"
            style={{
              zIndex: 2,
              display: "grid",
              placeItems: "center",
              width: 70,
              height: 70,
              borderRadius: 99,
              color: "white",
              background: "conic-gradient(from 210deg, #168ee0, #35c8ad, #8065ec, #168ee0)",
              border: "7px solid rgba(255,255,255,0.92)",
              boxShadow: "0 18px 38px rgba(73,103,184,0.26)",
              fontSize: 31,
              fontWeight: 900,
              opacity: interpolate(frame, [1.2 * fps, 1.75 * fps], [0, 1], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
                easing: Easing.bezier(0.16, 1, 0.3, 1),
              }),
              scale: interpolate(frame, [1.2 * fps, 1.9 * fps], [0.6, 1], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
                easing: Easing.spring({ damping: 150 }),
                output: "perceptual-scale",
              }),
            }}
          >
            ✓
          </Interactive.Div>
          <Interactive.Div
            name="Resolution to support line"
            style={{
              height: 3,
              borderRadius: 99,
              background: "linear-gradient(90deg, #35c8ad, #8065ec)",
              scale: `${interpolate(frame, [1.2 * fps, 1.75 * fps], [0, 1], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
                easing: Easing.bezier(0.16, 1, 0.3, 1),
              })} 1`,
              transformOrigin: "left center",
              boxShadow: "0 0 20px rgba(109,92,231,0.32)",
            }}
          />
          <RoleBadge role="Support" compact />
        </Interactive.Div>

        <Interactive.H2
          name="Outcome headline"
          style={{
            margin: "48px 0 22px",
            color: palette.ink,
            fontSize: 84,
            lineHeight: 1.01,
            fontWeight: 790,
            letterSpacing: -4.2,
          }}
        >
          Du besoin à la résolution.
          <br />
          <span
            style={{
              background: "linear-gradient(90deg, #168ee0, #6d5ce7)",
              backgroundClip: "text",
              color: "transparent",
            }}
          >
            Sans perdre le fil.
          </span>
        </Interactive.H2>
        <Interactive.P
          name="Outcome subtitle"
          style={{ margin: 0, color: palette.muted, fontSize: 30, lineHeight: 1.4 }}
        >
          ShellDeck réunit vos utilisateurs et votre équipe support.
        </Interactive.P>
        <Interactive.Div
          name="Outcome website"
          style={{
            marginTop: 42,
            padding: "18px 32px",
            borderRadius: 17,
            color: "white",
            background: "linear-gradient(100deg, #168ee0, #6d5ce7)",
            boxShadow: "0 18px 42px rgba(65,103,199,0.28)",
            fontSize: 25,
            lineHeight: 1,
            fontWeight: 750,
            opacity: interpolate(frame, [2 * fps, 2.55 * fps], [0, 1], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            }),
            translate: interpolate(frame, [2 * fps, 2.55 * fps], ["0px 18px", "0px 0px"], {
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
