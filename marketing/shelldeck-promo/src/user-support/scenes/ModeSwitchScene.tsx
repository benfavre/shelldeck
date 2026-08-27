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

export const ModeSwitchScene: React.FC = () => {
  const frame = useCurrentFrame();
  const supportActive = frame >= 52;

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <JourneyBackdrop tone={supportActive ? "support" : "user"} />
      <Interactive.Div name="Mode switch brand" style={{ position: "absolute", left: 104, top: 70 }}>
        <Brand compact />
      </Interactive.Div>
      <Interactive.Div
        name="Mode switch headline"
        style={{
          position: "absolute",
          left: 260,
          right: 260,
          top: 150,
          textAlign: "center",
          opacity: interpolate(frame, [0, 18], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
          translate: interpolate(frame, [0, 18], ["0px 22px", "0px 0px"], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.bezier(0.16, 1, 0.3, 1) }),
        }}
      >
        <div style={{ color: supportActive ? "#6d5ce7" : "#168ee0", fontSize: 20, fontWeight: 780, letterSpacing: 3.1, textTransform: "uppercase" }}>Changer de perspective</div>
        <h2 style={{ margin: "16px 0 0", color: palette.ink, fontSize: 72, lineHeight: 1, fontWeight: 790, letterSpacing: -3.8 }}>
          La demande reste. <span style={{ color: supportActive ? "#6d5ce7" : "#168ee0" }}>Le mode change.</span>
        </h2>
      </Interactive.Div>

      <Interactive.Div
        name="ShellDeck mode switcher"
        className="us-glass"
        style={{
          position: "absolute",
          left: 310,
          right: 310,
          bottom: 116,
          height: 590,
          borderRadius: 38,
          overflow: "hidden",
          opacity: interpolate(frame, [10, 28], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
          scale: interpolate(frame, [10, 30], [0.96, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.spring({ damping: 180 }), output: "perceptual-scale" }),
        }}
      >
        <div style={{ height: 76, display: "flex", alignItems: "center", justifyContent: "space-between", padding: "0 30px", borderBottom: "1px solid #dfe5ed", background: "rgba(255,255,255,.9)" }}>
          <span style={{ color: palette.ink, fontSize: 20, fontWeight: 800 }}>ShellDeck</span>
          <Interactive.Div name="Mode segmented control" style={{ position: "relative", display: "flex", padding: 5, borderRadius: 15, background: "#edf1f5" }}>
            <Interactive.Div
              name="Mode active indicator"
              style={{
                position: "absolute",
                top: 5,
                left: 5,
                width: 156,
                height: 42,
                borderRadius: 11,
                background: supportActive ? "linear-gradient(120deg, #755ee9, #5e49d8)" : "linear-gradient(120deg, #168ee0, #2e72dc)",
                boxShadow: supportActive ? "0 8px 20px rgba(109,92,231,.24)" : "0 8px 20px rgba(22,142,224,.22)",
                translate: interpolate(frame, [47, 60], ["0px 0px", "156px 0px"], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.spring({ damping: 190 }) }),
              }}
            />
            {[
              { label: "Utilisateur", active: !supportActive },
              { label: "Support", active: supportActive },
            ].map((mode) => (
              <div key={mode.label} style={{ zIndex: 2, width: 156, padding: "11px 0", textAlign: "center", color: mode.active ? "white" : palette.muted, fontSize: 17, fontWeight: 780 }}>{mode.label}</div>
            ))}
          </Interactive.Div>
          <div style={{ width: 92, color: palette.muted, fontSize: 15, textAlign: "right" }}>Atelier Nord</div>
        </div>

        <Interactive.Div
          name="User mode preview"
          style={{
            position: "absolute",
            inset: "76px 0 0",
            padding: "42px 48px",
            opacity: interpolate(frame, [44, 58], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
            translate: interpolate(frame, [44, 58], ["0px 0px", "-34px 0px"], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
          }}
        >
          <div style={{ color: palette.ink, fontSize: 31, fontWeight: 780 }}>Bonjour, Camille</div>
          <div style={{ marginTop: 10, color: palette.muted, fontSize: 19 }}>Suivez vos sites et vos demandes au même endroit.</div>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 22, marginTop: 34 }}>
            {[{ value: "3", label: "Sites disponibles" }, { value: "1", label: "Demande en cours" }].map((item) => (
              <div key={item.label} style={{ padding: "28px 30px", borderRadius: 22, background: "white", border: "1px solid #e0e6ee", boxShadow: "0 13px 34px rgba(38,63,92,.07)" }}>
                <div style={{ color: "#168ee0", fontSize: 38, fontWeight: 800 }}>{item.value}</div>
                <div style={{ marginTop: 8, color: palette.muted, fontSize: 17 }}>{item.label}</div>
              </div>
            ))}
          </div>
          <div style={{ marginTop: 24, padding: "23px 28px", borderRadius: 20, color: palette.ink, background: "#edf7ff", fontSize: 20, fontWeight: 720 }}>✓ Demande #248 envoyée au support</div>
        </Interactive.Div>

        <Interactive.Div
          name="Support mode preview"
          style={{
            position: "absolute",
            inset: "76px 0 0",
            padding: "42px 48px",
            opacity: interpolate(frame, [54, 68], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
            translate: interpolate(frame, [54, 68], ["34px 0px", "0px 0px"], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.bezier(0.16, 1, 0.3, 1) }),
          }}
        >
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <div>
              <div style={{ color: palette.ink, fontSize: 31, fontWeight: 780 }}>Bonjour, l’équipe Support</div>
              <div style={{ marginTop: 10, color: palette.muted, fontSize: 19 }}>Une nouvelle demande requiert votre attention.</div>
            </div>
            <span style={{ padding: "10px 15px", borderRadius: 999, color: "#6d5ce7", background: "#eeeafe", fontSize: 16, fontWeight: 800 }}>1 nouvelle</span>
          </div>
          <div style={{ marginTop: 34, padding: "29px 30px", borderRadius: 24, background: "white", border: "2px solid rgba(109,92,231,.28)", boxShadow: "0 18px 48px rgba(109,92,231,.12)" }}>
            <div style={{ display: "flex", justifyContent: "space-between" }}>
              <span style={{ color: palette.ink, fontSize: 24, fontWeight: 780 }}>Accès au serveur de préproduction</span>
              <span style={{ color: "#6d5ce7", fontSize: 16, fontWeight: 760 }}>À traiter</span>
            </div>
            <div style={{ marginTop: 12, color: palette.muted, fontSize: 17 }}>Atelier Nord · Camille · maintenant</div>
            <div style={{ marginTop: 22, color: palette.ink, fontSize: 19 }}>Préparer un accès temporaire pour la mise en ligne.</div>
          </div>
        </Interactive.Div>
        <ActionCursor name="Switch to Support cursor" appearAt={25} clickAt={51} from={[850, 24]} to={[730, 30]} color="#6d5ce7" />
        <Interactive.Div
          name="Mode click ripple"
          style={{
            position: "absolute",
            left: 714,
            top: 25,
            width: 32,
            height: 32,
            borderRadius: 999,
            border: "3px solid #6d5ce7",
            opacity: interpolate(frame, [50, 53, 64], [0, 0.75, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
            scale: interpolate(frame, [50, 64], [0.45, 2.4], { extrapolateLeft: "clamp", extrapolateRight: "clamp", output: "perceptual-scale" }),
          }}
        />
      </Interactive.Div>
    </AbsoluteFill>
  );
};
