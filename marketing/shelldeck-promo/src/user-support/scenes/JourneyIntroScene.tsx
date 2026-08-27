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

const STAGE_WIDTH = 1320;
const NODE_SIZE = 144;
const NODE_CENTER_Y = 112;
const USER_CENTER_X = 132;
const SUPPORT_CENTER_X = STAGE_WIDTH - USER_CENTER_X;
const LINE_START_X = USER_CENTER_X + NODE_SIZE / 2 - 1;
const LINE_END_X = SUPPORT_CENTER_X - NODE_SIZE / 2 + 1;

const Person: React.FC<{
  role: "Utilisateur" | "Support";
  color: string;
  symbol: string;
  centerX: number;
  opacity: number;
  offsetX: number;
}> = ({
  role,
  color,
  symbol,
  centerX,
  opacity,
  offsetX,
}) => (
  <div
    style={{
      position: "absolute",
      left: centerX - 120,
      top: NODE_CENTER_Y - NODE_SIZE / 2,
      zIndex: 2,
      display: "flex",
      width: 240,
      flexDirection: "column",
      alignItems: "center",
      gap: 18,
      opacity,
      translate: `${offsetX}px 0px`,
    }}
  >
    <div
      style={{
        display: "grid",
        placeItems: "center",
        width: NODE_SIZE,
        height: NODE_SIZE,
        borderRadius: 999,
        color,
        background: "rgba(255,255,255,0.92)",
        border: `2px solid ${color}2d`,
        boxShadow: `0 28px 70px ${color}24`,
        fontSize: 58,
        fontWeight: 800,
      }}
    >
      {symbol}
    </div>
    <RoleBadge role={role} compact />
  </div>
);

export const JourneyIntroScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const lineProgress = interpolate(frame, [1.02 * fps, 1.92 * fps], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });
  const cardProgress = interpolate(frame, [1.28 * fps, 1.82 * fps], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <JourneyBackdrop tone="blend" />
      <Interactive.Div name="Journey brand" style={{ position: "absolute", left: 104, top: 74 }}>
        <Brand compact />
      </Interactive.Div>

      <Interactive.Div
        name="Journey intro copy"
        style={{
          position: "absolute",
          left: 260,
          right: 260,
          top: 168,
          textAlign: "center",
          opacity: interpolate(frame, [0.1 * fps, 0.75 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [0.1 * fps, 0.75 * fps], ["0px 28px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        <div style={{ color: palette.blueDeep, fontSize: 21, fontWeight: 750, letterSpacing: 3.2, textTransform: "uppercase" }}>
          Un parcours partagé
        </div>
        <Interactive.H1
          name="Journey headline"
          style={{ margin: "18px 0 16px", color: palette.ink, fontSize: 86, lineHeight: 0.98, fontWeight: 790, letterSpacing: -4.5 }}
        >
          Une demande. <span style={{ color: "#6d5ce7" }}>Deux perspectives.</span>
        </Interactive.H1>
        <p style={{ margin: 0, color: palette.muted, fontSize: 30 }}>
          L’utilisateur explique. Le support agit. ShellDeck garde le fil.
        </p>
      </Interactive.Div>

      <Interactive.Div
        name="Minimal journey illustration"
        style={{
          position: "absolute",
          left: "50%",
          bottom: 104,
          width: STAGE_WIDTH,
          height: 312,
          translate: "-50% 0px",
        }}
      >
        <Interactive.Div
          name="User node"
        >
          <Person
            role="Utilisateur"
            color="#168ee0"
            symbol="U"
            centerX={USER_CENTER_X}
            opacity={interpolate(frame, [0.7 * fps, 1.25 * fps], [0, 1], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
            })}
            offsetX={interpolate(frame, [0.7 * fps, 1.25 * fps], [-30, 0], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            })}
          />
        </Interactive.Div>

        <div
          className="us-flow-line"
          style={{
            position: "absolute",
            left: LINE_START_X,
            top: NODE_CENTER_Y - 2,
            width: LINE_END_X - LINE_START_X,
            zIndex: 1,
            scale: `${lineProgress} 1`,
            transformOrigin: "left center",
          }}
        />

        <Interactive.Div
          name="Request ticket"
          style={{
            position: "absolute",
            left: "50%",
            top: NODE_CENTER_Y,
            zIndex: 3,
            translate: "-50% -50%",
          }}
        >
          <div
            className="us-glass"
            style={{
              width: 438,
              padding: "25px 29px 27px",
              borderRadius: 24,
              borderColor: "rgba(176, 194, 214, 0.34)",
              background: "rgba(255,255,255,0.94)",
              boxShadow: "0 24px 62px rgba(39, 64, 94, 0.14), 0 6px 18px rgba(39, 64, 94, 0.07)",
              opacity: cardProgress,
              translate: `0px ${interpolate(cardProgress, [0, 1], [16, 0])}px`,
            }}
          >
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <span style={{ color: "#168ee0", fontSize: 16, fontWeight: 800, letterSpacing: 1.45 }}>DEMANDE #248</span>
              <span style={{ padding: "6px 11px", borderRadius: 999, color: "#6d5ce7", background: "#eeeafe", fontSize: 13, fontWeight: 750 }}>Normale</span>
            </div>
            <div style={{ marginTop: 17, maxWidth: 330, color: palette.ink, fontSize: 25, lineHeight: 1.14, fontWeight: 760 }}>Accès au serveur de préproduction</div>
            <div style={{ marginTop: 12, color: palette.muted, fontSize: 16 }}>Atelier Nord · contexte joint</div>
          </div>
        </Interactive.Div>

        <Interactive.Div
          name="Support node"
        >
          <Person
            role="Support"
            color="#6d5ce7"
            symbol="S"
            centerX={SUPPORT_CENTER_X}
            opacity={interpolate(frame, [1.25 * fps, 1.85 * fps], [0, 1], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
            })}
            offsetX={interpolate(frame, [1.25 * fps, 1.85 * fps], [30, 0], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            })}
          />
        </Interactive.Div>
      </Interactive.Div>
    </AbsoluteFill>
  );
};
