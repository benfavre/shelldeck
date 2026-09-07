import {
  AbsoluteFill,
  Easing,
  Interactive,
  interpolate,
  useCurrentFrame,
} from "remotion";
import { Brand } from "../../components/Brand";
import { palette } from "../../theme";
import { ActionCursor } from "../ActionCursor";
import { JourneyBackdrop } from "../JourneyBackdrop";

export const MobileModeSwitchScene: React.FC = () => {
  const frame = useCurrentFrame();
  const supportActive = frame >= 51;

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <JourneyBackdrop tone="blend" />
      <Interactive.Div
        name="Mobile switch brand"
        style={{ position: "absolute", left: 80, top: 62 }}
      >
        <Brand compact />
      </Interactive.Div>

      <Interactive.Div
        name="Mobile mode switch copy"
        style={{
          position: "absolute",
          left: 80,
          right: 80,
          top: 190,
          textAlign: "center",
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
        <div
          style={{
            color: "#6d5ce7",
            fontSize: 18,
            fontWeight: 800,
            letterSpacing: 3,
            textTransform: "uppercase",
          }}
        >
          Changer de perspective
        </div>
        <Interactive.H2
          name="Mobile mode switch headline"
          style={{
            margin: "20px 0 0",
            color: palette.ink,
            fontSize: 80,
            lineHeight: 1,
            fontWeight: 790,
            letterSpacing: -3.8,
          }}
        >
          La demande reste.
          <br />
          <span style={{ color: "#6d5ce7" }}>Le mode change.</span>
        </Interactive.H2>
      </Interactive.Div>

      <Interactive.Div
        name="Mobile mode surface"
        className="us-glass"
        style={{
          position: "absolute",
          left: 80,
          right: 80,
          top: 515,
          height: 650,
          padding: "34px",
          borderRadius: 34,
          opacity: interpolate(frame, [12, 34], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          }),
          translate: interpolate(frame, [12, 36], ["0px 48px", "0px 0px"], {
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
            alignItems: "center",
          }}
        >
          <span style={{ color: palette.ink, fontSize: 24, fontWeight: 780 }}>
            ShellDeck
          </span>
          <span style={{ color: palette.muted, fontSize: 16 }}>
            Atelier Nord
          </span>
        </div>

        <Interactive.Div
          name="Mobile mode selector"
          style={{
            position: "relative",
            width: 510,
            height: 66,
            margin: "42px auto 0",
            display: "grid",
            gridTemplateColumns: "1fr 1fr",
            padding: 6,
            borderRadius: 18,
            background: "#eef2f6",
            color: palette.muted,
            fontSize: 19,
            fontWeight: 760,
          }}
        >
          <Interactive.Div
            name="Mobile selected mode"
            style={{
              position: "absolute",
              left: 6,
              top: 6,
              width: 249,
              height: 54,
              borderRadius: 14,
              background: supportActive
                ? "linear-gradient(110deg,#6d5ce7,#855ff0)"
                : "linear-gradient(110deg,#168ee0,#2fa8df)",
              boxShadow: supportActive
                ? "0 10px 24px rgba(109,92,231,.28)"
                : "0 10px 24px rgba(22,142,224,.24)",
              translate: interpolate(
                frame,
                [36, 52],
                ["0px 0px", "249px 0px"],
                {
                  extrapolateLeft: "clamp",
                  extrapolateRight: "clamp",
                  easing: Easing.bezier(0.16, 1, 0.3, 1),
                },
              ),
            }}
          />
          <div
            style={{
              zIndex: 2,
              display: "grid",
              placeItems: "center",
              color: supportActive ? palette.muted : "white",
            }}
          >
            Utilisateur
          </div>
          <div
            style={{
              zIndex: 2,
              display: "grid",
              placeItems: "center",
              color: supportActive ? "white" : palette.muted,
            }}
          >
            Support
          </div>
          <ActionCursor
            name="Mobile switch cursor"
            appearAt={24}
            clickAt={51}
            from={[330, -55]}
            to={[370, 29]}
            color="#6d5ce7"
          />
        </Interactive.Div>

        <Interactive.Div
          name="Mobile switched request"
          style={{
            marginTop: 58,
            padding: "30px",
            borderRadius: 24,
            background: "white",
            border: `2px solid ${supportActive ? "rgba(109,92,231,.3)" : "rgba(22,142,224,.26)"}`,
            boxShadow: supportActive
              ? "0 22px 52px rgba(109,92,231,.13)"
              : "0 22px 52px rgba(22,142,224,.1)",
            opacity: interpolate(frame, [18, 34], [0, 1], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
            }),
          }}
        >
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              color: supportActive ? "#6d5ce7" : "#168ee0",
              fontSize: 16,
              fontWeight: 800,
            }}
          >
            <span>DEMANDE #248</span>
            <span>{supportActive ? "À traiter" : "Envoyée"}</span>
          </div>
          <div
            style={{
              marginTop: 20,
              color: palette.ink,
              fontSize: 31,
              lineHeight: 1.15,
              fontWeight: 770,
            }}
          >
            Accès au serveur de préproduction
          </div>
          <div style={{ marginTop: 14, color: palette.muted, fontSize: 19 }}>
            Atelier Nord · Camille · maintenant
          </div>
          <div
            style={{
              marginTop: 26,
              paddingTop: 22,
              borderTop: "1px solid #e3e8ef",
              color: palette.muted,
              fontSize: 20,
            }}
          >
            Le même contexte arrive côté Support.
          </div>
        </Interactive.Div>
      </Interactive.Div>
    </AbsoluteFill>
  );
};
