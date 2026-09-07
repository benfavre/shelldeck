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
import { JourneyBackdrop } from "../JourneyBackdrop";
import { RoleBadge } from "../RoleBadge";

const steps = ["Décrire", "Joindre", "Envoyer"];

export const UserRequestScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const requestTitle = "Accès au serveur de préproduction";
  const requestDetails = "Préparer un accès temporaire pour la mise en ligne de l’Atelier Nord.";
  const typedTitle = requestTitle.slice(
    0,
    Math.floor(
      interpolate(frame, [36, 76], [0, requestTitle.length], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
      }),
    ),
  );
  const typedDetails = requestDetails.slice(
    0,
    Math.floor(
      interpolate(frame, [70, 116], [0, requestDetails.length], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
      }),
    ),
  );

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <JourneyBackdrop tone="user" />
      <Interactive.Div
        name="User scene brand"
        style={{ position: "absolute", left: 104, top: 70 }}
      >
        <Brand compact />
      </Interactive.Div>
      <Interactive.Div
        name="User role"
        style={{ position: "absolute", right: 104, top: 74 }}
      >
        <RoleBadge role="Utilisateur" />
      </Interactive.Div>

      <Interactive.Div
        name="User request copy"
        style={{
          position: "absolute",
          left: 104,
          top: 190,
          width: 610,
          zIndex: 4,
          opacity: interpolate(frame, [0, 0.65 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [0, 0.65 * fps], ["0px 36px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        <Interactive.H2
          name="User request headline"
          style={{
            margin: 0,
            color: palette.ink,
            fontSize: 82,
            lineHeight: 1.01,
            fontWeight: 790,
            letterSpacing: -4.2,
          }}
        >
          Décrivez le besoin.
          <br />
          <span style={{ color: "#168ee0" }}>Gardez le suivi.</span>
        </Interactive.H2>
        <Interactive.P
          name="User request subtitle"
          style={{
            margin: "28px 0 0",
            color: palette.muted,
            fontSize: 30,
            lineHeight: 1.4,
          }}
        >
          Une demande claire, son site, sa priorité et ses pièces jointes.
        </Interactive.P>
      </Interactive.Div>

      <Interactive.Div
        name="User request steps"
        style={{
          position: "absolute",
          left: 104,
          top: 595,
          zIndex: 5,
          display: "flex",
          flexDirection: "column",
          gap: 14,
        }}
      >
        {steps.map((step, index) => (
          <Interactive.Div
            key={step}
            name={`Step ${step}`}
            className="us-glass"
            style={{
              width: 294,
              display: "flex",
              alignItems: "center",
              gap: 16,
              padding: "15px 18px",
              borderRadius: 18,
              color: palette.ink,
              fontSize: 22,
              fontWeight: 650,
              opacity: interpolate(
                frame,
                [(0.8 + index * 0.22) * fps, (1.25 + index * 0.22) * fps],
                [0, 1],
                {
                  extrapolateLeft: "clamp",
                  extrapolateRight: "clamp",
                  easing: Easing.bezier(0.16, 1, 0.3, 1),
                },
              ),
              translate: interpolate(
                frame,
                [(0.8 + index * 0.22) * fps, (1.25 + index * 0.22) * fps],
                ["-28px 0px", "0px 0px"],
                {
                  extrapolateLeft: "clamp",
                  extrapolateRight: "clamp",
                  easing: Easing.bezier(0.16, 1, 0.3, 1),
                },
              ),
            }}
          >
            <span
              style={{
                display: "grid",
                placeItems: "center",
                width: 34,
                height: 34,
                borderRadius: 12,
                color: "white",
                background: frame >= 72 + index * 22 ? "linear-gradient(135deg, #20a873, #35c99b)" : "linear-gradient(135deg, #168ee0, #39b9e5)",
                boxShadow: "0 8px 18px rgba(22,142,224,0.26)",
                fontSize: 16,
                fontWeight: 800,
              }}
            >
              {frame >= 72 + index * 22 ? "✓" : index + 1}
            </span>
            {step}
          </Interactive.Div>
        ))}
      </Interactive.Div>

      <Interactive.Div
        name="Illustrated request composer"
        className="us-focus-frame"
        style={{
          position: "absolute",
          width: 840,
          height: 610,
          right: 104,
          bottom: 92,
          borderRadius: 30,
          opacity: interpolate(frame, [0.35 * fps, 1.15 * fps], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          translate: interpolate(frame, [0.35 * fps, 1.15 * fps], ["64px 34px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
          scale: interpolate(frame, [0.35 * fps, 1.15 * fps], [0.96, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
            output: "perceptual-scale",
          }),
        }}
      >
        <div className="us-focus-chrome" style={{ height: 58, display: "flex", alignItems: "center", gap: 10, padding: "0 22px" }}>
          <span className="us-window-dot" style={{ backgroundColor: "#ff786b" }} />
          <span className="us-window-dot" style={{ backgroundColor: "#ffc64f" }} />
          <span className="us-window-dot" style={{ backgroundColor: "#48c78e" }} />
          <span style={{ marginLeft: 12, color: "#65758a", fontSize: 17, fontWeight: 700 }}>Nouvelle demande</span>
        </div>
        <div style={{ flex: 1, padding: "28px 30px 26px", background: "linear-gradient(160deg, #fbfcfe, #f5f8fc)" }}>
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "18px 20px", borderRadius: 16, color: "#3152bd", background: "linear-gradient(135deg, #eef3ff, #f6f4ff)", border: "1px solid rgba(74,90,204,.22)", fontSize: 18, fontWeight: 760 }}>
            <span>✦ Préparer avec l’IA</span>
            <span style={{ padding: "7px 12px", borderRadius: 10, color: palette.ink, background: "white", border: "1px solid #dce3ec", fontSize: 14 }}>Codex ›</span>
          </div>
          <div style={{ marginTop: 18, padding: "24px 24px 20px", borderRadius: 20, background: "white", border: "1px solid #dce3ec", boxShadow: "0 12px 34px rgba(41,64,92,.07)" }}>
            <div style={{ display: "flex", gap: 10 }}>
              <span style={{ padding: "8px 12px", borderRadius: 999, color: palette.muted, background: "#f0f3f7", fontSize: 14, fontWeight: 700 }}>◎ Aucun site précis</span>
              <span style={{ padding: "8px 12px", borderRadius: 999, color: "#168ee0", background: "#edf7ff", fontSize: 14, fontWeight: 750 }}>● Normale</span>
            </div>
            <div style={{ marginTop: 28, minHeight: 46, paddingBottom: 14, borderBottom: "2px solid #e5eaf0", color: typedTitle ? palette.ink : "#a3adba", fontSize: 24, fontWeight: 680 }}>
              {typedTitle || "Titre de la demande"}
              {frame >= 36 && frame < 78 ? <span style={{ color: "#168ee0" }}>│</span> : null}
            </div>
            <div style={{ marginTop: 20, height: 112, color: typedDetails ? palette.muted : "#a3adba", fontSize: 18, lineHeight: 1.5 }}>
              {typedDetails || "Détails de la demande…"}
              {frame >= 70 && frame < 118 ? <span style={{ color: "#168ee0" }}>│</span> : null}
            </div>
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginTop: 12 }}>
              <span style={{ color: palette.muted, fontSize: 20 }}>＋ &nbsp; ✦</span>
              <div style={{ position: "relative" }}>
                <Interactive.Div
                  name="Create request button"
                  style={{
                    padding: "13px 24px",
                    borderRadius: 13,
                    color: "white",
                    background: frame >= 128 ? "linear-gradient(110deg, #20a873, #2abb8d)" : "linear-gradient(110deg, #168ee0, #315bd6)",
                    boxShadow: "0 12px 28px rgba(22,142,224,.25)",
                    fontSize: 17,
                    fontWeight: 780,
                    opacity: interpolate(frame, [42, 58], [0.55, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
                    scale: interpolate(frame, [121, 125, 129], [1, 0.87, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.spring({ damping: 170 }), output: "perceptual-scale" }),
                  }}
                >
                  {frame >= 128 ? "Envoyée ✓" : "Créer"}
                </Interactive.Div>
                <ActionCursor
                  name="Create request cursor"
                  appearAt={106}
                  clickAt={125}
                  from={[94, -92]}
                  to={[-4, -3]}
                  relativeToParentCenter
                />
              </div>
            </div>
          </div>
          <div style={{ marginTop: 13, color: "#9aa5b4", fontSize: 13 }}>PNG, JPEG, WebP · 9 Mo max</div>
        </div>
      </Interactive.Div>
      <Interactive.Div
        name="Request sent confirmation"
        className="us-glass"
        style={{
          position: "absolute",
          right: 136,
          top: 208,
          width: 365,
          zIndex: 18,
          display: "flex",
          alignItems: "center",
          gap: 13,
          padding: "15px 20px",
          borderRadius: 17,
          color: "#167a58",
          fontSize: 17,
          fontWeight: 760,
          opacity: interpolate(frame, [127, 135], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
          translate: interpolate(frame, [127, 137], ["0px 18px", "0px 0px"], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.spring({ damping: 180 }) }),
        }}
      >
        <span style={{ display: "grid", placeItems: "center", width: 28, height: 28, borderRadius: 99, color: "white", background: "#20a873" }}>✓</span>
        Demande transmise au Support
      </Interactive.Div>
    </AbsoluteFill>
  );
};
