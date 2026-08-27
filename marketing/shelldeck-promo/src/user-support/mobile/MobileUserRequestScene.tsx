import {
  AbsoluteFill,
  Easing,
  Interactive,
  interpolate,
  useCurrentFrame,
} from "remotion";
import { palette } from "../../theme";
import { ActionCursor } from "../ActionCursor";
import { JourneyBackdrop } from "../JourneyBackdrop";
import { MobileHeader, MobilePill, MobileWindowChrome } from "./MobileShared";

export const MobileUserRequestScene: React.FC = () => {
  const frame = useCurrentFrame();
  const title = "Accès au serveur de préproduction";
  const details = "Préparer un accès temporaire pour la mise en ligne.";
  const typedTitle = title.slice(
    0,
    Math.floor(
      interpolate(frame, [34, 76], [0, title.length], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
      }),
    ),
  );
  const typedDetails = details.slice(
    0,
    Math.floor(
      interpolate(frame, [70, 115], [0, details.length], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
      }),
    ),
  );

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <JourneyBackdrop tone="user" />
      <MobileHeader role="Utilisateur" />

      <Interactive.Div
        name="Mobile user request copy"
        style={{
          position: "absolute",
          left: 80,
          right: 80,
          top: 170,
          opacity: interpolate(frame, [0, 18], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          }),
          translate: interpolate(frame, [0, 20], ["0px 28px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        <Interactive.H2
          name="Mobile user headline"
          style={{
            margin: 0,
            color: palette.ink,
            fontSize: 78,
            lineHeight: 1,
            fontWeight: 790,
            letterSpacing: -3.7,
          }}
        >
          Décrivez le besoin.
          <br />
          <span style={{ color: "#168ee0" }}>Gardez le suivi.</span>
        </Interactive.H2>
      </Interactive.Div>

      <Interactive.Div
        name="Mobile request composer"
        className="us-focus-frame"
        style={{
          position: "absolute",
          left: 80,
          right: 80,
          top: 430,
          height: 790,
          borderRadius: 30,
          opacity: interpolate(frame, [12, 32], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          }),
          translate: interpolate(frame, [12, 34], ["0px 46px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        <MobileWindowChrome title="Nouvelle demande" />
        <div
          style={{
            flex: 1,
            padding: "28px",
            background: "linear-gradient(160deg,#fbfcfe,#f5f8fc)",
          }}
        >
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              padding: "18px 20px",
              borderRadius: 16,
              color: "#3152bd",
              background: "#f0f2ff",
              border: "1px solid rgba(74,90,204,.2)",
              fontSize: 19,
              fontWeight: 760,
            }}
          >
            <span>✦ Préparer avec l’IA</span>
            <span
              style={{
                padding: "7px 11px",
                borderRadius: 9,
                background: "white",
                color: palette.ink,
                fontSize: 14,
              }}
            >
              Codex ›
            </span>
          </div>
          <div
            style={{
              marginTop: 20,
              padding: "26px",
              borderRadius: 22,
              background: "white",
              border: "1px solid #dce3ec",
              boxShadow: "0 14px 36px rgba(41,64,92,.07)",
            }}
          >
            <div style={{ display: "flex", gap: 10 }}>
              <MobilePill color="#65788b">◎ Atelier Nord</MobilePill>
              <MobilePill>● Normale</MobilePill>
            </div>
            <div
              style={{
                marginTop: 30,
                minHeight: 72,
                paddingBottom: 15,
                borderBottom: "2px solid #e5eaf0",
                color: typedTitle ? palette.ink : "#a3adba",
                fontSize: 27,
                lineHeight: 1.2,
                fontWeight: 700,
              }}
            >
              {typedTitle || "Titre de la demande"}
              {frame >= 34 && frame < 78 ? (
                <span style={{ color: "#168ee0" }}>│</span>
              ) : null}
            </div>
            <div
              style={{
                marginTop: 22,
                height: 150,
                color: typedDetails ? palette.muted : "#a3adba",
                fontSize: 21,
                lineHeight: 1.5,
              }}
            >
              {typedDetails || "Détails de la demande…"}
              {frame >= 70 && frame < 118 ? (
                <span style={{ color: "#168ee0" }}>│</span>
              ) : null}
            </div>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                marginTop: 14,
              }}
            >
              <span style={{ color: palette.muted, fontSize: 23 }}>
                ＋ &nbsp; ✦
              </span>
              <div style={{ position: "relative" }}>
                <Interactive.Div
                  name="Mobile create request button"
                  style={{
                    padding: "15px 28px",
                    borderRadius: 14,
                    color: "white",
                    background:
                      frame >= 128
                        ? "linear-gradient(110deg,#20a873,#2abb8d)"
                        : "linear-gradient(110deg,#168ee0,#315bd6)",
                    boxShadow: "0 12px 28px rgba(22,142,224,.25)",
                    fontSize: 19,
                    fontWeight: 780,
                    scale: interpolate(frame, [121, 125, 129], [1, 0.88, 1], {
                      extrapolateLeft: "clamp",
                      extrapolateRight: "clamp",
                      output: "perceptual-scale",
                    }),
                  }}
                >
                  {frame >= 128 ? "Envoyée ✓" : "Créer"}
                </Interactive.Div>
                <ActionCursor
                  name="Mobile create cursor"
                  appearAt={105}
                  clickAt={125}
                  from={[88, -80]}
                  to={[-4, -3]}
                  relativeToParentCenter
                />
              </div>
            </div>
          </div>
          <div style={{ marginTop: 16, color: "#9aa5b4", fontSize: 14 }}>
            PNG, JPEG, WebP · 9 Mo max
          </div>
        </div>
      </Interactive.Div>
    </AbsoluteFill>
  );
};
