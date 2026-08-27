import {
  AbsoluteFill,
  Easing,
  Interactive,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";
import { palette } from "../../theme";
import { JourneyBackdrop } from "../JourneyBackdrop";
import { RoleBadge } from "../RoleBadge";
import { MobileHeader } from "./MobileShared";

export const MobileIntroScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <JourneyBackdrop tone="blend" />
      <MobileHeader />

      <Interactive.Div
        name="Mobile intro copy"
        style={{
          position: "absolute",
          left: 80,
          right: 80,
          top: 220,
          textAlign: "center",
          opacity: interpolate(frame, [0.1 * fps, 0.7 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(
            frame,
            [0.1 * fps, 0.7 * fps],
            ["0px 28px", "0px 0px"],
            {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            },
          ),
        }}
      >
        <div
          style={{
            color: palette.blueDeep,
            fontSize: 18,
            fontWeight: 800,
            letterSpacing: 3,
            textTransform: "uppercase",
          }}
        >
          Un parcours partagé
        </div>
        <Interactive.H1
          name="Mobile intro headline"
          style={{
            margin: "22px 0 20px",
            color: palette.ink,
            fontSize: 84,
            lineHeight: 0.98,
            fontWeight: 790,
            letterSpacing: -4,
          }}
        >
          Une demande.
          <br />
          <span style={{ color: "#6d5ce7" }}>Deux perspectives.</span>
        </Interactive.H1>
        <p
          style={{
            margin: 0,
            color: palette.muted,
            fontSize: 32,
            lineHeight: 1.35,
          }}
        >
          L’utilisateur explique. Le support agit.
          <br />
          ShellDeck garde le fil.
        </p>
      </Interactive.Div>

      <Interactive.Div
        name="Mobile journey flow"
        style={{
          position: "absolute",
          left: 80,
          right: 80,
          top: 700,
          height: 300,
          display: "grid",
          gridTemplateColumns: "max-content 1fr 390px 1fr max-content",
          columnGap: 8,
          alignItems: "center",
        }}
      >
        <Interactive.Div
          name="Mobile user badge"
          style={{
            zIndex: 2,
            opacity: interpolate(frame, [0.7 * fps, 1.2 * fps], [0, 1], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
            }),
            translate: interpolate(
              frame,
              [0.7 * fps, 1.2 * fps],
              ["-24px 0px", "0px 0px"],
              {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
                easing: Easing.bezier(0.16, 1, 0.3, 1),
              },
            ),
          }}
        >
          <RoleBadge role="Utilisateur" compact />
        </Interactive.Div>
        <div
          className="us-flow-line"
          style={{
            height: 3,
            scale: `${interpolate(frame, [1 * fps, 1.5 * fps], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })} 1`,
            transformOrigin: "left center",
          }}
        />
        <Interactive.Div
          name="Mobile request card"
          className="us-glass"
          style={{
            zIndex: 3,
            padding: "28px 30px",
            borderRadius: 24,
            opacity: interpolate(frame, [1.2 * fps, 1.8 * fps], [0, 1], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
            }),
            translate: interpolate(
              frame,
              [1.2 * fps, 1.8 * fps],
              ["0px 18px", "0px 0px"],
              {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
                easing: Easing.bezier(0.16, 1, 0.3, 1),
              },
            ),
          }}
        >
          <div
            style={{
              color: "#168ee0",
              fontSize: 15,
              fontWeight: 800,
              letterSpacing: 1.4,
            }}
          >
            DEMANDE #248
          </div>
          <div
            style={{
              marginTop: 16,
              color: palette.ink,
              fontSize: 25,
              lineHeight: 1.14,
              fontWeight: 760,
            }}
          >
            Accès au serveur de préproduction
          </div>
          <div style={{ marginTop: 12, color: palette.muted, fontSize: 16 }}>
            Atelier Nord · contexte joint
          </div>
        </Interactive.Div>
        <div
          className="us-flow-line"
          style={{
            height: 3,
            scale: `${interpolate(frame, [1.45 * fps, 1.9 * fps], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })} 1`,
            transformOrigin: "left center",
          }}
        />
        <Interactive.Div
          name="Mobile support badge"
          style={{
            zIndex: 2,
            opacity: interpolate(frame, [1.35 * fps, 1.9 * fps], [0, 1], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
            }),
            translate: interpolate(
              frame,
              [1.35 * fps, 1.9 * fps],
              ["24px 0px", "0px 0px"],
              {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
                easing: Easing.bezier(0.16, 1, 0.3, 1),
              },
            ),
          }}
        >
          <RoleBadge role="Support" compact />
        </Interactive.Div>
      </Interactive.Div>
    </AbsoluteFill>
  );
};
