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
import { ActionCursor } from "../ActionCursor";
import { FocusedCapture } from "../FocusedCapture";
import { JourneyBackdrop } from "../JourneyBackdrop";
import { RoleBadge } from "../RoleBadge";

const contextItems = ["Ticket sélectionné", "Résumé prêt", "Réponse proposée"];

export const AssistResolveScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <JourneyBackdrop tone="support" />
      <Interactive.Div
        name="Assist scene brand"
        style={{ position: "absolute", left: 104, top: 70 }}
      >
        <Brand compact />
      </Interactive.Div>
      <Interactive.Div
        name="Assist support role"
        style={{ position: "absolute", right: 104, top: 74 }}
      >
        <RoleBadge role="Support" />
      </Interactive.Div>

      <Interactive.Div
        name="Assist copy"
        style={{
          position: "absolute",
          left: 104,
          top: 180,
          width: 820,
          zIndex: 5,
          opacity: interpolate(frame, [0, 0.65 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [0, 0.65 * fps], ["0px 34px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        <Interactive.H2
          name="Assist headline"
          style={{
            margin: 0,
            color: palette.ink,
            fontSize: 76,
            lineHeight: 1.01,
            fontWeight: 790,
            letterSpacing: -4.2,
          }}
        >
          Le contexte est déjà là.
          <br />
          <span
            style={{
              background: "linear-gradient(90deg, #6d5ce7, #168ee0)",
              backgroundClip: "text",
              color: "transparent",
            }}
          >
            L’IA aide à agir.
          </span>
        </Interactive.H2>
      </Interactive.Div>

      <FocusedCapture
        src="ai-support.webp"
        name="Support AI assistant"
        label="Assistant contextuel"
        zoom={2.25}
        imagePosition="right"
        style={{
          right: 104,
          bottom: 92,
          width: 800,
          height: 720,
          opacity: interpolate(frame, [0.45 * fps, 1.2 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [0.45 * fps, 1.2 * fps], ["56px 42px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          scale: interpolate(frame, [0.45 * fps, 1.2 * fps], [0.96, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
            output: "perceptual-scale",
          }),
        }}
      >
        <Interactive.Div
          name="AI panel glow"
          style={{
            position: "absolute",
            left: 22,
            right: 22,
            top: 276,
            height: 80,
            border: "2px solid rgba(109,92,231,0.58)",
            borderRadius: 16,
            boxShadow: "0 0 46px rgba(109,92,231,0.16)",
            opacity: interpolate(frame, [1.25 * fps, 1.85 * fps], [0, 1], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            }),
          }}
        />
        <Interactive.Div
          name="Generated AI summary"
          style={{
            position: "absolute",
            right: 28,
            bottom: 30,
            width: 500,
            padding: "22px 24px",
            borderRadius: 20,
            color: palette.ink,
            background: "rgba(255,255,255,.97)",
            border: "1px solid rgba(109,92,231,.25)",
            boxShadow: "0 20px 48px rgba(62,52,145,.18)",
            opacity: interpolate(frame, [82, 94], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
            translate: interpolate(frame, [82, 96], ["24px 24px", "0px 0px"], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.spring({ damping: 180 }) }),
          }}
        >
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", color: "#6d5ce7", fontSize: 16, fontWeight: 800 }}>
            <span>✦ Résumé généré</span>
            <span style={{ color: "#20a873" }}>Prêt ✓</span>
          </div>
          <div style={{ marginTop: 16, color: palette.muted, fontSize: 16, lineHeight: 1.45 }}>
            Accès temporaire demandé pour Atelier Nord. Priorité normale, mise en ligne imminente.
          </div>
          <div style={{ marginTop: 17, padding: "12px 15px", borderRadius: 13, color: "#3152bd", background: "#f0f3ff", fontSize: 15, fontWeight: 750 }}>
            Utiliser dans la réponse →
          </div>
        </Interactive.Div>
      </FocusedCapture>

      <Interactive.Div
        name="AI context stack"
        style={{
          position: "absolute",
          left: 104,
          bottom: 132,
          zIndex: 6,
          display: "flex",
          flexDirection: "column",
          alignItems: "flex-start",
          gap: 12,
        }}
      >
        {contextItems.map((item, index) => (
          <Interactive.Div
            key={item}
            name={item}
            className="us-glass"
            style={{
              display: "flex",
              alignItems: "center",
              gap: 12,
              padding: "14px 18px",
              borderRadius: 17,
              color: palette.ink,
              fontSize: 19,
              fontWeight: 700,
              opacity: interpolate(
                frame,
                [(1.7 + index * 0.25) * fps, (2.15 + index * 0.25) * fps],
                [0, 1],
                {
                  extrapolateLeft: "clamp",
                  extrapolateRight: "clamp",
                  easing: Easing.bezier(0.16, 1, 0.3, 1),
                },
              ),
              translate: interpolate(
                frame,
                [(1.7 + index * 0.25) * fps, (2.15 + index * 0.25) * fps],
                ["-28px 0px", "0px 0px"],
                {
                  extrapolateLeft: "clamp",
                  extrapolateRight: "clamp",
                  easing: Easing.bezier(0.16, 1, 0.3, 1),
                },
              ),
            }}
          >
            <span style={{ color: "#6d5ce7", fontSize: 20 }}>✦</span>
            {item}
          </Interactive.Div>
        ))}
      </Interactive.Div>
      <ActionCursor name="Summarize ticket cursor" appearAt={48} clickAt={68} from={[1810, 510]} to={[1604, 638]} color="#6d5ce7" />
    </AbsoluteFill>
  );
};
