export interface LandingDownloadInfo {
  version: string;
  linux?: { url: string; size: number };
  macos?: { url: string; size: number };
  windows?: { url: string; size: number };
}

const GITHUB = "https://github.com/benfavre/shelldeck";
const GITHUB_RELEASES = `${GITHUB}/releases`;

function formatSize(bytes: number): string {
  if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

export function renderMarketingLandingPage(dl: LandingDownloadInfo): Response {
  const v = dl.version;
  const linuxUrl = dl.linux?.url ?? `${GITHUB_RELEASES}/latest`;
  const macosUrl = dl.macos?.url ?? `${GITHUB_RELEASES}/latest`;
  const windowsUrl = dl.windows?.url ?? `${GITHUB_RELEASES}/latest`;
  const linuxMeta = dl.linux ? ` · ${formatSize(dl.linux.size)}` : "";
  const macosMeta = dl.macos ? ` · ${formatSize(dl.macos.size)}` : "";
  const windowsMeta = dl.windows ? ` · ${formatSize(dl.windows.size)}` : "";

  const html = `<!doctype html>
<html lang="fr">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="theme-color" content="#0d1110">
  <link rel="icon" href="/favicon.svg" type="image/svg+xml">
  <title>ShellDeck — Votre infrastructure, enfin réunie</title>
  <meta name="description" content="ShellDeck réunit SSH, terminaux, scripts et tunnels dans une application desktop native, rapide et open source.">
  <meta property="og:title" content="ShellDeck — Connectez. Pilotez. Respirez.">
  <meta property="og:description" content="Votre quotidien d’exploitation, réuni dans une seule application desktop native.">
  <meta property="og:image" content="https://shelldeck.1clic.pro/campaign/control-v2.webp">
  <meta property="og:type" content="website">
  <style>
    :root {
      color-scheme: dark;
      --ink: #0d1110;
      --ink-2: #111715;
      --surface: #171d1b;
      --surface-2: #1c2421;
      --line: rgba(230, 230, 230, .11);
      --line-strong: rgba(46, 196, 168, .28);
      --mint: #2ec4a8;
      --mint-soft: #67dfc7;
      --blue: #69a7ff;
      --white: #f4f7f6;
      --soft-white: #e6e6e6;
      --muted: #9ca9a5;
      --quiet: #6e7c78;
      --gradient: linear-gradient(110deg, var(--mint), var(--blue));
      --shadow: 0 32px 90px rgba(0, 0, 0, .45);
      --radius: 22px;
    }

    * { box-sizing: border-box; }
    html { scroll-behavior: smooth; }
    body {
      margin: 0;
      color: var(--soft-white);
      background:
        radial-gradient(circle at 68% 12%, rgba(46, 196, 168, .08), transparent 30%),
        radial-gradient(circle at 16% 42%, rgba(105, 167, 255, .055), transparent 26%),
        var(--ink);
      font-family: Inter, ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      line-height: 1.5;
      -webkit-font-smoothing: antialiased;
    }
    body::before {
      content: "";
      position: fixed;
      inset: 0;
      z-index: -1;
      pointer-events: none;
      opacity: .16;
      background-image:
        linear-gradient(rgba(255,255,255,.025) 1px, transparent 1px),
        linear-gradient(90deg, rgba(255,255,255,.025) 1px, transparent 1px);
      background-size: 72px 72px;
      mask-image: linear-gradient(to bottom, black, transparent 80%);
    }
    a { color: inherit; text-decoration: none; }
    button { font: inherit; }
    svg { display: block; }
    .shell { width: min(1180px, calc(100% - 40px)); margin-inline: auto; }
    .gradient-text {
      color: var(--mint);
      background: var(--gradient);
      -webkit-background-clip: text;
      background-clip: text;
      -webkit-text-fill-color: transparent;
    }

    .nav-wrap {
      position: sticky;
      top: 0;
      z-index: 20;
      border-bottom: 1px solid rgba(255,255,255,.06);
      background: rgba(13, 17, 16, .76);
      backdrop-filter: blur(18px);
    }
    nav {
      height: 72px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 24px;
    }
    .brand { display: flex; align-items: center; gap: 11px; font-size: 20px; font-weight: 800; letter-spacing: -.04em; }
    .brand-mark { width: 34px; height: 34px; filter: drop-shadow(0 0 18px rgba(46,196,168,.2)); }
    .wordmark { color: var(--white); }
    .wordmark b { color: var(--mint); font-weight: inherit; }
    .nav-links { display: flex; align-items: center; gap: 28px; color: var(--muted); font-size: 14px; font-weight: 650; }
    .nav-links a:hover { color: var(--white); }
    .nav-cta {
      display: inline-flex;
      align-items: center;
      gap: 8px;
      padding: 10px 15px;
      border: 1px solid var(--line-strong);
      border-radius: 12px;
      color: var(--white);
      background: rgba(46,196,168,.08);
    }
    .nav-cta:hover { border-color: var(--mint); background: rgba(46,196,168,.13); }

    .hero { padding: 76px 0 58px; overflow: hidden; }
    .hero-grid {
      display: grid;
      grid-template-columns: minmax(0, .91fr) minmax(480px, 1.09fr);
      align-items: center;
      gap: 52px;
    }
    .eyebrow {
      display: inline-flex;
      align-items: center;
      gap: 10px;
      margin-bottom: 22px;
      color: #b9c6c2;
      font-size: 12px;
      font-weight: 800;
      letter-spacing: .18em;
      text-transform: uppercase;
    }
    .eyebrow::before { content: ""; width: 28px; height: 1px; background: var(--mint); box-shadow: 0 0 12px var(--mint); }
    .hero h1 {
      max-width: 680px;
      margin: 0;
      color: var(--white);
      font-size: clamp(50px, 6.3vw, 82px);
      line-height: .98;
      letter-spacing: -.065em;
    }
    .hero-copy {
      max-width: 590px;
      margin: 27px 0 0;
      color: var(--muted);
      font-size: 18px;
      line-height: 1.68;
    }
    .hero-copy strong { color: var(--soft-white); font-weight: 650; }
    .actions { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; margin-top: 31px; }
    .button {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 9px;
      min-height: 48px;
      padding: 0 19px;
      border: 1px solid var(--line);
      border-radius: 13px;
      color: var(--white);
      font-size: 14px;
      font-weight: 760;
      transition: transform .18s ease, border-color .18s ease, background .18s ease;
    }
    .button:hover { transform: translateY(-2px); border-color: rgba(255,255,255,.25); }
    .button-primary { border-color: transparent; color: #07110f; background: var(--gradient); box-shadow: 0 14px 38px rgba(46,196,168,.19); }
    .button-secondary { background: rgba(255,255,255,.035); }
    .hero-note { margin-top: 17px; color: var(--quiet); font-size: 12px; }

    .hero-visual { position: relative; min-height: 550px; }
    .hero-visual::before {
      content: "";
      position: absolute;
      inset: 9% 2% 3% 8%;
      border-radius: 50%;
      background: rgba(46,196,168,.18);
      filter: blur(90px);
    }
    .campaign-frame {
      position: absolute;
      inset: 0;
      overflow: hidden;
      border: 1px solid rgba(255,255,255,.12);
      border-radius: 30px;
      background: #0a0e0d;
      box-shadow: var(--shadow);
      transform: perspective(1400px) rotateY(-4deg) rotateX(1deg);
    }
    .campaign-frame::after {
      content: "";
      position: absolute;
      inset: 0;
      pointer-events: none;
      background: linear-gradient(120deg, rgba(255,255,255,.09), transparent 24%, transparent 70%, rgba(46,196,168,.06));
    }
    .campaign-frame img { width: 100%; height: 100%; object-fit: cover; }
    .window-bar {
      position: absolute;
      z-index: 2;
      top: 16px;
      left: 18px;
      right: 18px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 10px 12px;
      border: 1px solid rgba(255,255,255,.09);
      border-radius: 12px;
      background: rgba(10,14,13,.68);
      backdrop-filter: blur(14px);
      color: #b4c0bc;
      font: 600 11px ui-monospace, SFMono-Regular, Menlo, monospace;
    }
    .dots { display: flex; gap: 6px; }
    .dots i { width: 7px; height: 7px; border-radius: 50%; background: #3b4945; }
    .dots i:first-child { background: var(--mint); box-shadow: 0 0 10px rgba(46,196,168,.5); }
    .floating-card {
      position: absolute;
      z-index: 3;
      right: -18px;
      bottom: 24px;
      width: 255px;
      padding: 16px;
      border: 1px solid rgba(255,255,255,.12);
      border-radius: 16px;
      background: rgba(18,24,22,.86);
      box-shadow: 0 22px 50px rgba(0,0,0,.38);
      backdrop-filter: blur(18px);
    }
    .floating-card-top { display: flex; align-items: center; justify-content: space-between; color: var(--white); font-size: 12px; font-weight: 760; }
    .live { display: flex; align-items: center; gap: 6px; color: var(--mint-soft); font-size: 10px; text-transform: uppercase; letter-spacing: .12em; }
    .live::before { content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--mint); box-shadow: 0 0 10px var(--mint); }
    .host-list { display: grid; gap: 8px; margin-top: 14px; }
    .host { display: grid; grid-template-columns: 8px 1fr auto; align-items: center; gap: 9px; color: var(--muted); font: 11px ui-monospace, SFMono-Regular, Menlo, monospace; }
    .host::before { content: ""; width: 6px; height: 6px; border-radius: 2px; background: var(--mint); }
    .host span:last-child { color: #71807b; }

    .proof { padding: 22px 0 78px; }
    .proof-row {
      display: grid;
      grid-template-columns: repeat(4, 1fr);
      border-block: 1px solid var(--line);
    }
    .proof-item { padding: 25px 20px; border-right: 1px solid var(--line); }
    .proof-item:last-child { border-right: 0; }
    .proof-value { color: var(--white); font-size: 15px; font-weight: 760; }
    .proof-label { margin-top: 4px; color: var(--quiet); font-size: 12px; }

    section { padding: 86px 0; }
    .section-head { display: grid; grid-template-columns: .85fr 1.15fr; gap: 60px; align-items: end; margin-bottom: 36px; }
    .section-kicker { color: var(--mint); font-size: 11px; font-weight: 850; letter-spacing: .2em; text-transform: uppercase; }
    .section-head h2 { margin: 11px 0 0; color: var(--white); font-size: clamp(36px, 5vw, 58px); line-height: 1.02; letter-spacing: -.055em; }
    .section-head p { max-width: 560px; margin: 0; color: var(--muted); font-size: 17px; line-height: 1.7; }

    .story-grid { display: grid; grid-template-columns: 1.1fr .9fr; grid-template-rows: repeat(2, 330px); gap: 16px; }
    .story-card {
      position: relative;
      overflow: hidden;
      min-height: 300px;
      border: 1px solid var(--line);
      border-radius: var(--radius);
      background: var(--surface);
    }
    .story-card:first-child { grid-row: 1 / 3; }
    .story-card img { width: 100%; height: 100%; object-fit: cover; transition: transform .5s ease; }
    .story-card:hover img { transform: scale(1.025); }
    .story-card::after { content: ""; position: absolute; inset: 0; background: linear-gradient(to top, rgba(7,12,10,.96), rgba(7,12,10,.08) 68%); }
    .story-content { position: absolute; z-index: 2; left: 28px; right: 28px; bottom: 27px; }
    .story-index { color: var(--mint); font: 700 11px ui-monospace, SFMono-Regular, Menlo, monospace; letter-spacing: .14em; }
    .story-card h3 { margin: 8px 0 7px; color: var(--white); font-size: clamp(24px, 3vw, 38px); line-height: 1.05; letter-spacing: -.04em; }
    .story-card:not(:first-child) h3 { font-size: 26px; }
    .story-card p { max-width: 440px; margin: 0; color: #b6c2be; font-size: 14px; }

    .capabilities { border-block: 1px solid var(--line); background: rgba(255,255,255,.012); }
    .cap-grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; }
    .cap {
      padding: 24px;
      border: 1px solid transparent;
      border-radius: 18px;
      transition: border-color .18s ease, background .18s ease;
    }
    .cap:hover { border-color: var(--line); background: rgba(255,255,255,.025); }
    .cap-icon {
      display: grid;
      place-items: center;
      width: 42px;
      height: 42px;
      margin-bottom: 17px;
      border: 1px solid var(--line-strong);
      border-radius: 12px;
      color: var(--mint);
      background: rgba(46,196,168,.07);
    }
    .cap h3 { margin: 0 0 8px; color: var(--white); font-size: 16px; }
    .cap p { margin: 0; color: var(--quiet); font-size: 13px; line-height: 1.65; }

    .ai-section { overflow: hidden; }
    .ai-panel {
      position: relative;
      overflow: hidden;
      display: grid;
      grid-template-columns: .82fr 1.18fr;
      min-height: 520px;
      border: 1px solid rgba(46,196,168,.22);
      border-radius: 28px;
      background:
        radial-gradient(circle at 8% 14%, rgba(46,196,168,.1), transparent 34%),
        linear-gradient(145deg, #141b19, #0e1311);
      box-shadow: var(--shadow);
    }
    .ai-copy { position: relative; z-index: 2; padding: clamp(34px, 5vw, 64px); align-self: center; }
    .ai-copy h2 { margin: 11px 0 0; color: var(--white); font-size: clamp(40px, 5vw, 60px); line-height: 1.01; letter-spacing: -.055em; }
    .ai-copy > p { margin: 22px 0 0; color: var(--muted); font-size: 16px; line-height: 1.72; }
    .ai-copy > p strong { color: var(--soft-white); }
    .ai-contexts { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 25px; }
    .ai-contexts span { padding: 7px 10px; border: 1px solid var(--line); border-radius: 999px; color: #b9c7c2; background: rgba(255,255,255,.025); font-size: 10px; font-weight: 720; }
    .ai-safety { display: flex; align-items: center; gap: 9px; margin-top: 19px; color: var(--mint-soft); font-size: 11px; font-weight: 700; }
    .ai-safety svg { flex: 0 0 auto; }
    .ai-visual { position: relative; min-width: 0; min-height: 520px; border-left: 1px solid var(--line); }
    .ai-visual > img { width: 100%; height: 100%; object-fit: cover; object-position: center; }
    .ai-visual::after { content: ""; position: absolute; inset: 0; background: linear-gradient(90deg, #111815 0, transparent 28%), linear-gradient(0deg, rgba(6,10,9,.7), transparent 36%); pointer-events: none; }
    .ai-review-badge { position: absolute; z-index: 2; top: 24px; right: 24px; display: flex; align-items: center; gap: 8px; padding: 9px 11px; border: 1px solid rgba(46,196,168,.24); border-radius: 10px; color: var(--mint-soft); background: rgba(8,13,11,.78); backdrop-filter: blur(12px); font-size: 9px; font-weight: 820; letter-spacing: .12em; text-transform: uppercase; }
    .ai-review-badge::before { content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--mint); box-shadow: 0 0 10px var(--mint); }
    .ai-sequence { position: absolute; z-index: 2; left: 24px; right: 24px; bottom: 22px; display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px; }
    .ai-sequence span { padding: 10px; border: 1px solid var(--line); border-radius: 10px; color: #afbbb7; background: rgba(7,11,10,.78); backdrop-filter: blur(12px); font-size: 9px; text-align: center; }
    .ai-sequence b { display: block; margin-bottom: 3px; color: var(--white); font-size: 10px; }

    .inklura-section { overflow: hidden; }
    .inklura-panel {
      position: relative;
      overflow: hidden;
      display: grid;
      grid-template-columns: .9fr 1.1fr;
      gap: clamp(36px, 6vw, 78px);
      align-items: center;
      padding: clamp(30px, 5vw, 62px);
      border: 1px solid rgba(105,167,255,.2);
      border-radius: 28px;
      background:
        radial-gradient(circle at 4% 12%, rgba(105,167,255,.1), transparent 29%),
        radial-gradient(circle at 96% 88%, rgba(46,196,168,.09), transparent 32%),
        linear-gradient(145deg, #141b19, #0f1412);
      box-shadow: var(--shadow);
    }
    .inklura-panel::before {
      content: "";
      position: absolute;
      inset: 0;
      pointer-events: none;
      opacity: .32;
      background-image: radial-gradient(rgba(105,167,255,.3) 1px, transparent 1px);
      background-size: 24px 24px;
      mask-image: linear-gradient(115deg, black, transparent 36%);
    }
    .inklura-panel::after {
      content: "";
      position: absolute;
      z-index: 0;
      width: 300px;
      height: 430px;
      right: -54px;
      top: -104px;
      opacity: .1;
      background: center / contain no-repeat url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 138 178'%3E%3Crect x='30' y='30' width='78' height='118' rx='24' fill='%23146BFF'/%3E%3Ccircle cx='69' cy='60' r='8' fill='white'/%3E%3Crect x='61' y='72' width='16' height='40' rx='7' fill='white'/%3E%3Cpath d='M46 111C52 126 86 126 92 111' fill='none' stroke='white' stroke-width='7' stroke-linecap='round'/%3E%3C/svg%3E");
      transform: rotate(8deg);
      pointer-events: none;
    }
    .inklura-copy { position: relative; z-index: 1; }
    .inklura-lockup { display: flex; align-items: center; gap: 12px; margin-bottom: 22px; }
    .inklura-logo { width: 128px; height: auto; margin: -15px 0; }
    .inklura-audience { padding-left: 12px; border-left: 1px solid rgba(255,255,255,.16); color: #aebbb7; font-size: 10px; font-weight: 820; letter-spacing: .14em; text-transform: uppercase; }
    .inklura-copy h2 { margin: 0; color: var(--white); font-size: clamp(38px, 5vw, 58px); line-height: 1.02; letter-spacing: -.055em; }
    .inklura-copy > p { margin: 22px 0 0; color: var(--muted); font-size: 16px; line-height: 1.72; }
    .inklura-copy > p strong { color: var(--soft-white); font-weight: 680; }
    .inklura-points { display: grid; gap: 11px; margin: 26px 0 0; padding: 0; list-style: none; }
    .inklura-points li { display: flex; align-items: center; gap: 10px; color: #bdc8c4; font-size: 13px; }
    .inklura-points svg { color: var(--mint); flex: 0 0 auto; }
    .inklura-actions { display: flex; flex-wrap: wrap; gap: 11px; margin-top: 28px; }

    .request-demo {
      position: relative;
      z-index: 1;
      padding: 18px;
      border: 1px solid rgba(255,255,255,.12);
      border-radius: 22px;
      background: rgba(8,12,11,.74);
      box-shadow: 0 28px 70px rgba(0,0,0,.36);
      backdrop-filter: blur(16px);
    }
    .request-window-top { display: flex; align-items: center; justify-content: space-between; gap: 18px; padding: 2px 2px 16px; }
    .request-window-title { display: flex; align-items: center; gap: 10px; color: var(--white); font-size: 13px; font-weight: 760; }
    .request-window-title svg { color: var(--mint); }
    .request-sync { display: flex; align-items: center; gap: 7px; color: var(--mint-soft); font-size: 9px; font-weight: 820; letter-spacing: .13em; text-transform: uppercase; }
    .request-sync::before { content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--mint); box-shadow: 0 0 10px var(--mint); }
    .request-form { padding: 14px; border: 1px solid var(--line); border-radius: 16px; background: var(--surface); }
    .request-ai-bar { display: flex; align-items: center; justify-content: space-between; gap: 10px; margin-bottom: 11px; padding: 9px 10px; border: 1px solid rgba(105,167,255,.25); border-radius: 9px; color: #82b6ff; background: rgba(105,167,255,.06); font-size: 10px; font-weight: 760; }
    .request-ai-label { display: flex; align-items: center; gap: 7px; }
    .request-ai-mode { color: var(--quiet); font-size: 8px; font-weight: 760; letter-spacing: .08em; text-transform: uppercase; }
    .request-label { margin-bottom: 7px; color: var(--quiet); font-size: 10px; font-weight: 780; letter-spacing: .11em; text-transform: uppercase; }
    .request-input { padding: 11px 12px; border: 1px solid var(--line); border-radius: 10px; color: #dfe6e3; background: #0b100e; font-size: 12px; }
    .request-site { display: flex; align-items: center; justify-content: space-between; gap: 7px; padding: 8px 10px; border: 1px solid rgba(105,167,255,.18); border-radius: 8px; color: #a8c9fa; background: rgba(105,167,255,.06); font: 10px ui-monospace, SFMono-Regular, Menlo, monospace; }
    .request-site-main { display: flex; align-items: center; gap: 7px; }
    .request-title-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; margin-top: 9px; }
    .request-ai-name { display: flex; align-items: center; gap: 6px; padding: 0 10px; border: 1px solid rgba(105,167,255,.24); border-radius: 9px; color: #82b6ff; background: rgba(105,167,255,.06); font-size: 9px; font-weight: 760; }
    .request-description { min-height: 58px; margin-top: 9px; padding: 10px 11px; border: 1px solid var(--line); border-radius: 10px; color: #899691; background: #0b100e; font-size: 10px; line-height: 1.5; }
    .attachment-panel { margin-top: 9px; padding: 9px; border: 1px solid var(--line); border-radius: 10px; background: rgba(255,255,255,.018); }
    .attachment-heading { display: flex; justify-content: space-between; gap: 10px; margin-bottom: 7px; color: var(--quiet); font-size: 8px; }
    .attachment-heading strong { color: #9facA7; font-weight: 720; }
    .attachment-tools { display: grid; grid-template-columns: repeat(3, 1fr); gap: 6px; }
    .attachment-tool { display: flex; align-items: center; justify-content: center; gap: 5px; min-width: 0; padding: 7px 5px; border: 1px solid var(--line); border-radius: 7px; color: #aab7b2; background: #0b100e; font-size: 8px; font-weight: 700; white-space: nowrap; }
    .request-form-bottom { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-top: 12px; }
    .priority-selected { display: flex; align-items: center; gap: 7px; color: var(--quiet); font-size: 8px; }
    .priority-selected strong { padding: 6px 9px; border: 1px solid rgba(105,167,255,.55); border-radius: 999px; color: #b9d6ff; background: rgba(105,167,255,.08); font-size: 9px; }
    .mini-submit { padding: 8px 12px; border-radius: 9px; color: #07110f; background: var(--gradient); font-size: 10px; font-weight: 820; }
    .request-flow { display: grid; grid-template-columns: repeat(3, 1fr); gap: 0; margin-top: 14px; }
    .flow-step { position: relative; padding: 25px 10px 3px 3px; color: var(--quiet); font-size: 9px; line-height: 1.4; }
    .flow-step::before { content: ""; position: absolute; left: 3px; top: 7px; width: 7px; height: 7px; border-radius: 2px; background: var(--mint); box-shadow: 0 0 9px rgba(46,196,168,.4); }
    .flow-step:not(:last-child)::after { content: ""; position: absolute; left: 13px; right: -2px; top: 10px; height: 1px; background: linear-gradient(90deg, rgba(46,196,168,.55), rgba(105,167,255,.2)); }
    .flow-step strong { display: block; margin-bottom: 3px; color: #cbd5d1; font-size: 10px; }

    .download-panel {
      position: relative;
      overflow: hidden;
      padding: clamp(28px, 5vw, 58px);
      border: 1px solid var(--line-strong);
      border-radius: 28px;
      background:
        radial-gradient(circle at 85% 15%, rgba(46,196,168,.13), transparent 34%),
        linear-gradient(145deg, #151c19, #101513);
      box-shadow: var(--shadow);
    }
    .download-panel::before { content: ""; position: absolute; width: 380px; height: 380px; right: -150px; top: -180px; border: 48px solid rgba(46,196,168,.06); border-radius: 28%; transform: rotate(22deg); }
    .download-top { position: relative; display: flex; justify-content: space-between; align-items: end; gap: 30px; }
    .download-top h2 { max-width: 650px; margin: 10px 0 0; color: var(--white); font-size: clamp(37px, 5vw, 58px); line-height: 1; letter-spacing: -.055em; }
    .version { color: var(--muted); font: 12px ui-monospace, SFMono-Regular, Menlo, monospace; }
    .download-grid { position: relative; display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; margin-top: 38px; }
    .download-card { padding: 22px; border: 1px solid var(--line); border-radius: 17px; background: rgba(9,13,12,.56); }
    .download-card.detected { border-color: var(--mint); box-shadow: inset 0 0 0 1px rgba(46,196,168,.12); }
    .platform { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
    .platform-name { display: flex; align-items: center; gap: 11px; }
    .os-icon { width: 27px; height: 27px; object-fit: contain; flex: 0 0 auto; }
    .platform strong { color: var(--white); font-size: 18px; }
    .platform-badge { display: none; color: var(--mint); font-size: 9px; font-weight: 850; letter-spacing: .12em; text-transform: uppercase; }
    .detected .platform-badge { display: block; }
    .download-meta { margin: 6px 0 18px; color: var(--quiet); font-size: 12px; }
    .download-card .button { width: 100%; min-height: 43px; }
    .install-box { position: relative; display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: 13px; margin-top: 14px; padding: 14px 15px; border: 1px solid var(--line); border-radius: 14px; background: #090d0c; }
    .install-box > span { color: var(--mint); font: 700 13px ui-monospace, SFMono-Regular, Menlo, monospace; }
    .install-box code { min-width: 0; overflow: hidden; color: #c9d5d1; font: 12px ui-monospace, SFMono-Regular, Menlo, monospace; white-space: nowrap; text-overflow: ellipsis; }
    .copy {
      display: grid;
      place-items: center;
      width: 34px;
      height: 34px;
      border: 1px solid var(--line);
      border-radius: 9px;
      color: var(--muted);
      background: rgba(255,255,255,.025);
      cursor: pointer;
    }
    .copy:hover { color: var(--white); border-color: var(--line-strong); }
    .copy.copied { color: var(--mint); }

    footer { padding: 30px 0 42px; }
    .footer-row { display: flex; align-items: center; justify-content: space-between; gap: 24px; color: var(--quiet); font-size: 12px; }
    .footer-links { display: flex; gap: 20px; }
    .footer-links a:hover { color: var(--white); }
    .wd29-credit { display: inline-grid; grid-template-columns: auto 62px; align-items: center; gap: 10px; color: var(--quiet); }
    .wd29-credit:hover { color: var(--mint); }
    .wd29-credit span { display: flex; align-items: center; height: 22px; font-size: 10px; font-weight: 750; line-height: 1; letter-spacing: .13em; text-transform: uppercase; }
    .wd29-logo { display: block; width: 62px; height: 22px; color: currentColor; }
    :focus-visible { outline: 2px solid var(--mint); outline-offset: 4px; }

    @media (max-width: 960px) {
      .hero { padding-top: 56px; }
      .hero-grid { grid-template-columns: 1fr; }
      .hero-copy { max-width: 700px; }
      .hero-visual { min-height: 500px; }
      .campaign-frame { transform: none; }
      .section-head { grid-template-columns: 1fr; gap: 18px; }
      .story-grid { grid-template-columns: 1fr; grid-template-rows: 470px 310px 310px; }
      .story-card:first-child { grid-row: auto; }
      .cap-grid { grid-template-columns: repeat(2, 1fr); }
      .ai-panel { grid-template-columns: 1fr; }
      .ai-visual { min-height: 460px; border-top: 1px solid var(--line); border-left: 0; }
      .ai-visual::after { background: linear-gradient(180deg, #111815 0, transparent 28%), linear-gradient(0deg, rgba(6,10,9,.7), transparent 36%); }
      .inklura-panel { grid-template-columns: 1fr; }
      .download-grid { grid-template-columns: 1fr; }
    }
    @media (max-width: 680px) {
      .shell { width: min(100% - 28px, 1180px); }
      nav { height: 64px; }
      .nav-links > a:not(.nav-cta) { display: none; }
      .hero { padding: 44px 0 38px; }
      .hero-grid { gap: 38px; }
      .hero h1 { font-size: clamp(47px, 14vw, 66px); }
      .hero-copy { font-size: 16px; }
      .hero-visual { min-height: 370px; }
      .floating-card { right: 10px; bottom: -18px; width: 230px; }
      .proof-row { grid-template-columns: repeat(2, 1fr); }
      .proof-item:nth-child(2) { border-right: 0; }
      .proof-item:nth-child(-n+2) { border-bottom: 1px solid var(--line); }
      section { padding: 66px 0; }
      .story-grid { grid-template-rows: repeat(3, 340px); }
      .story-content { left: 20px; right: 20px; bottom: 20px; }
      .cap-grid { grid-template-columns: 1fr; }
      .cap { padding: 18px 4px; }
      .ai-copy { padding: 30px 22px; }
      .ai-visual { min-height: 350px; }
      .ai-review-badge { top: 16px; right: 16px; }
      .ai-sequence { left: 14px; right: 14px; bottom: 14px; }
      .ai-sequence span { padding: 8px 5px; }
      .inklura-panel { padding: 26px 20px; }
      .inklura-panel::after { width: 240px; height: 344px; right: -84px; top: 560px; opacity: .08; }
      .request-ai-mode { display: none; }
      .request-flow { grid-template-columns: 1fr; }
      .flow-step { padding: 3px 0 17px 23px; }
      .flow-step::before { top: 6px; }
      .flow-step:not(:last-child)::after { left: 6px; right: auto; top: 17px; bottom: 0; width: 1px; height: auto; }
      .download-top { align-items: start; flex-direction: column; }
      .install-box { grid-template-columns: auto minmax(0, 1fr) auto; }
      .footer-row { align-items: flex-start; flex-direction: column; }
    }
    @media (prefers-reduced-motion: reduce) {
      html { scroll-behavior: auto; }
      *, *::before, *::after { transition: none !important; }
    }
  </style>
</head>
<body>
  <header class="nav-wrap">
    <nav class="shell" aria-label="Navigation principale">
      <a class="brand" href="#" aria-label="ShellDeck, accueil">
        <svg class="brand-mark" viewBox="125 125 774 774" aria-hidden="true">
          <path fill="#2ec4a8" d="M207 125h610l82 70v634l-82 70H207l-82-70V195z"/>
          <path fill="#1a1a1a" d="M246 189h532l40 36v574l-40 28H246l-39-28V225z"/>
          <path fill="#e6e6e6" d="M261 432l121 80-121 80h43l122-80-122-80zM763 432l-121 80 121 80h-43l-122-80 122-80z"/>
          <rect x="417" y="624" width="190" height="40" rx="6" fill="#e6e6e6"/>
        </svg>
        <span class="wordmark">Shell<b>Deck</b></span>
      </a>
      <div class="nav-links">
        <a href="#produit">Produit</a>
        <a href="#capacites">Capacités</a>
        <a href="#ia">IA</a>
        <a href="#inklura">Inklura</a>
        <a class="nav-cta" href="#telecharger">
          Télécharger
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M12 3v12m0 0 4-4m-4 4-4-4"/><path d="M5 21h14"/></svg>
        </a>
      </div>
    </nav>
  </header>

  <main>
    <section class="hero">
      <div class="shell hero-grid">
        <div>
          <div class="eyebrow">Votre infra, enfin réunie</div>
          <h1>Connectez.<br> Pilotez.<br><span class="gradient-text">Respirez.</span></h1>
          <p class="hero-copy"><strong>SSH, terminaux, scripts et tunnels</strong> dans un seul espace de contrôle. ShellDeck est l’application desktop native qui remet du calme dans votre quotidien d’exploitation.</p>
          <div class="actions">
            <a class="button button-primary" href="#telecharger">
              <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" aria-hidden="true"><path d="M12 3v12m0 0 4-4m-4 4-4-4"/><path d="M5 21h14"/></svg>
              Télécharger ShellDeck
            </a>
            <a class="button button-secondary" href="${GITHUB}" target="_blank" rel="noopener">
              Voir sur GitHub
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M5 12h14m-5-5 5 5-5 5"/></svg>
            </a>
          </div>
          <div class="hero-note">Version ${v} · Gratuit et open source · Linux, macOS et Windows</div>
        </div>

        <div class="hero-visual" aria-label="Illustration du poste de contrôle ShellDeck">
          <div class="campaign-frame">
            <img src="/campaign/control-v2.webp" alt="" width="1599" height="900">
            <div class="window-bar">
              <div class="dots"><i></i><i></i><i></i></div>
              <span>~/infrastructure</span>
              <span>ShellDeck</span>
            </div>
          </div>
          <div class="floating-card">
            <div class="floating-card-top"><span>Flotte</span><span class="live">En ligne</span></div>
            <div class="host-list">
              <div class="host"><span>activ-2</span><span>18 ms</span></div>
              <div class="host"><span>production-web</span><span>24 ms</span></div>
              <div class="host"><span>staging-api</span><span>31 ms</span></div>
            </div>
          </div>
        </div>
      </div>
    </section>

    <div class="proof">
      <div class="shell proof-row">
        <div class="proof-item"><div class="proof-value">100 % natif</div><div class="proof-label">Construit en Rust avec GPUI</div></div>
        <div class="proof-item"><div class="proof-value">Rendu GPU</div><div class="proof-label">Fluide, même sous charge</div></div>
        <div class="proof-item"><div class="proof-value">Multi-plateforme</div><div class="proof-label">Linux, macOS et Windows</div></div>
        <div class="proof-item"><div class="proof-value">Open source</div><div class="proof-label">Transparent et extensible</div></div>
      </div>
    </div>

    <section id="produit">
      <div class="shell">
        <div class="section-head">
          <div><div class="section-kicker">Un seul point de vue</div><h2>Moins d’outils.<br>Plus de contrôle.</h2></div>
          <p>Passez d’un serveur à l’autre sans passer d’une application à l’autre. ShellDeck garde vos connexions, vos sessions et vos opérations au même endroit — sans toucher à votre configuration SSH.</p>
        </div>
        <div class="story-grid">
          <article class="story-card">
            <img src="/campaign/fleet-v2.webp" alt="Icône ShellDeck reliant une flotte de serveurs" width="1199" height="675">
            <div class="story-content">
              <div class="story-index">01 / FLOTTE</div>
              <h3>Chaque site.<br><span class="gradient-text">Toujours à portée.</span></h3>
              <p>Organisez vos hôtes, synchronisez vos profils et retrouvez instantanément le bon environnement.</p>
            </div>
          </article>
          <article class="story-card">
            <img src="/campaign/automation-v2.webp" alt="Modules ShellDeck circulant sur des rails d’exécution" width="1199" height="675">
            <div class="story-content">
              <div class="story-index">02 / AUTOMATISATION</div>
              <h3>Répétez moins. Exécutez mieux.</h3>
              <p>Lancez vos scripts à distance et suivez chaque résultat en direct.</p>
            </div>
          </article>
          <article class="story-card">
            <img src="/campaign/native-v2.webp" alt="Monolithe ShellDeck natif en verre sombre" width="1199" height="675">
            <div class="story-content">
              <div class="story-index">03 / NATIF</div>
              <h3>Le terminal. Sans le chaos.</h3>
              <p>Une expérience desktop rapide, précise et pensée pour durer.</p>
            </div>
          </article>
        </div>
      </div>
    </section>

    <section class="capabilities" id="capacites">
      <div class="shell">
        <div class="section-head">
          <div><div class="section-kicker">Le deck complet</div><h2>Tout ce qu’il faut.<br>Rien qui déborde.</h2></div>
          <p>Des outils d’exploitation concrets, rassemblés dans une interface cohérente. Chaque surface reste lisible, rapide et proche de votre flux de travail.</p>
        </div>
        <div class="cap-grid">
          <article class="cap"><div class="cap-icon"><svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3m6 0h4"/></svg></div><h3>Terminaux natifs</h3><p>Sessions locales et SSH, onglets persistants, couleurs et rendu de texte accéléré par le GPU.</p></article>
          <article class="cap"><div class="cap-icon"><svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="4" y="4" width="6" height="6" rx="2"/><rect x="14" y="14" width="6" height="6" rx="2"/><path d="M10 7h4a3 3 0 0 1 3 3v4M7 10v4a3 3 0 0 0 3 3h4"/></svg></div><h3>Connexions SSH</h3><p>Import de ~/.ssh/config, groupes, clés, jump hosts et accès en un clic à chaque machine.</p></article>
          <article class="cap"><div class="cap-icon"><svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="6" cy="12" r="3"/><circle cx="18" cy="12" r="3"/><path d="M9 12h6"/></svg></div><h3>Tunnels visuels</h3><p>Forwarding local, distant et SOCKS avec une carte claire des ports actifs.</p></article>
          <article class="cap"><div class="cap-icon"><svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M6 3h9l3 3v15H6z"/><path d="M14 3v4h4M9 12l2 2-2 2m4 0h2"/></svg></div><h3>Scripts distants</h3><p>Éditez, enregistrez et exécutez vos routines avec sortie en direct et historique.</p></article>
          <article class="cap"><div class="cap-icon"><svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M20 7h-9M14 17H5"/><circle cx="7" cy="7" r="2"/><circle cx="17" cy="17" r="2"/></svg></div><h3>Contrôle multi-sites</h3><p>Une flotte organisée par contexte, avec changement de site et accès Manage intégrés.</p></article>
          <article class="cap"><div class="cap-icon"><svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M20 16V8l-8-5-8 5v8l8 5z"/><path d="m8.5 12 2.2 2.2 4.8-5"/></svg></div><h3>Secrets protégés</h3><p>Les identifiants restent dans le trousseau du système, jamais dans votre configuration SSH.</p></article>
        </div>
      </div>
    </section>

    <section class="ai-section" id="ia">
      <div class="shell ai-panel">
        <div class="ai-copy">
          <div class="section-kicker">IA contextuelle</div>
          <h2>Elle prépare.<br><span class="gradient-text">Vous décidez.</span></h2>
          <p>ShellDeck rassemble uniquement le contexte utile et demande à votre fournisseur IA de produire un <strong>brouillon clair à relire</strong>. Rien n’est exécuté, envoyé ou modifié sans une action explicite de votre part.</p>
          <div class="ai-contexts">
            <span>Terminal</span>
            <span>Demandes</span>
            <span>Scripts</span>
            <span>Support</span>
          </div>
          <div class="ai-safety">
            <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" aria-hidden="true"><path d="M20 13c0 5-3.5 7.5-8 9-4.5-1.5-8-4-8-9V5l8-3 8 3z"/><path d="m9 12 2 2 4-4"/></svg>
            Brouillon contrôlé par l’utilisateur
          </div>
        </div>
        <div class="ai-visual" aria-label="Le contexte ShellDeck devient un brouillon soumis à validation">
          <img src="/campaign/ai-context-v1.webp" alt="" width="1400" height="788">
          <div class="ai-review-badge">Validation humaine</div>
          <div class="ai-sequence">
            <span><b>01</b>Contexte borné</span>
            <span><b>02</b>Brouillon IA</span>
            <span><b>03</b>Votre validation</span>
          </div>
        </div>
      </div>
    </section>

    <section class="inklura-section" id="inklura">
      <div class="shell inklura-panel">
        <div class="inklura-copy">
          <div class="inklura-lockup">
            <img class="inklura-logo" src="/brand/inklura-dark.svg" width="400" height="180" alt="Inklura">
            <span class="inklura-audience">Demandes utilisateurs</span>
          </div>
          <h2>Votre demande.<br><span class="gradient-text">Déjà au bon endroit.</span></h2>
          <p>Depuis ShellDeck, créez une demande avec le <strong>site concerné, le contexte et vos captures</strong>. Elle rejoint immédiatement Inklura Manage, où l’équipe peut la qualifier, vous répondre et suivre sa résolution.</p>
          <ul class="inklura-points">
            <li><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m5 12 4 4L19 6"/></svg>Créez et suivez vos demandes sans quitter ShellDeck</li>
            <li><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m5 12 4 4L19 6"/></svg>Ajoutez captures, commentaires et site concerné</li>
            <li><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m5 12 4 4L19 6"/></svg>Retrouvez le même fil dans Inklura Manage</li>
          </ul>
          <div class="inklura-actions">
            <a class="button button-primary" href="#telecharger">Utiliser ShellDeck</a>
            <a class="button button-secondary" href="https://manage.inklura.fr/manage/ai-operations/issues" target="_blank" rel="noopener">
              Accéder à Inklura Manage
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M15 4h5v5M20 4l-9 9"/><path d="M18 13v6a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h6"/></svg>
            </a>
          </div>
        </div>

        <div class="request-demo" aria-label="Aperçu d’une demande Inklura créée dans ShellDeck">
          <div class="request-window-top">
            <div class="request-window-title">
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4Z"/><path d="M8 9h8M8 13h5"/></svg>
              Nouvelle demande
            </div>
            <div class="request-sync">Synchronisé</div>
          </div>
          <div class="request-form">
            <div class="request-ai-bar">
              <span class="request-ai-label">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="m12 3 1.3 4.2L17.5 8.5l-4.2 1.3L12 14l-1.3-4.2-4.2-1.3 4.2-1.3z"/><path d="m18.5 14 .8 2.2 2.2.8-2.2.8-.8 2.2-.8-2.2-2.2-.8 2.2-.8z"/></svg>
                Préparer avec l’IA
              </span>
              <span class="request-ai-mode">Brouillon uniquement</span>
            </div>
            <div class="request-label">Site concerné</div>
            <div class="request-site">
              <span class="request-site-main">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a15 15 0 0 1 0 18M12 3a15 15 0 0 0 0 18"/></svg>
                example.fr
              </span>
              <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m7 10 5 5 5-5"/></svg>
            </div>
            <div class="request-title-row">
              <div class="request-input">Le déploiement reste bloqué en staging</div>
              <div class="request-ai-name">
                <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="m12 3 1.3 4.2L17.5 8.5l-4.2 1.3L12 14l-1.3-4.2-4.2-1.3 4.2-1.3z"/></svg>
                Nommer
              </div>
            </div>
            <div class="request-description">Après la dernière mise à jour, la phase de publication ne se termine plus. Capture et sortie du terminal jointes.</div>
            <div class="attachment-panel">
              <div class="attachment-heading"><strong>Images jointes</strong><span>PNG, JPEG, WebP</span></div>
              <div class="attachment-tools">
                <span class="attachment-tool">
                  <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M12 16V4m0 0-4 4m4-4 4 4"/><path d="M5 14v5h14v-5"/></svg>
                  Fichier
                </span>
                <span class="attachment-tool">
                  <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="7" y="4" width="10" height="16" rx="2"/><path d="M9 4V2h6v2M10 9h4m-4 4h4"/></svg>
                  Coller
                </span>
                <span class="attachment-tool">
                  <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 8V4h4M16 4h4v4M20 16v4h-4M8 20H4v-4"/><rect x="7" y="8" width="10" height="8" rx="1"/></svg>
                  Capturer
                </span>
              </div>
            </div>
            <div class="request-form-bottom">
              <div class="priority-selected">Priorité <strong>Normale</strong></div>
              <div class="mini-submit">Créer la demande</div>
            </div>
          </div>
          <div class="request-flow">
            <div class="flow-step"><strong>ShellDeck</strong>Demande créée</div>
            <div class="flow-step"><strong>Inklura Manage</strong>Qualification &amp; réponse</div>
            <div class="flow-step"><strong>Suivi partagé</strong>Jusqu’à résolution</div>
          </div>
        </div>
      </div>
    </section>

    <section id="telecharger">
      <div class="shell download-panel">
        <div class="download-top">
          <div><div class="section-kicker">Prêt à prendre le contrôle ?</div><h2>Téléchargez ShellDeck.</h2></div>
          <div class="version">version ${v}</div>
        </div>
        <div class="download-grid">
          <article class="download-card" data-platform="linux">
            <div class="platform">
              <div class="platform-name">
                <img class="os-icon" src="https://cdn.simpleicons.org/linux/FCC624?viewbox=auto&amp;size=27" width="27" height="27" alt="" referrerpolicy="no-referrer">
                <strong>Linux</strong>
              </div>
              <span class="platform-badge">Votre système</span>
            </div>
            <div class="download-meta">AppImage · x86_64${linuxMeta}</div>
            <a class="button button-primary" href="${linuxUrl}">Télécharger pour Linux</a>
          </article>
          <article class="download-card" data-platform="macos">
            <div class="platform">
              <div class="platform-name">
                <img class="os-icon" src="https://cdn.simpleicons.org/apple/E6E6E6?viewbox=auto&amp;size=27" width="27" height="27" alt="" referrerpolicy="no-referrer">
                <strong>macOS</strong>
              </div>
              <span class="platform-badge">Votre système</span>
            </div>
            <div class="download-meta">DMG · Apple Silicon${macosMeta}</div>
            <a class="button button-primary" href="${macosUrl}">Télécharger pour macOS</a>
          </article>
          <article class="download-card" data-platform="windows">
            <div class="platform">
              <div class="platform-name">
                <svg class="os-icon" viewBox="0 0 24 24" aria-hidden="true">
                  <path fill="#69a7ff" d="M2 4.45 10.15 3.3v7.83H2V4.45Zm9.25-1.31L22 1.62v9.51H11.25V3.14ZM2 12.25h8.15v7.84L2 18.94v-6.69Zm9.25 0H22v9.51l-10.75-1.52v-7.99Z"/>
                </svg>
                <strong>Windows</strong>
              </div>
              <span class="platform-badge">Votre système</span>
            </div>
            <div class="download-meta">Installeur · x86_64${windowsMeta}</div>
            <a class="button button-primary" href="${windowsUrl}">Télécharger pour Windows</a>
          </article>
        </div>
        <div class="install-box">
          <span>$</span>
          <code>curl -fsSL https://shelldeck.1clic.pro/install.sh | bash</code>
          <button class="copy" type="button" aria-label="Copier la commande d’installation">
            <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M15 9V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h3"/></svg>
          </button>
        </div>
      </div>
    </section>
  </main>

  <footer>
    <div class="shell footer-row">
      <a class="brand" href="#"><svg class="brand-mark" viewBox="125 125 774 774" aria-hidden="true"><path fill="#2ec4a8" d="M207 125h610l82 70v634l-82 70H207l-82-70V195z"/><path fill="#1a1a1a" d="M246 189h532l40 36v574l-40 28H246l-39-28V225z"/><path fill="#e6e6e6" d="M261 432l121 80-121 80h43l122-80-122-80zM763 432l-121 80 121 80h-43l-122-80 122-80z"/><rect x="417" y="624" width="190" height="40" rx="6" fill="#e6e6e6"/></svg><span class="wordmark">Shell<b>Deck</b></span></a>
      <div class="footer-links"><a href="${GITHUB}" target="_blank" rel="noopener">GitHub</a><a href="${GITHUB_RELEASES}" target="_blank" rel="noopener">Versions</a><span>Licence MIT</span></div>
      <a class="wd29-credit" href="https://webdesign29.net" target="_blank" rel="noopener" aria-label="Conçu par Webdesign29">
        <span>Conçu par</span>
        <svg class="wd29-logo" viewBox="0 0 92 33" aria-hidden="true">
          <g transform="translate(29.3553 28.2191)"><path fill="currentColor" d="M0-23.419V-9.791l-9.023-13.628h-4.674v13.85l-9.142-13.85h-6.516L-13.697 0h5.543v-12.623L.426 0H5.97v-23.419H0Z"/></g>
          <g transform="translate(42.8962 10.0868)"><path fill="currentColor" d="M0 12.93h2.567c4.469 0 5.737-3.105 5.737-6.434 0-1.33-.286-3.01-1.142-4.341C6.466 1.109 5.261 0 2.599 0H0v12.93ZM-5.99-5.261h7.765c2.631 0 7.321 0 10.364 4.374 1.616 2.228 2.154 4.731 2.154 7.299 0 6.402-3.169 11.789-12.074 11.789H-5.99V-5.261"/></g>
          <g transform="translate(57.9099 9.3764)"><path fill="currentColor" d="M0 14.325c2.086-1.745 4.143-3.487 6.197-5.231 2.428-2.024 4.016-3.425 4.016-5.916 0-1.837-1.059-2.585-2.459-2.585-2.025 0-2.336 1.961-2.367 3.643H-.218c.062-1.494.125-3.394 1.37-5.293 1.962-3.053 5.418-3.519 7.099-3.519 5.138 0 7.847 3.424 7.847 7.13 0 2.803-.84 4.827-3.798 7.536-1.402 1.277-2.803 2.552-4.236 3.861h10.323l-3.732 4.95H0v-4.576"/></g>
          <g transform="translate(85.2243 19.964)"><path fill="currentColor" d="M0-6.85c0-2.086-1.683-3.145-3.177-3.145-1.681 0-3.176 1.246-3.176 3.083 0 1.494 1.089 3.145 3.176 3.145C-1.246-3.767 0-5.2 0-6.819v-.031ZM-8.875 8.314l4.173-5.698c.31-.436.622-.841.901-1.245-.03-.032-.745.062-1.181.062-3.364 0-7.256-2.585-7.256-7.816 0-5.262 4.08-8.781 8.936-8.781 2.896 0 5.574 1.059 7.132 3.114 1.463 1.651 1.961 4.017 1.961 5.792 0 2.335-.966 4.39-2.117 6.165l-5.731 8.407h-6.818"/></g>
        </svg>
      </a>
    </div>
  </footer>

  <script>
    (function () {
      var platform = navigator.platform || "";
      var ua = navigator.userAgent || "";
      var os = /Linux/.test(platform) ? "linux" : /Mac/.test(platform) ? "macos" : /Win/.test(platform) ? "windows" : "";
      if (!os && /Android/.test(ua)) os = "linux";
      if (!os && /iPhone|iPad/.test(ua)) os = "macos";
      var card = os && document.querySelector('[data-platform="' + os + '"]');
      if (card) card.classList.add("detected");

      var copy = document.querySelector(".copy");
      if (copy) copy.addEventListener("click", function () {
        var code = copy.parentElement.querySelector("code").textContent;
        var original = copy.innerHTML;
        navigator.clipboard.writeText(code).then(function () {
          copy.classList.add("copied");
          copy.setAttribute("aria-label", "Commande copiée");
          copy.innerHTML = '<svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m5 12 4 4L19 6"/></svg>';
          setTimeout(function () {
            copy.classList.remove("copied");
            copy.setAttribute("aria-label", "Copier la commande d’installation");
            copy.innerHTML = original;
          }, 1600);
        });
      });
    })();
  </script>
</body>
</html>`;

  return new Response(html, {
    headers: {
      "Content-Type": "text/html;charset=UTF-8",
      "Cache-Control": "public, max-age=300",
      "Content-Language": "fr",
    },
  });
}
