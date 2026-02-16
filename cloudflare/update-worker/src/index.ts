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
  const fallback: DownloadInfo = { version: "0.1.1" };
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
    --text-bright: #e0e6ff;
    --border: #292e42;
    --green: #9ece6a;
    --orange: #ff9e64;
    --red: #f7768e;
    --purple: #bb9af7;
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
    background: rgba(26, 27, 38, 0.85);
    backdrop-filter: blur(12px);
    border-bottom: 1px solid var(--border);
    padding: 0 2rem;
    height: 60px;
    display: flex; align-items: center; justify-content: space-between;
  }
  .nav-brand { display: flex; align-items: center; gap: 0.75rem; font-weight: 700; font-size: 1.1rem; color: var(--text-bright); }
  .nav-links { display: flex; gap: 1.5rem; align-items: center; }
  .nav-links a { color: var(--text-muted); font-size: 0.9rem; transition: color 0.2s; }
  .nav-links a:hover { color: var(--text-bright); }

  /* Hero */
  .hero {
    min-height: 100vh;
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    text-align: center;
    padding: 6rem 2rem 4rem;
    position: relative;
    overflow: hidden;
  }
  .hero::before {
    content: "";
    position: absolute; top: -50%; left: -50%; width: 200%; height: 200%;
    background: radial-gradient(ellipse at 50% 30%, rgba(122, 162, 247, 0.08) 0%, transparent 60%);
    pointer-events: none;
  }
  .hero-logo { position: relative; margin-bottom: 2rem; }
  .hero h1 {
    font-size: clamp(2.5rem, 6vw, 4rem);
    font-weight: 800;
    color: var(--text-bright);
    letter-spacing: -0.02em;
    margin-bottom: 1rem;
  }
  .hero .tagline {
    font-size: clamp(1rem, 2.5vw, 1.35rem);
    color: var(--text-muted);
    max-width: 600px;
    margin: 0 auto 2.5rem;
  }
  .hero-actions { display: flex; gap: 1rem; flex-wrap: wrap; justify-content: center; }
  .btn {
    display: inline-flex; align-items: center; gap: 0.5rem;
    padding: 0.75rem 1.75rem;
    border-radius: 8px;
    font-size: 1rem;
    font-weight: 600;
    border: none;
    cursor: pointer;
    transition: all 0.2s;
  }
  .btn-primary {
    background: var(--accent);
    color: var(--bg);
  }
  .btn-primary:hover { background: var(--accent-hover); color: var(--bg); transform: translateY(-1px); }
  .btn-secondary {
    background: transparent;
    color: var(--text);
    border: 1px solid var(--border);
  }
  .btn-secondary:hover { border-color: var(--text-muted); color: var(--text-bright); transform: translateY(-1px); }
  .version-badge {
    display: inline-block;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 20px;
    padding: 0.3rem 0.9rem;
    font-size: 0.8rem;
    color: var(--text-muted);
    margin-bottom: 1.5rem;
  }

  /* Section */
  section { padding: 5rem 2rem; max-width: 1100px; margin: 0 auto; }
  .section-title {
    text-align: center;
    font-size: 2rem;
    font-weight: 700;
    color: var(--text-bright);
    margin-bottom: 0.75rem;
  }
  .section-subtitle {
    text-align: center;
    color: var(--text-muted);
    max-width: 550px;
    margin: 0 auto 3rem;
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
    padding: 1.75rem;
    transition: border-color 0.2s, transform 0.2s;
  }
  .feature-card:hover { border-color: var(--accent); transform: translateY(-2px); }
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
    padding: 2rem;
    text-align: center;
    transition: border-color 0.2s, transform 0.2s;
  }
  .dl-card:hover { border-color: var(--accent); transform: translateY(-2px); }
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
    border-top: 1px solid var(--border);
    padding: 2rem;
    text-align: center;
    color: var(--text-muted);
    font-size: 0.85rem;
  }
  footer .footer-links { display: flex; gap: 1.5rem; justify-content: center; margin-bottom: 0.75rem; }

  /* Mobile */
  @media (max-width: 640px) {
    nav { padding: 0 1rem; }
    .nav-links { gap: 1rem; }
    section { padding: 3rem 1rem; }
    .features-grid, .downloads-grid { grid-template-columns: 1fr; }
    .hero-actions { flex-direction: column; align-items: center; }
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
</section>

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
})();
</script>

</body>
</html>`;

  return htmlResponse(html);
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

    return textResponse("Not Found", 404);
  },
};
