export interface Env {
  SHELLDECK_KV: KVNamespace;
}

interface PlatformRelease {
  url: string;
  sha256: string;
  size: number;
}

interface UpdateManifest {
  version: string;
  pub_date: string;
  platforms: Record<string, PlatformRelease>;
}

const CORS_HEADERS: Record<string, string> = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
};

function jsonResponse(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      "Content-Type": "application/json",
      ...CORS_HEADERS,
    },
  });
}

function textResponse(text: string, status = 200): Response {
  return new Response(text, {
    status,
    headers: {
      "Content-Type": "text/plain",
      ...CORS_HEADERS,
    },
  });
}

function htmlResponse(html: string, status = 200): Response {
  return new Response(html, {
    status,
    headers: {
      "Content-Type": "text/html;charset=UTF-8",
      "Cache-Control": "public, max-age=300",
    },
  });
}

interface DownloadInfo {
  version: string;
  linux?: { url: string; size: number };
  macos?: { url: string; size: number };
  windows?: { url: string; size: number };
}

async function getDownloadInfo(env: Env): Promise<DownloadInfo> {
  const fallback: DownloadInfo = { version: "0.1.2" };
  try {
    const raw = await env.SHELLDECK_KV.get("latest-release");
    if (!raw) return fallback;
    const manifest: UpdateManifest = JSON.parse(raw);
    const info: DownloadInfo = { version: manifest.version };
    if (manifest.platforms["linux-x86_64"]) {
      info.linux = {
        url: manifest.platforms["linux-x86_64"].url,
        size: manifest.platforms["linux-x86_64"].size,
      };
    }
    if (manifest.platforms["darwin-aarch64"]) {
      info.macos = {
        url: manifest.platforms["darwin-aarch64"].url,
        size: manifest.platforms["darwin-aarch64"].size,
      };
    } else if (manifest.platforms["darwin-x86_64"]) {
      info.macos = {
        url: manifest.platforms["darwin-x86_64"].url,
        size: manifest.platforms["darwin-x86_64"].size,
      };
    }
    if (manifest.platforms["windows-x86_64"]) {
      info.windows = {
        url: manifest.platforms["windows-x86_64"].url,
        size: manifest.platforms["windows-x86_64"].size,
      };
    }
    return info;
  } catch {
    return fallback;
  }
}

function formatSize(bytes: number): string {
  if (bytes >= 1048576) return (bytes / 1048576).toFixed(1) + " MB";
  if (bytes >= 1024) return (bytes / 1024).toFixed(1) + " KB";
  return bytes + " B";
}

const GITHUB_RELEASES = "https://github.com/benfavre/shelldeck/releases";

async function renderLandingPage(env: Env): Promise<Response> {
  const dl = await getDownloadInfo(env);
  const v = dl.version;
  const linuxUrl = dl.linux?.url ?? `${GITHUB_RELEASES}/latest`;
  const macosUrl = dl.macos?.url ?? `${GITHUB_RELEASES}/latest`;
  const windowsUrl = dl.windows?.url ?? `${GITHUB_RELEASES}/latest`;
  const linuxSize = dl.linux ? formatSize(dl.linux.size) : "";
  const macosSize = dl.macos ? formatSize(dl.macos.size) : "";
  const windowsSize = dl.windows ? formatSize(dl.windows.size) : "";

  const html = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>ShellDeck — GPU-Accelerated SSH &amp; Terminal Companion</title>
<meta name="description" content="A GPU-accelerated native desktop SSH and terminal companion app. Manage connections, forward ports, run scripts — all from one polished UI.">
<style>
  :root {
    --bg: #1a1b26;
    --bg-surface: #1e2030;
    --bg-card: #24283b;
    --accent: #7aa2f7;
    --accent-hover: #89b4fa;
    --text: #c0caf5;
    --text-muted: #565f89;
    --text-bright: #eef0ff;
    --border: #292e42;
    --green: #9ece6a;
    --orange: #ff9e64;
    --red: #f7768e;
    --purple: #bb9af7;
    --gradient-accent: linear-gradient(135deg, #7aa2f7, #bb9af7);
  }

  @keyframes float1 {
    0%, 100% { transform: translate(0, 0) scale(1); }
    33% { transform: translate(30px, -20px) scale(1.05); }
    66% { transform: translate(-20px, 15px) scale(0.95); }
  }
  @keyframes float2 {
    0%, 100% { transform: translate(0, 0) scale(1); }
    33% { transform: translate(-25px, 20px) scale(0.95); }
    66% { transform: translate(20px, -25px) scale(1.05); }
  }
  @keyframes float3 {
    0%, 100% { transform: translate(0, 0) scale(1); }
    50% { transform: translate(15px, 25px) scale(1.03); }
  }
  @keyframes fadeInUp {
    from { opacity: 0; transform: translateY(20px); }
    to { opacity: 1; transform: translateY(0); }
  }
  @keyframes shimmer {
    0% { background-position: -200% center; }
    100% { background-position: 200% center; }
  }
  @keyframes cursorBlink {
    0%, 100% { opacity: 1; }
    50% { opacity: 0; }
  }
  @keyframes pulseGlow {
    0%, 100% { box-shadow: 0 0 15px rgba(122, 162, 247, 0.3); }
    50% { box-shadow: 0 0 25px rgba(122, 162, 247, 0.5); }
  }
  *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
  html { scroll-behavior: smooth; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
    background: var(--bg);
    color: var(--text);
    line-height: 1.6;
    -webkit-font-smoothing: antialiased;
  }
  a { color: var(--accent); text-decoration: none; }
  a:hover { color: var(--accent-hover); }

  /* Nav */
  nav {
    position: fixed; top: 0; left: 0; right: 0; z-index: 100;
    background: rgba(26, 27, 38, 0.88);
    backdrop-filter: blur(16px);
    border-bottom: 1px solid var(--border);
    padding: 0 2rem;
    height: 64px;
    display: flex; align-items: center; justify-content: space-between;
  }
  .nav-brand { display: flex; align-items: center; gap: 0.75rem; font-weight: 700; font-size: 1.1rem; color: var(--text-bright); }
  .nav-links { display: flex; gap: 1.5rem; align-items: center; }
  .nav-links a {
    color: var(--text-muted); font-size: 0.9rem; transition: color 0.2s;
    position: relative; padding-bottom: 2px;
  }
  .nav-links a::after {
    content: ""; position: absolute; bottom: -2px; left: 0; right: 0; height: 2px;
    background: var(--gradient-accent); border-radius: 1px;
    transform: scaleX(0); transition: transform 0.25s ease;
  }
  .nav-links a:hover { color: var(--text-bright); }
  .nav-links a:hover::after { transform: scaleX(1); }

  /* Hero */
  .hero {
    min-height: 100vh;
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    text-align: center;
    padding: 7rem 2rem 5rem;
    position: relative;
    overflow: hidden;
  }
  .hero::after {
    content: "";
    position: absolute; inset: 0;
    background-image: radial-gradient(rgba(122, 162, 247, 0.07) 1px, transparent 1px);
    background-size: 24px 24px;
    pointer-events: none;
    opacity: 0.5;
  }
  .hero-orb {
    position: absolute; border-radius: 50%; pointer-events: none;
    filter: blur(80px); opacity: 0.5;
  }
  .hero-orb--1 {
    width: 400px; height: 400px; top: 10%; left: 15%;
    background: rgba(122, 162, 247, 0.15);
    animation: float1 12s ease-in-out infinite;
  }
  .hero-orb--2 {
    width: 350px; height: 350px; top: 20%; right: 10%;
    background: rgba(187, 154, 247, 0.12);
    animation: float2 15s ease-in-out infinite;
  }
  .hero-orb--3 {
    width: 300px; height: 300px; bottom: 15%; left: 40%;
    background: rgba(122, 162, 247, 0.1);
    animation: float3 10s ease-in-out infinite;
  }
  .hero-content { position: relative; z-index: 1; }
  .hero-logo {
    position: relative; margin-bottom: 2rem;
    animation: fadeInUp 0.6s ease both;
  }
  .hero h1 {
    font-size: clamp(2.8rem, 7vw, 4.5rem);
    font-weight: 800;
    letter-spacing: -0.03em;
    margin-bottom: 1rem;
    background: var(--gradient-accent);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
    animation: fadeInUp 0.6s ease 0.1s both;
  }
  .hero .tagline {
    font-size: clamp(1.05rem, 2.5vw, 1.4rem);
    color: var(--text-muted);
    max-width: 620px;
    margin: 0 auto 2.5rem;
    line-height: 1.7;
    animation: fadeInUp 0.6s ease 0.2s both;
  }
  .hero-actions {
    display: flex; gap: 1rem; flex-wrap: wrap; justify-content: center;
    animation: fadeInUp 0.6s ease 0.3s both;
  }
  .hero-strip {
    display: flex; gap: 1.5rem; align-items: center; justify-content: center;
    margin-top: 2rem; flex-wrap: wrap;
    animation: fadeInUp 0.6s ease 0.4s both;
  }
  .hero-strip span {
    font-size: 0.85rem; color: var(--text-muted); display: flex; align-items: center; gap: 0.4rem;
  }
  .hero-strip span::before {
    content: ""; display: inline-block; width: 6px; height: 6px;
    background: var(--accent); border-radius: 50%; flex-shrink: 0;
  }
  .btn {
    display: inline-flex; align-items: center; gap: 0.5rem;
    padding: 0.75rem 1.75rem;
    border-radius: 8px;
    font-size: 1rem;
    font-weight: 600;
    border: none;
    cursor: pointer;
    transition: all 0.25s;
  }
  .btn-primary {
    background: var(--gradient-accent);
    color: var(--bg);
    box-shadow: 0 0 20px rgba(122, 162, 247, 0.25);
    position: relative; overflow: hidden;
  }
  .btn-primary::after {
    content: "";
    position: absolute; inset: 0;
    background: linear-gradient(90deg, transparent 0%, rgba(255,255,255,0.2) 50%, transparent 100%);
    background-size: 200% 100%;
    animation: shimmer 3s ease-in-out infinite;
  }
  .btn-primary:hover {
    color: var(--bg); transform: translateY(-2px);
    box-shadow: 0 0 30px rgba(122, 162, 247, 0.4);
  }
  .btn-secondary {
    background: transparent;
    color: var(--text);
    border: 1px solid var(--border);
  }
  .btn-secondary:hover { border-color: var(--text-muted); color: var(--text-bright); transform: translateY(-2px); }
  .version-badge {
    display: inline-block;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 20px;
    padding: 0.3rem 0.9rem;
    font-size: 0.8rem;
    color: var(--text-muted);
    margin-bottom: 1.5rem;
    animation: fadeInUp 0.6s ease both, pulseGlow 4s ease-in-out infinite;
  }

  /* Section */
  section { padding: 6rem 2rem; max-width: 1100px; margin: 0 auto; }
  .section-divider {
    height: 1px; max-width: 1100px; margin: 0 auto;
    background: linear-gradient(90deg, transparent, var(--accent), var(--purple), transparent);
    opacity: 0.3;
  }
  .section-title {
    text-align: center;
    font-size: 2rem;
    font-weight: 700;
    margin-bottom: 0.75rem;
    background: var(--gradient-accent);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
  }
  .section-subtitle {
    text-align: center;
    color: var(--text-muted);
    max-width: 550px;
    margin: 0 auto 3rem;
    line-height: 1.7;
  }

  /* Features grid */
  .features-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: 1.5rem;
  }
  .feature-card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 2rem;
    transition: transform 0.3s, box-shadow 0.3s, border-color 0.3s;
    position: relative;
  }
  .feature-card:hover {
    border-color: rgba(122, 162, 247, 0.4);
    transform: translateY(-3px);
    box-shadow: 0 8px 30px rgba(122, 162, 247, 0.08);
  }
  .feature-icon {
    width: 40px; height: 40px;
    border-radius: 8px;
    display: flex; align-items: center; justify-content: center;
    margin-bottom: 1rem;
    font-size: 1.2rem;
  }
  .feature-icon.gpu { background: rgba(122, 162, 247, 0.15); color: var(--accent); }
  .feature-icon.ssh { background: rgba(158, 206, 106, 0.15); color: var(--green); }
  .feature-icon.term { background: rgba(187, 154, 247, 0.15); color: var(--purple); }
  .feature-icon.port { background: rgba(255, 158, 100, 0.15); color: var(--orange); }
  .feature-icon.script { background: rgba(247, 118, 142, 0.15); color: var(--red); }
  .feature-icon.sync { background: rgba(122, 162, 247, 0.15); color: var(--accent); }
  .feature-card h3 { font-size: 1.05rem; color: var(--text-bright); margin-bottom: 0.5rem; }
  .feature-card p { font-size: 0.9rem; color: var(--text-muted); line-height: 1.55; }

  /* Downloads */
  .downloads-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 1.5rem;
  }
  .dl-card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 2.25rem;
    text-align: center;
    transition: transform 0.3s, box-shadow 0.3s, border-color 0.3s;
  }
  .dl-card:hover {
    border-color: rgba(122, 162, 247, 0.4);
    transform: translateY(-3px);
    box-shadow: 0 8px 30px rgba(122, 162, 247, 0.08);
  }
  .dl-card.detected { border-color: var(--accent); box-shadow: 0 0 20px rgba(122, 162, 247, 0.1); }
  .dl-card .os-icon { font-size: 2rem; margin-bottom: 0.75rem; }
  .dl-card h3 { font-size: 1.1rem; color: var(--text-bright); margin-bottom: 0.25rem; }
  .dl-card .dl-meta { font-size: 0.8rem; color: var(--text-muted); margin-bottom: 1.25rem; }
  .dl-card .btn { width: 100%; justify-content: center; }
  .dl-card .dl-hint {
    display: none;
    font-size: 0.75rem;
    color: var(--accent);
    margin-top: 0.75rem;
  }
  .dl-card.detected .dl-hint { display: block; }

  /* Footer */
  footer {
    border-top: 1px solid transparent;
    border-image: linear-gradient(90deg, transparent, var(--accent), var(--purple), transparent) 1;
    padding: 2.5rem 2rem;
    text-align: center;
    color: var(--text-muted);
    font-size: 0.85rem;
  }
  footer .footer-links { display: flex; gap: 1.5rem; justify-content: center; margin-bottom: 0.75rem; }

  /* Install */
  .install-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(380px, 1fr));
    gap: 1.5rem;
  }
  .install-card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 1.5rem;
  }
  .install-label {
    font-size: 0.85rem;
    color: var(--text-muted);
    margin-bottom: 0.75rem;
    font-weight: 600;
  }
  .install-code {
    display: flex;
    align-items: center;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.75rem 1rem;
    gap: 0.75rem;
  }
  .install-code code {
    flex: 1;
    font-family: "SF Mono", "Fira Code", "Cascadia Code", monospace;
    font-size: 0.85rem;
    color: var(--green);
    white-space: nowrap;
    overflow-x: auto;
  }
  .copy-btn {
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.4rem;
    cursor: pointer;
    color: var(--text-muted);
    transition: all 0.2s;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .copy-btn:hover { border-color: var(--accent); color: var(--accent); }
  .copy-btn.copied { border-color: var(--green); color: var(--green); }

  /* Hero install */
  .hero-install {
    width: 100%; max-width: 540px;
    margin-top: 2rem;
    animation: fadeInUp 0.6s ease 0.35s both;
    position: relative; z-index: 1;
  }
  .hero-install-label {
    font-size: 0.8rem; color: var(--text-muted); margin-bottom: 0.5rem;
    display: flex; align-items: center; gap: 0.4rem;
  }
  .hero-install-label .os-name { color: var(--accent); font-weight: 600; }
  .hero-install-box {
    display: flex; align-items: center;
    background: rgba(26, 27, 38, 0.8);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 0.7rem 0.9rem;
    gap: 0.75rem;
    backdrop-filter: blur(8px);
    transition: border-color 0.2s;
  }
  .hero-install-box:hover { border-color: rgba(122, 162, 247, 0.4); }
  .hero-install-box code {
    flex: 1;
    font-family: "SF Mono", "Fira Code", "Cascadia Code", monospace;
    font-size: 0.85rem;
    color: var(--green);
    white-space: nowrap;
    overflow-x: auto;
    text-align: left;
  }
  .hero-install-tabs {
    display: flex; gap: 0; margin-bottom: -1px; position: relative; z-index: 1;
  }
  .hero-install-tab {
    background: transparent; border: 1px solid transparent; border-bottom: none;
    border-radius: 6px 6px 0 0;
    padding: 0.35rem 0.75rem;
    font-size: 0.75rem; font-weight: 600;
    color: var(--text-muted);
    cursor: pointer; transition: all 0.2s;
    font-family: inherit;
  }
  .hero-install-tab.active {
    background: rgba(26, 27, 38, 0.8);
    border-color: var(--border);
    color: var(--accent);
  }
  .hero-install-tab:not(.active):hover { color: var(--text); }
  .hero-install-cmd { display: none; }
  .hero-install-cmd.active { display: flex; }

  /* Terminal mockup */
  .terminal-mockup {
    width: 100%; max-width: 620px;
    margin-top: 3rem;
    border-radius: 10px;
    overflow: hidden;
    box-shadow: 0 4px 40px rgba(122, 162, 247, 0.12), 0 0 80px rgba(122, 162, 247, 0.05);
    animation: fadeInUp 0.6s ease 0.5s both;
    position: relative; z-index: 1;
  }
  .terminal-titlebar {
    background: #1e2030;
    padding: 0.6rem 0.85rem;
    display: flex; align-items: center; gap: 0.5rem;
    border-bottom: 1px solid var(--border);
  }
  .terminal-dots { display: flex; gap: 6px; }
  .terminal-dots span {
    width: 10px; height: 10px; border-radius: 50%;
  }
  .terminal-dots span:nth-child(1) { background: #f7768e; }
  .terminal-dots span:nth-child(2) { background: #ff9e64; }
  .terminal-dots span:nth-child(3) { background: #9ece6a; }
  .terminal-title {
    flex: 1; text-align: center;
    font-size: 0.75rem; color: var(--text-muted);
    font-family: "SF Mono", "Fira Code", "Cascadia Code", monospace;
  }
  .terminal-body {
    background: #1a1b26;
    padding: 1rem 1.1rem;
    font-family: "SF Mono", "Fira Code", "Cascadia Code", monospace;
    font-size: 0.82rem;
    line-height: 1.65;
    color: var(--text);
    text-align: left;
    overflow-x: auto;
  }
  .terminal-body .t-prompt { color: var(--green); }
  .terminal-body .t-cmd { color: var(--text-bright); }
  .terminal-body .t-flag { color: var(--orange); }
  .terminal-body .t-string { color: var(--accent); }
  .terminal-body .t-comment { color: var(--text-muted); }
  .terminal-body .t-output { color: var(--text-muted); }
  .terminal-body .t-success { color: var(--green); }
  .terminal-body .t-cursor {
    display: inline-block; width: 8px; height: 15px;
    background: var(--accent);
    animation: cursorBlink 1s step-end infinite;
    vertical-align: text-bottom;
    margin-left: 2px;
  }

  /* Mobile */
  @media (max-width: 640px) {
    nav { padding: 0 1rem; }
    .nav-links { gap: 1rem; }
    section { padding: 3.5rem 1rem; }
    .features-grid, .downloads-grid, .install-grid { grid-template-columns: 1fr; }
    .install-code code { font-size: 0.75rem; }
    .hero-actions { flex-direction: column; align-items: center; }
    .hero-orb--1 { width: 250px; height: 250px; }
    .hero-orb--2 { width: 200px; height: 200px; }
    .hero-orb--3 { width: 180px; height: 180px; }
    .terminal-mockup { margin-top: 2rem; }
    .terminal-body { font-size: 0.72rem; padding: 0.8rem; }
    .hero-strip { gap: 1rem; }
    .hero-install-box code { font-size: 0.73rem; }
  }
</style>
</head>
<body>

<nav>
  <div class="nav-brand">
    <svg width="28" height="28" viewBox="0 0 28 28" fill="none" xmlns="http://www.w3.org/2000/svg">
      <rect width="28" height="28" rx="6" fill="#24283b"/>
      <text x="14" y="18.5" text-anchor="middle" font-family="-apple-system,BlinkMacSystemFont,sans-serif" font-size="13" font-weight="700" fill="#7aa2f7">SD</text>
    </svg>
    ShellDeck
  </div>
  <div class="nav-links">
    <a href="#features">Features</a>
    <a href="#download">Download</a>
    <a href="https://github.com/benfavre/shelldeck" target="_blank" rel="noopener">GitHub</a>
  </div>
</nav>

<section class="hero">
  <div class="hero-orb hero-orb--1"></div>
  <div class="hero-orb hero-orb--2"></div>
  <div class="hero-orb hero-orb--3"></div>
  <div class="hero-content">
    <div class="hero-logo">
      <svg width="80" height="80" viewBox="0 0 80 80" fill="none" xmlns="http://www.w3.org/2000/svg">
        <rect width="80" height="80" rx="18" fill="#24283b"/>
        <rect x="2" y="2" width="76" height="76" rx="16" stroke="#292e42" stroke-width="2" fill="none"/>
        <text x="40" y="49" text-anchor="middle" font-family="-apple-system,BlinkMacSystemFont,sans-serif" font-size="32" font-weight="800" fill="#7aa2f7">SD</text>
      </svg>
    </div>
    <div class="version-badge">v${v}</div>
    <h1>ShellDeck</h1>
    <p class="tagline">A GPU-accelerated native desktop SSH &amp; terminal companion. Manage connections, forward ports, run scripts — all from one polished UI.</p>
    <div class="hero-actions">
      <a href="#download" class="btn btn-primary">
        <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor"><path d="M8 12l-4-4h2.5V3h3v5H12L8 12z"/><path d="M3 13h10v1H3z"/></svg>
        Download
      </a>
      <a href="https://github.com/benfavre/shelldeck" target="_blank" rel="noopener" class="btn btn-secondary">
        <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor"><path fill-rule="evenodd" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>
        GitHub
      </a>
    </div>
    <div class="hero-install" id="hero-install">
      <div class="hero-install-tabs">
        <button class="hero-install-tab active" data-tab="unix" onclick="switchInstallTab('unix')">Linux / macOS</button>
        <button class="hero-install-tab" data-tab="win" onclick="switchInstallTab('win')">Windows</button>
      </div>
      <div class="hero-install-cmd active" data-cmd="unix">
        <div class="hero-install-box">
          <code>curl -fsSL https://shelldeck.1clic.pro/install.sh | bash</code>
          <button class="copy-btn" onclick="copyInstall(this)" title="Copy to clipboard">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="5" y="5" width="9" height="9" rx="1.5"/><path d="M5 11H3.5A1.5 1.5 0 012 9.5v-7A1.5 1.5 0 013.5 1h7A1.5 1.5 0 0112 2.5V5"/></svg>
          </button>
        </div>
      </div>
      <div class="hero-install-cmd" data-cmd="win">
        <div class="hero-install-box">
          <code>powershell -c "irm shelldeck.1clic.pro/install.ps1 | iex"</code>
          <button class="copy-btn" onclick="copyInstall(this)" title="Copy to clipboard">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="5" y="5" width="9" height="9" rx="1.5"/><path d="M5 11H3.5A1.5 1.5 0 012 9.5v-7A1.5 1.5 0 013.5 1h7A1.5 1.5 0 0112 2.5V5"/></svg>
          </button>
        </div>
      </div>
    </div>
    <div class="hero-strip">
      <span>Open Source</span>
      <span>Cross-Platform</span>
      <span>GPU-Accelerated</span>
    </div>

  </div>
</section>

<div class="section-divider"></div>
<section id="features">
  <h2 class="section-title">Built for Power Users</h2>
  <p class="section-subtitle">Everything you need to manage remote servers and terminal sessions, in a single native app.</p>
  <div class="features-grid">
    <div class="feature-card">
      <div class="feature-icon gpu">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none"><rect x="2" y="4" width="16" height="12" rx="2" stroke="currentColor" stroke-width="1.5"/><path d="M6 10h8M6 8h5M6 12h6" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>
      </div>
      <h3>GPU-Accelerated Rendering</h3>
      <p>Built with GPUI for buttery-smooth 120fps terminal rendering. Every frame is GPU-composed for minimal latency.</p>
    </div>
    <div class="feature-card">
      <div class="feature-icon ssh">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none"><rect x="1" y="6" width="7" height="8" rx="1.5" stroke="currentColor" stroke-width="1.5"/><rect x="12" y="6" width="7" height="8" rx="1.5" stroke="currentColor" stroke-width="1.5"/><path d="M8 10h4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>
      </div>
      <h3>SSH Connection Manager</h3>
      <p>Import from ~/.ssh/config, organize hosts into groups, and connect with one click. Supports key auth and jump hosts.</p>
    </div>
    <div class="feature-card">
      <div class="feature-icon term">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none"><rect x="2" y="3" width="16" height="14" rx="2" stroke="currentColor" stroke-width="1.5"/><path d="M5 8l3 2-3 2" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/><path d="M10 13h5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>
      </div>
      <h3>Terminal Emulator</h3>
      <p>Full-featured terminal with scrollback, alt-screen, mouse support, and complete SGR color rendering via a custom VTE parser.</p>
    </div>
    <div class="feature-card">
      <div class="feature-icon port">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none"><circle cx="5" cy="10" r="2.5" stroke="currentColor" stroke-width="1.5"/><circle cx="15" cy="10" r="2.5" stroke="currentColor" stroke-width="1.5"/><path d="M7.5 10h5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-dasharray="2 2"/></svg>
      </div>
      <h3>Port Forwarding</h3>
      <p>Local, remote, and SOCKS tunnels with a visual map. See active forwards at a glance and toggle them instantly.</p>
    </div>
    <div class="feature-card">
      <div class="feature-icon script">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none"><path d="M6 3h8l3 3v11a1.5 1.5 0 01-1.5 1.5h-9A1.5 1.5 0 015 17V4.5A1.5 1.5 0 016.5 3z" stroke="currentColor" stroke-width="1.5"/><path d="M8 10l2 1.5L8 13M12 10l-2 1.5L12 13" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/></svg>
      </div>
      <h3>Script Editor</h3>
      <p>Write, save, and execute scripts on remote hosts. Stream output in real time and keep an execution history log.</p>
    </div>
    <div class="feature-card">
      <div class="feature-icon sync">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none"><path d="M3 10a7 7 0 0112.45-4.33" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/><path d="M17 10a7 7 0 01-12.45 4.33" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/><path d="M14 4l2 2-2 2" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/><path d="M6 16l-2-2 2-2" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
      </div>
      <h3>Config Sync</h3>
      <p>Watches your SSH config for changes and syncs automatically. Your own connection data stays in ShellDeck&rsquo;s config, never touching ~/.ssh.</p>
    </div>
  </div>
</section>

<div class="section-divider"></div>
<section id="download">
  <h2 class="section-title">Download ShellDeck</h2>
  <p class="section-subtitle">Available for Linux, macOS, and Windows. Free and open source.</p>
  <div class="downloads-grid">
    <div class="dl-card" data-platform="linux">
      <div class="os-icon">
        <svg width="36" height="36" viewBox="0 0 36 36" fill="none"><path d="M18 4C11 4 8 10 8 16c0 4 1 7 3 9 1.5 1.5 1 3.5 1 5h12c0-1.5-.5-3.5 1-5 2-2 3-5 3-9 0-6-3-12-10-12z" stroke="var(--text-muted)" stroke-width="1.5" fill="none"/><circle cx="14" cy="15" r="1.5" fill="var(--text-muted)"/><circle cx="22" cy="15" r="1.5" fill="var(--text-muted)"/><path d="M14 20c1 1.5 7 1.5 8 0" stroke="var(--text-muted)" stroke-width="1.2" stroke-linecap="round"/></svg>
      </div>
      <h3>Linux</h3>
      <div class="dl-meta">AppImage \u00b7 x86_64${linuxSize ? " \u00b7 " + linuxSize : ""}</div>
      <a href="${linuxUrl}" class="btn btn-primary">Download for Linux</a>
      <div class="dl-hint">Detected your platform</div>
    </div>
    <div class="dl-card" data-platform="macos">
      <div class="os-icon">
        <svg width="36" height="36" viewBox="0 0 36 36" fill="none"><path d="M25.2 18.8c-.04-3.56 2.9-5.27 3.03-5.35-1.65-2.42-4.22-2.75-5.14-2.78-2.18-.22-4.27 1.29-5.38 1.29-1.11 0-2.83-1.26-4.65-1.22-2.39.04-4.6 1.39-5.83 3.54-2.49 4.32-.64 10.72 1.79 14.22 1.18 1.72 2.6 3.64 4.46 3.57 1.79-.07 2.46-1.16 4.62-1.16 2.16 0 2.77 1.16 4.66 1.12 1.93-.03 3.15-1.75 4.32-3.47 1.36-1.98 1.92-3.9 1.96-4 0-.05-3.76-1.45-3.84-5.76z" stroke="var(--text-muted)" stroke-width="1.3" fill="none"/><path d="M22.2 8.7c.98-1.19 1.65-2.85 1.47-4.5-1.42.06-3.13.94-4.15 2.14-.91 1.06-1.71 2.74-1.5 4.36 1.59.12 3.2-.8 4.18-2z" stroke="var(--text-muted)" stroke-width="1.3" fill="none"/></svg>
      </div>
      <h3>macOS</h3>
      <div class="dl-meta">DMG \u00b7 Apple Silicon${macosSize ? " \u00b7 " + macosSize : ""}</div>
      <a href="${macosUrl}" class="btn btn-primary">Download for macOS</a>
      <div class="dl-hint">Detected your platform</div>
    </div>
    <div class="dl-card" data-platform="windows">
      <div class="os-icon">
        <svg width="36" height="36" viewBox="0 0 36 36" fill="none"><path d="M6 10.5l10-1.4v9.7H6zM17.5 9l12.5-1.7v11.5h-12.5zM6 20.2h10v9.7l-10-1.4zM17.5 20.2H30V31.7L17.5 30z" stroke="var(--text-muted)" stroke-width="1.2" fill="none"/></svg>
      </div>
      <h3>Windows</h3>
      <div class="dl-meta">Installer \u00b7 x86_64${windowsSize ? " \u00b7 " + windowsSize : ""}</div>
      <a href="${windowsUrl}" class="btn btn-primary">Download for Windows</a>
      <div class="dl-hint">Detected your platform</div>
    </div>
  </div>
</section>

<footer>
  <div class="footer-links">
    <a href="https://github.com/benfavre/shelldeck" target="_blank" rel="noopener">GitHub</a>
    <a href="${GITHUB_RELEASES}" target="_blank" rel="noopener">Releases</a>
    <span>MIT License</span>
  </div>
  <div>ShellDeck v${v}</div>
</footer>

<script>
function copyInstall(btn) {
  var code = btn.parentElement.querySelector('code').textContent;
  navigator.clipboard.writeText(code).then(function() {
    btn.classList.add('copied');
    btn.innerHTML = '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3.5 8.5 6.5 11.5 12.5 4.5"/></svg>';
    setTimeout(function() {
      btn.classList.remove('copied');
      btn.innerHTML = '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="5" y="5" width="9" height="9" rx="1.5"/><path d="M5 11H3.5A1.5 1.5 0 012 9.5v-7A1.5 1.5 0 013.5 1h7A1.5 1.5 0 0112 2.5V5"/></svg>';
    }, 2000);
  });
}
function switchInstallTab(tab) {
  document.querySelectorAll('.hero-install-tab').forEach(function(t) {
    t.classList.toggle('active', t.getAttribute('data-tab') === tab);
  });
  document.querySelectorAll('.hero-install-cmd').forEach(function(c) {
    c.classList.toggle('active', c.getAttribute('data-cmd') === tab);
  });
}
(function(){
  var ua = navigator.userAgent || "";
  var p = navigator.platform || "";
  var os = /Linux/.test(p) ? "linux" : /Mac/.test(p) ? "macos" : /Win/.test(p) ? "windows" : "";
  if (!os && /Android/.test(ua)) os = "linux";
  if (!os && /iPhone|iPad/.test(ua)) os = "macos";
  if (os) {
    var card = document.querySelector('.dl-card[data-platform="' + os + '"]');
    if (card) card.classList.add("detected");
  }
  if (os === "windows") {
    switchInstallTab("win");
  }
})();
</script>

</body>
</html>`;

  return htmlResponse(html);
}

async function renderInstallSh(env: Env): Promise<Response> {
  let version = "0.1.2";
  let linuxX86Url = "";
  let linuxX86Sha = "";
  let darwinArm64Url = "";
  let darwinArm64Sha = "";
  let darwinX86Url = "";
  let darwinX86Sha = "";

  try {
    const raw = await env.SHELLDECK_KV.get("latest-release");
    if (raw) {
      const m: UpdateManifest = JSON.parse(raw);
      version = m.version;
      const p = m.platforms;
      if (p["linux-x86_64"]) {
        linuxX86Url = p["linux-x86_64"].url;
        linuxX86Sha = p["linux-x86_64"].sha256;
      }
      if (p["darwin-aarch64"]) {
        darwinArm64Url = p["darwin-aarch64"].url;
        darwinArm64Sha = p["darwin-aarch64"].sha256;
      }
      if (p["darwin-x86_64"]) {
        darwinX86Url = p["darwin-x86_64"].url;
        darwinX86Sha = p["darwin-x86_64"].sha256;
      }
    }
  } catch {
    // use fallbacks
  }

  const gh = `https://github.com/benfavre/shelldeck/releases/download/v${version}`;
  if (!linuxX86Url) linuxX86Url = `${gh}/shelldeck-linux-x86_64.tar.gz`;
  if (!darwinArm64Url) darwinArm64Url = `${gh}/shelldeck-macos-aarch64.zip`;
  if (!darwinX86Url) darwinX86Url = `${gh}/shelldeck-macos-x86_64.zip`;

  const script = `#!/bin/bash
set -euo pipefail

# ShellDeck installer — generated dynamically
# https://shelldeck.1clic.pro

VERSION="${version}"
INSTALL_DIR="$HOME/.shelldeck/bin"

info()  { printf "\\033[0;34m==>\\033[0m %s\\n" "$1"; }
ok()    { printf "\\033[0;32m==>\\033[0m %s\\n" "$1"; }
warn()  { printf "\\033[0;33m==>\\033[0m %s\\n" "$1"; }
error() { printf "\\033[0;31merror:\\033[0m %s\\n" "$1" >&2; exit 1; }

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)  ;;
  Darwin) ;;
  *) error "Unsupported OS: $OS" ;;
esac

case "$ARCH" in
  x86_64|amd64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) error "Unsupported architecture: $ARCH" ;;
esac

DOWNLOAD_URL=""
EXPECTED_SHA256=""

if [ "$OS" = "Linux" ] && [ "$ARCH" = "x86_64" ]; then
  DOWNLOAD_URL="${linuxX86Url}"
  EXPECTED_SHA256="${linuxX86Sha}"
elif [ "$OS" = "Darwin" ] && [ "$ARCH" = "aarch64" ]; then
  DOWNLOAD_URL="${darwinArm64Url}"
  EXPECTED_SHA256="${darwinArm64Sha}"
elif [ "$OS" = "Darwin" ] && [ "$ARCH" = "x86_64" ]; then
  DOWNLOAD_URL="${darwinX86Url}"
  EXPECTED_SHA256="${darwinX86Sha}"
else
  error "No pre-built binary for $OS/$ARCH"
fi

[ -z "$DOWNLOAD_URL" ] && error "No download URL for $OS/$ARCH"

info "Installing ShellDeck v$VERSION for $OS/$ARCH..."

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

ARCHIVE="$WORK_DIR/shelldeck-archive"
info "Downloading..."
if command -v curl &>/dev/null; then
  curl -fSL --progress-bar -o "$ARCHIVE" "$DOWNLOAD_URL"
elif command -v wget &>/dev/null; then
  wget -q --show-progress -O "$ARCHIVE" "$DOWNLOAD_URL"
else
  error "curl or wget required"
fi

if [ -n "$EXPECTED_SHA256" ]; then
  info "Verifying checksum..."
  if command -v sha256sum &>/dev/null; then
    ACTUAL="$(sha256sum "$ARCHIVE" | cut -d' ' -f1)"
  elif command -v shasum &>/dev/null; then
    ACTUAL="$(shasum -a 256 "$ARCHIVE" | cut -d' ' -f1)"
  else
    warn "No sha256sum or shasum found, skipping verification"
    ACTUAL="$EXPECTED_SHA256"
  fi
  if [ "$ACTUAL" != "$EXPECTED_SHA256" ]; then
    error "Checksum mismatch (expected $EXPECTED_SHA256, got $ACTUAL)"
  fi
  ok "Checksum verified"
fi

info "Extracting..."
mkdir -p "$INSTALL_DIR"

case "$DOWNLOAD_URL" in
  *.tar.gz) tar -xzf "$ARCHIVE" -C "$WORK_DIR" ;;
  *.zip)    unzip -qo "$ARCHIVE" -d "$WORK_DIR" ;;
  *)        error "Unknown archive format" ;;
esac

BINARY="$(find "$WORK_DIR" -name 'shelldeck' -type f ! -path "$ARCHIVE" 2>/dev/null | head -1)"
if [ -z "$BINARY" ]; then
  BINARY="$(find "$WORK_DIR" -type f ! -name '*.tar.gz' ! -name '*.zip' ! -path "$ARCHIVE" 2>/dev/null | head -1)"
fi
[ -z "$BINARY" ] && error "Could not find shelldeck binary in archive"

cp "$BINARY" "$INSTALL_DIR/shelldeck"
chmod +x "$INSTALL_DIR/shelldeck"
ok "Installed to $INSTALL_DIR/shelldeck"

# Check runtime dependencies on Linux
if [ "$OS" = "Linux" ]; then
  MISSING_LIBS=""
  MISSING_PKGS=""
  if command -v ldd &>/dev/null; then
    MISSING_LIBS="$(ldd "$INSTALL_DIR/shelldeck" 2>/dev/null | grep "not found" || true)"
  fi
  if [ -n "$MISSING_LIBS" ]; then
    warn "Some system libraries are missing:"
    echo "$MISSING_LIBS" | while read -r line; do echo "    $line"; done
    echo ""
    # Map common missing libs to package names (Debian/Ubuntu)
    if command -v apt-get &>/dev/null; then
      echo "$MISSING_LIBS" | grep -q "libxkbcommon" && MISSING_PKGS="$MISSING_PKGS libxkbcommon0 libxkbcommon-x11-0"
      echo "$MISSING_LIBS" | grep -q "libwayland" && MISSING_PKGS="$MISSING_PKGS libwayland-client0"
      echo "$MISSING_LIBS" | grep -q "libvulkan" && MISSING_PKGS="$MISSING_PKGS libvulkan1"
      echo "$MISSING_LIBS" | grep -q "libfontconfig" && MISSING_PKGS="$MISSING_PKGS libfontconfig1"
      echo "$MISSING_LIBS" | grep -q "libxcb" && MISSING_PKGS="$MISSING_PKGS libxcb1 libxcb-shape0 libxcb-xfixes0"
      echo "$MISSING_LIBS" | grep -q "libssl" && MISSING_PKGS="$MISSING_PKGS libssl3"
      if [ -n "$MISSING_PKGS" ]; then
        info "Install them with:"
        echo "    sudo apt-get update && sudo apt-get install -y$MISSING_PKGS"
      fi
    elif command -v dnf &>/dev/null; then
      info "Install missing libraries with your package manager (dnf)."
    elif command -v pacman &>/dev/null; then
      info "Install missing libraries with your package manager (pacman)."
    fi
    echo ""
  fi
fi

add_to_path() {
  local rc="$1"
  if [ -f "$rc" ] && grep -qF '.shelldeck/bin' "$rc" 2>/dev/null; then return; fi
  printf '\\n# ShellDeck\\nexport PATH="$HOME/.shelldeck/bin:$PATH"\\n' >> "$rc"
  info "Added to PATH in $rc"
}

SHELL_NAME="$(basename "\${SHELL:-/bin/bash}")"
case "$SHELL_NAME" in
  zsh)  add_to_path "$HOME/.zshrc" ;;
  bash)
    [ -f "$HOME/.bashrc" ] && add_to_path "$HOME/.bashrc"
    if [ -f "$HOME/.bash_profile" ]; then add_to_path "$HOME/.bash_profile"
    elif [ -f "$HOME/.profile" ]; then add_to_path "$HOME/.profile"; fi
    ;;
  fish)
    mkdir -p "$HOME/.config/fish"
    FISH_RC="$HOME/.config/fish/config.fish"
    if ! grep -qF '.shelldeck/bin' "$FISH_RC" 2>/dev/null; then
      printf '\\n# ShellDeck\\nset -gx PATH $HOME/.shelldeck/bin $PATH\\n' >> "$FISH_RC"
      info "Added to PATH in $FISH_RC"
    fi
    ;;
  *) add_to_path "$HOME/.profile" ;;
esac

echo ""
ok "ShellDeck v$VERSION installed successfully!"
echo ""
echo "  Run 'shelldeck' to get started."
echo "  You may need to restart your shell or run:"
echo '    export PATH="$HOME/.shelldeck/bin:$PATH"'
echo ""
`;

  return textResponse(script);
}

async function renderInstallPs1(env: Env): Promise<Response> {
  let version = "0.1.2";
  let windowsUrl = "";
  let windowsSha = "";

  try {
    const raw = await env.SHELLDECK_KV.get("latest-release");
    if (raw) {
      const m: UpdateManifest = JSON.parse(raw);
      version = m.version;
      if (m.platforms["windows-x86_64"]) {
        windowsUrl = m.platforms["windows-x86_64"].url;
        windowsSha = m.platforms["windows-x86_64"].sha256;
      }
    }
  } catch {
    // use fallbacks
  }

  const gh = `https://github.com/benfavre/shelldeck/releases/download/v${version}`;
  if (!windowsUrl) windowsUrl = `${gh}/shelldeck-windows-x86_64.zip`;

  const script = `# ShellDeck installer for Windows
# https://shelldeck.1clic.pro

$ErrorActionPreference = "Stop"
$Version = "${version}"
$DownloadUrl = "${windowsUrl}"
$ExpectedHash = "${windowsSha}"
$InstallDir = "$env:LOCALAPPDATA\\ShellDeck"

Write-Host "==> Installing ShellDeck v$Version..." -ForegroundColor Blue

# Create install directory
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

# Download
$TmpFile = Join-Path ([System.IO.Path]::GetTempPath()) "shelldeck-download.zip"
Write-Host "==> Downloading..." -ForegroundColor Blue
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $TmpFile -UseBasicParsing
} catch {
    Write-Host "error: Download failed: $_" -ForegroundColor Red
    exit 1
}

# Verify checksum
if ($ExpectedHash -ne "") {
    Write-Host "==> Verifying checksum..." -ForegroundColor Blue
    $ActualHash = (Get-FileHash -Path $TmpFile -Algorithm SHA256).Hash.ToLower()
    if ($ActualHash -ne $ExpectedHash) {
        Remove-Item -Force $TmpFile -ErrorAction SilentlyContinue
        Write-Host "error: Checksum mismatch (expected $ExpectedHash, got $ActualHash)" -ForegroundColor Red
        exit 1
    }
    Write-Host "==> Checksum verified" -ForegroundColor Green
}

# Extract
Write-Host "==> Extracting..." -ForegroundColor Blue
try {
    Expand-Archive -Path $TmpFile -DestinationPath $InstallDir -Force
} catch {
    Remove-Item -Force $TmpFile -ErrorAction SilentlyContinue
    Write-Host "error: Extraction failed: $_" -ForegroundColor Red
    exit 1
}
Remove-Item -Force $TmpFile -ErrorAction SilentlyContinue

# Find binary
$Binary = Get-ChildItem -Path $InstallDir -Filter "shelldeck.exe" -Recurse -File | Select-Object -First 1
if (-not $Binary) {
    $Binary = Get-ChildItem -Path $InstallDir -Filter "*.exe" -Recurse -File | Select-Object -First 1
}
if (-not $Binary) {
    Write-Host "error: Could not find shelldeck.exe in archive" -ForegroundColor Red
    exit 1
}

# Move binary to install dir root if nested
if ($Binary.DirectoryName -ne $InstallDir) {
    Move-Item -Path $Binary.FullName -Destination (Join-Path $InstallDir "shelldeck.exe") -Force
}

# Add to PATH
$CurrentPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($CurrentPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$InstallDir;$CurrentPath", "User")
    Write-Host "==> Added $InstallDir to user PATH" -ForegroundColor Blue
}

Write-Host ""
Write-Host "==> ShellDeck v$Version installed successfully!" -ForegroundColor Green
Write-Host ""
Write-Host "  Run 'shelldeck' to get started."
Write-Host "  Restart your terminal for PATH changes to take effect."
Write-Host ""
`;

  return textResponse(script);
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    // Handle CORS preflight
    if (request.method === "OPTIONS") {
      return new Response(null, { status: 204, headers: CORS_HEADERS });
    }

    if (url.pathname === "/" || url.pathname === "") {
      return renderLandingPage(env);
    }

    if (url.pathname === "/health") {
      return textResponse("ok");
    }

    if (url.pathname === "/api/releases/latest") {
      const platform = url.searchParams.get("platform");
      if (!platform) {
        return jsonResponse({ error: "Missing 'platform' query parameter" }, 400);
      }

      const raw = await env.SHELLDECK_KV.get("latest-release");
      if (!raw) {
        return jsonResponse({ error: "No release manifest found" }, 404);
      }

      let manifest: UpdateManifest;
      try {
        manifest = JSON.parse(raw);
      } catch {
        return jsonResponse({ error: "Corrupt manifest data" }, 500);
      }

      const platformData = manifest.platforms[platform];
      if (!platformData) {
        return jsonResponse(
          { error: `No release available for platform '${platform}'` },
          404
        );
      }

      return jsonResponse({
        version: manifest.version,
        url: platformData.url,
        sha256: platformData.sha256,
        size: platformData.size,
        pub_date: manifest.pub_date,
      });
    }

    if (url.pathname === "/install.sh") {
      return renderInstallSh(env);
    }

    if (url.pathname === "/install.ps1") {
      return renderInstallPs1(env);
    }

    return textResponse("Not Found", 404);
  },
};
