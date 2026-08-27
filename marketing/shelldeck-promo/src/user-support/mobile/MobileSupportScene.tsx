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

export const MobileSupportScene: React.FC = () => {
  const frame = useCurrentFrame();
  const reply = "Je vérifie les accès et le service. Je reviens vers vous ici.";
  const typedReply = reply.slice(
    0,
    Math.floor(
      interpolate(frame, [72, 125], [0, reply.length], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
      }),
    ),
  );

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <JourneyBackdrop tone="support" />
      <MobileHeader role="Support" />
      <Interactive.Div
        name="Mobile support copy"
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
          name="Mobile support headline"
          style={{
            margin: 0,
            color: palette.ink,
            fontSize: 78,
            lineHeight: 1,
            fontWeight: 790,
            letterSpacing: -3.7,
          }}
        >
          Le Support reprend.
          <br />
          <span style={{ color: "#6d5ce7" }}>Sans perdre le contexte.</span>
        </Interactive.H2>
      </Interactive.Div>

      <Interactive.Div
        name="Mobile support workspace"
        className="us-focus-frame"
        style={{
          position: "absolute",
          left: 80,
          right: 80,
          top: 430,
          height: 800,
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
        <MobileWindowChrome title="Support · Atelier Nord" />
        <div
          style={{
            flex: 1,
            padding: "26px",
            background: "linear-gradient(160deg,#fbfcfe,#f7f6ff)",
          }}
        >
          <Interactive.Div
            name="Mobile support ticket"
            style={{
              padding: "24px",
              borderRadius: 21,
              background: "white",
              border: "1px solid rgba(109,92,231,.26)",
              boxShadow: "0 14px 38px rgba(64,50,138,.08)",
            }}
          >
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
              }}
            >
              <MobilePill color="#6d5ce7">● À traiter</MobilePill>
              <span style={{ color: palette.muted, fontSize: 15 }}>
                il y a 1 min
              </span>
            </div>
            <div
              style={{
                marginTop: 20,
                color: palette.ink,
                fontSize: 29,
                fontWeight: 770,
              }}
            >
              Accès au serveur de préproduction
            </div>
            <div style={{ marginTop: 13, color: palette.muted, fontSize: 18 }}>
              Camille · Atelier Nord
            </div>
          </Interactive.Div>

          <div
            style={{
              marginTop: 22,
              padding: "22px 24px",
              borderRadius: "20px 20px 20px 6px",
              background: "#edf6ff",
              color: palette.ink,
              fontSize: 20,
              lineHeight: 1.45,
            }}
          >
            Bonjour, pouvez-vous préparer un accès temporaire avant la mise en
            ligne ?
          </div>
          <div
            style={{
              margin: "18px 0 0 110px",
              padding: "22px 24px",
              borderRadius: "20px 20px 6px 20px",
              background: "#f1efff",
              color: palette.ink,
              fontSize: 20,
              lineHeight: 1.45,
              opacity: interpolate(frame, [52, 68], [0, 1], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
              }),
            }}
          >
            {typedReply || "Réponse…"}
            {frame >= 72 && frame < 127 ? (
              <span style={{ color: "#6d5ce7" }}>│</span>
            ) : null}
          </div>

          <div
            style={{
              position: "absolute",
              left: 26,
              right: 26,
              bottom: 28,
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              padding: "18px 20px",
              borderRadius: 18,
              background: "white",
              border: "1px solid #dce3ec",
            }}
          >
            <span
              style={{
                color: typedReply ? palette.ink : "#9aa5b4",
                fontSize: 18,
              }}
            >
              {typedReply ? "Réponse prête" : "Écrire une réponse…"}
            </span>
            <div style={{ position: "relative" }}>
              <Interactive.Div
                name="Mobile send reply button"
                style={{
                  padding: "13px 22px",
                  borderRadius: 13,
                  color: "white",
                  background: frame >= 131 ? "#20a873" : "#6d5ce7",
                  fontSize: 17,
                  fontWeight: 780,
                  scale: interpolate(frame, [124, 128, 132], [1, 0.88, 1], {
                    extrapolateLeft: "clamp",
                    extrapolateRight: "clamp",
                    output: "perceptual-scale",
                  }),
                }}
              >
                {frame >= 131 ? "Envoyé ✓" : "Envoyer ↑"}
              </Interactive.Div>
              <ActionCursor
                name="Mobile reply cursor"
                appearAt={111}
                clickAt={128}
                from={[78, -74]}
                to={[-4, -3]}
                relativeToParentCenter
                color="#6d5ce7"
              />
            </div>
          </div>
        </div>
      </Interactive.Div>
    </AbsoluteFill>
  );
};
