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

const requests = [
  { title: "Accès à la préproduction", meta: "Atelier Nord · maintenant", active: true },
  { title: "Certificat à renouveler", meta: "Studio Cobalt · 12 min", active: false },
  { title: "Domaine à connecter", meta: "Maison Lune · 24 min", active: false },
];

export const SupportTriageScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const reply = "Accès préparé. Je vous envoie les identifiants sécurisés.";
  const typedReply = reply.slice(
    0,
    Math.floor(
      interpolate(frame, [70, 116], [0, reply.length], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
      }),
    ),
  );

  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <JourneyBackdrop tone="support" />
      <Interactive.Div name="Support scene brand" style={{ position: "absolute", left: 104, top: 70 }}>
        <Brand compact />
      </Interactive.Div>
      <Interactive.Div name="Support role" style={{ position: "absolute", right: 104, top: 74 }}>
        <RoleBadge role="Support" />
      </Interactive.Div>

      <Interactive.Div
        name="Support triage copy"
        style={{
          position: "absolute",
          left: 104,
          top: 170,
          zIndex: 4,
          opacity: interpolate(frame, [0, 0.65 * fps], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
          translate: interpolate(frame, [0, 0.65 * fps], ["0px 28px", "0px 0px"], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.bezier(0.16, 1, 0.3, 1) }),
        }}
      >
        <Interactive.H2 name="Support triage headline" style={{ margin: 0, color: palette.ink, fontSize: 78, lineHeight: 1.01, fontWeight: 790, letterSpacing: -4 }}>
          Trier moins. <span style={{ color: "#6d5ce7" }}>Résoudre plus vite.</span>
        </Interactive.H2>
        <p style={{ margin: "20px 0 0", color: palette.muted, fontSize: 27 }}>
          Chaque demande arrive avec son site, sa priorité et son historique.
        </p>
      </Interactive.Div>

      <Interactive.Div
        name="Illustrated request queue"
        className="us-glass"
        style={{
          position: "absolute",
          left: 104,
          bottom: 92,
          width: 520,
          height: 545,
          padding: 24,
          borderRadius: 30,
          opacity: interpolate(frame, [0.45 * fps, 1.15 * fps], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
          translate: interpolate(frame, [0.45 * fps, 1.15 * fps], ["-42px 28px", "0px 0px"], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.bezier(0.16, 1, 0.3, 1) }),
        }}
      >
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", color: palette.ink }}>
          <span style={{ fontSize: 25, fontWeight: 780 }}>Demandes</span>
          <span style={{ padding: "7px 12px", borderRadius: 999, color: "#6d5ce7", background: "#eeeafe", fontSize: 15, fontWeight: 800 }}>{frame < 28 ? "2" : "3"} à traiter</span>
        </div>
        <div style={{ marginTop: 20, padding: "13px 16px", border: "1px solid #dce3ec", borderRadius: 14, color: "#95a1b1", background: "rgba(255,255,255,.72)", fontSize: 16 }}>
          Rechercher une demande…
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 12, marginTop: 18 }}>
          {requests.map((request, index) => (
            <Interactive.Div
              key={request.title}
              name={`Queue item ${index + 1}`}
              style={{
                padding: "18px 18px 17px",
                borderRadius: 17,
                border: request.active ? "1px solid rgba(109,92,231,.34)" : "1px solid rgba(137,155,176,.2)",
                background: request.active ? "linear-gradient(135deg, #f0edff, #f7faff)" : "rgba(255,255,255,.62)",
                boxShadow: request.active ? "0 14px 34px rgba(109,92,231,.12)" : "none",
                opacity: interpolate(frame, [index === 0 ? 12 : (0.9 + index * 0.18) * fps, index === 0 ? 30 : (1.35 + index * 0.18) * fps], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
                translate: interpolate(frame, [index === 0 ? 12 : (0.9 + index * 0.18) * fps, index === 0 ? 30 : (1.35 + index * 0.18) * fps], [index === 0 ? "0px -28px" : "-22px 0px", "0px 0px"], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.bezier(0.16, 1, 0.3, 1) }),
              }}
            >
              <div style={{ color: palette.ink, fontSize: 19, fontWeight: 730 }}>{request.title}</div>
              <div style={{ display: "flex", alignItems: "center", gap: 9, marginTop: 9, color: palette.muted, fontSize: 14 }}>
                <span style={{ width: 8, height: 8, borderRadius: 99, backgroundColor: request.active ? "#6d5ce7" : "#a9b4c1" }} />
                {request.meta}
              </div>
            </Interactive.Div>
          ))}
        </div>
      </Interactive.Div>

      <Interactive.Div
        name="Illustrated request detail"
        className="us-glass"
        style={{
          position: "absolute",
          right: 104,
          bottom: 92,
          width: 1080,
          height: 545,
          padding: "34px 38px",
          borderRadius: 30,
          opacity: interpolate(frame, [0.7 * fps, 1.4 * fps], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
          translate: interpolate(frame, [0.7 * fps, 1.4 * fps], ["48px 28px", "0px 0px"], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.bezier(0.16, 1, 0.3, 1) }),
        }}
      >
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start" }}>
          <div>
            <div style={{ color: palette.ink, fontSize: 31, fontWeight: 780 }}>Accès au serveur de préproduction</div>
            <div style={{ display: "flex", gap: 10, marginTop: 16 }}>
              {["À traiter", "Normale", "Atelier Nord"].map((item, index) => (
                <span key={item} style={{ padding: "8px 12px", borderRadius: 999, color: index === 0 ? "#6d5ce7" : palette.muted, background: index === 0 ? "#eeeafe" : "#edf1f5", fontSize: 15, fontWeight: 700 }}>{index === 0 && frame >= 54 ? "En cours" : item}</span>
              ))}
            </div>
          </div>
          <span style={{ color: "#6d5ce7", fontSize: 18, fontWeight: 750 }}>✦ Résumer</span>
        </div>
        <div style={{ marginTop: 48, padding: "22px 24px", borderRadius: 18, background: "rgba(255,255,255,.7)", border: "1px solid rgba(137,155,176,.18)" }}>
          <div style={{ color: palette.muted, fontSize: 15, fontWeight: 700 }}>Camille Bernard · il y a 1 h</div>
          <div style={{ marginTop: 11, color: palette.ink, fontSize: 20 }}>Préparer un accès temporaire pour la mise en ligne.</div>
        </div>
        <Interactive.Div
          name="Reply composer"
          style={{
            position: "absolute",
            left: 38,
            right: 38,
            bottom: 34,
            height: 116,
            padding: "19px 22px",
            borderRadius: 20,
            border: "2px solid rgba(109,92,231,.34)",
            background: "rgba(255,255,255,.88)",
            boxShadow: "0 16px 38px rgba(109,92,231,.1)",
            opacity: interpolate(frame, [1.65 * fps, 2.2 * fps], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
          }}
        >
          <div style={{ minHeight: 24, color: typedReply && frame < 131 ? palette.ink : "#8b97a6", fontSize: 18 }}>
            {frame >= 131 ? "Votre réponse…" : typedReply || "Répondre à Studio Cobalt…"}
            {frame >= 70 && frame < 118 ? <span style={{ color: "#6d5ce7" }}>│</span> : null}
          </div>
          <div style={{ display: "flex", justifyContent: "space-between", marginTop: 22, color: "#6d5ce7", fontSize: 15, fontWeight: 750 }}>
            <span>＋ &nbsp; ✦ Proposer une réponse</span>
            <Interactive.Span
              name="Send support reply"
              style={{
                color: frame >= 131 ? "#20a873" : "#6d5ce7",
                scale: interpolate(frame, [125, 128, 132], [1, 0.82, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.spring({ damping: 180 }), output: "perceptual-scale" }),
              }}
            >
              {frame >= 131 ? "Envoyé ✓" : "Envoyer ↑"}
            </Interactive.Span>
          </div>
        </Interactive.Div>
        <Interactive.Div
          name="Sent support message"
          style={{
            position: "absolute",
            right: 38,
            top: 270,
            maxWidth: 720,
            padding: "16px 20px",
            borderRadius: "18px 18px 5px 18px",
            color: "#31504a",
            background: "linear-gradient(135deg, #e9faf4, #f3fffb)",
            border: "1px solid rgba(32,168,115,.22)",
            boxShadow: "0 14px 32px rgba(32,168,115,.1)",
            fontSize: 17,
            lineHeight: 1.35,
            opacity: interpolate(frame, [130, 139], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
            translate: interpolate(frame, [130, 141], ["28px 14px", "0px 0px"], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.spring({ damping: 180 }) }),
          }}
        >
          {reply}
        </Interactive.Div>
      </Interactive.Div>
      <ActionCursor name="Send reply cursor" appearAt={111} clickAt={128} from={[1820, 845]} to={[1710, 900]} color="#6d5ce7" />
    </AbsoluteFill>
  );
};
