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
import { MobileHeader, MobileWindowChrome } from "./MobileShared";

export const MobileAssistScene: React.FC = () => {
  const frame = useCurrentFrame();

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <JourneyBackdrop tone="support" />
      <MobileHeader role="Support" />
      <Interactive.Div
        name="Mobile assist copy"
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
          name="Mobile assist headline"
          style={{
            margin: 0,
            color: palette.ink,
            fontSize: 78,
            lineHeight: 1,
            fontWeight: 790,
            letterSpacing: -3.7,
          }}
        >
          Le contexte est déjà là.
          <br />
          <span
            style={{
              background: "linear-gradient(90deg,#6d5ce7,#168ee0)",
              backgroundClip: "text",
              color: "transparent",
            }}
          >
            L’IA aide à agir.
          </span>
        </Interactive.H2>
      </Interactive.Div>

      <Interactive.Div
        name="Mobile AI assistant"
        className="us-focus-frame"
        style={{
          position: "absolute",
          left: 80,
          right: 80,
          top: 430,
          height: 800,
          borderRadius: 30,
          opacity: interpolate(frame, [12, 34], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          }),
          translate: interpolate(frame, [12, 36], ["0px 46px", "0px 0px"], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
          }),
        }}
      >
        <MobileWindowChrome title="Assistant contextuel" />
        <div
          style={{
            flex: 1,
            padding: "30px",
            background: "linear-gradient(160deg,#fbfcfe,#f8f7ff)",
          }}
        >
          <div style={{ color: palette.ink, fontSize: 34, fontWeight: 780 }}>
            Bonjour
          </div>
          <div style={{ marginTop: 9, color: palette.muted, fontSize: 24 }}>
            Sur quoi on travaille ?
          </div>
          <div style={{ marginTop: 14, color: palette.subtle, fontSize: 16 }}>
            Le ticket sélectionné fournit déjà le contexte utile.
          </div>

          <Interactive.Div
            name="Mobile AI actions"
            style={{
              marginTop: 32,
              display: "grid",
              gridTemplateColumns: "1fr 1fr",
              gap: 14,
            }}
          >
            <div
              style={{
                padding: "22px",
                borderRadius: 17,
                background: "white",
                border: "1px solid #e2e6ec",
                color: palette.ink,
                fontSize: 18,
                fontWeight: 700,
              }}
            >
              ↩ &nbsp; Rédiger
            </div>
            <div
              style={{
                position: "relative",
                padding: "22px",
                borderRadius: 17,
                background: "white",
                border: "2px solid rgba(109,92,231,.45)",
                color: "#6d5ce7",
                fontSize: 18,
                fontWeight: 780,
                boxShadow: "0 12px 30px rgba(109,92,231,.1)",
              }}
            >
              ▣ &nbsp; Résumer
              <ActionCursor
                name="Mobile summarize cursor"
                appearAt={48}
                clickAt={68}
                from={[120, -72]}
                to={[-4, -3]}
                relativeToParentCenter
                color="#6d5ce7"
              />
            </div>
          </Interactive.Div>

          <Interactive.Div
            name="Mobile generated summary"
            style={{
              marginTop: 28,
              padding: "28px",
              borderRadius: 23,
              background: "white",
              border: "1px solid rgba(109,92,231,.25)",
              boxShadow: "0 22px 52px rgba(62,52,145,.14)",
              opacity: interpolate(frame, [82, 96], [0, 1], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
              }),
              translate: interpolate(frame, [82, 98], ["0px 28px", "0px 0px"], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
                easing: Easing.bezier(0.16, 1, 0.3, 1),
              }),
            }}
          >
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                color: "#6d5ce7",
                fontSize: 18,
                fontWeight: 800,
              }}
            >
              <span>✦ Résumé généré</span>
              <span style={{ color: "#20a873" }}>Prêt ✓</span>
            </div>
            <div
              style={{
                marginTop: 20,
                color: palette.muted,
                fontSize: 21,
                lineHeight: 1.5,
              }}
            >
              Accès temporaire demandé pour Atelier Nord. Priorité normale, mise
              en ligne imminente.
            </div>
            <div
              style={{
                marginTop: 22,
                padding: "15px 18px",
                borderRadius: 14,
                color: "#3152bd",
                background: "#f0f3ff",
                fontSize: 18,
                fontWeight: 760,
              }}
            >
              Utiliser dans la réponse →
            </div>
          </Interactive.Div>

          <Interactive.Div
            name="Mobile context state"
            style={{
              marginTop: 24,
              display: "flex",
              gap: 10,
              flexWrap: "wrap",
              opacity: interpolate(frame, [96, 112], [0, 1], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
              }),
            }}
          >
            {["Ticket sélectionné", "Résumé prêt", "Réponse proposée"].map(
              (item) => (
                <span
                  key={item}
                  style={{
                    padding: "10px 13px",
                    borderRadius: 999,
                    color: "#6d5ce7",
                    background: "#f1efff",
                    fontSize: 15,
                    fontWeight: 720,
                  }}
                >
                  ✦ {item}
                </span>
              ),
            )}
          </Interactive.Div>
        </div>
      </Interactive.Div>
    </AbsoluteFill>
  );
};
