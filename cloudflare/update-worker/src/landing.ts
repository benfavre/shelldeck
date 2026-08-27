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
  <meta name="theme-color" content="#fffdf9">
  <link rel="icon" href="/favicon.svg" type="image/svg+xml">
  <title>ShellDeck — Le poste de contrôle de vos agents IA</title>
  <meta name="description" content="Lancez Monique et vos agents, suivez chaque session, approuvez les actions sensibles et intervenez sans quitter ShellDeck.">
  <meta property="og:title" content="ShellDeck — Le poste de contrôle de vos agents IA">
  <meta property="og:description" content="Un cockpit natif pour déléguer, superviser et reprendre la main, du ticket jusqu’au terminal.">
  <meta property="og:image" content="https://shelldeck.1clic.pro/campaign/roles-v1/user-support-promo-desktop-poster.webp">
  <meta property="og:type" content="website">
  <style>
    @font-face{font-family:Inter;src:url('/campaign/roles-v1/inter-regular.woff2') format('woff2');font-weight:400;font-style:normal;font-display:swap}
    @font-face{font-family:Inter;src:url('/campaign/roles-v1/inter-semibold.woff2') format('woff2');font-weight:600;font-style:normal;font-display:swap}
    @font-face{font-family:Inter;src:url('/campaign/roles-v1/inter-bold.woff2') format('woff2');font-weight:700;font-style:normal;font-display:swap}
    :root{--blue:#146bff;--blue-dark:#0d42bf;--ink:#111318;--muted:#5d6470;--line:#dfe4ec;--paper:#fffdf9;--soft:#f5f7fa;--coral:#f1644a;--yellow:#ffc84a;--green:#079a74;--violet:#6d46e7;--radius:24px}
    *{box-sizing:border-box}
    html{scroll-behavior:smooth}
    body{margin:0;color:var(--ink);background:var(--paper);font-family:Inter,Arial,sans-serif;-webkit-font-smoothing:antialiased}
    a{color:inherit;text-decoration:none}
    img,svg,video{display:block;max-width:100%}
    button{font:inherit}
    .shell{width:min(1200px,calc(100% - 48px));margin-inline:auto}
    .site-header{height:72px;position:sticky;top:0;z-index:30;border-bottom:1px solid rgba(17,19,24,.08);background:rgba(255,253,249,.94);backdrop-filter:blur(16px);transition:box-shadow .28s ease,background .28s ease}
    .site-header.scrolled{background:rgba(255,253,249,.98);box-shadow:0 12px 34px rgba(17,19,24,.08)}
    .nav{height:72px;display:grid;grid-template-columns:1fr auto 1fr;align-items:center;gap:32px}
    .brand{display:inline-flex;align-items:center;gap:11px;width:max-content;font-size:19px;font-weight:700;letter-spacing:-.035em}
    .brand-mark{width:32px;height:32px;border-radius:7px}
    .wordmark b{color:var(--blue);font-weight:inherit}
    .nav-links{display:flex;align-items:center;gap:30px;color:#39404a;font-size:14px}
    .nav-links a{position:relative;padding:8px 0;transition:color .18s ease}.nav-links a::after{content:"";position:absolute;left:0;right:100%;bottom:2px;height:2px;border-radius:9px;background:var(--blue);transition:right .22s ease}.nav-links a:hover{color:var(--blue)}.nav-links a:hover::after{right:0}
    .nav-cta{justify-self:end;display:inline-flex;align-items:center;gap:8px;padding:11px 18px;border-radius:12px;background:var(--blue);color:#fff;font-size:14px;font-weight:600;box-shadow:0 7px 18px rgba(20,107,255,.18);transition:transform .2s ease,box-shadow .2s ease}
    .nav-cta:hover{transform:translateY(-2px);box-shadow:0 13px 28px rgba(20,107,255,.28)}.nav-cta:active{transform:translateY(0) scale(.98)}
    .nav-cta svg{width:15px;height:15px;transition:transform .2s ease}.nav-cta:hover svg{transform:translateY(2px)}
    .hero{padding-top:74px;overflow:hidden;background:linear-gradient(180deg,#fffdf9 0,#fffdf9 58%,#f4f9ff 100%)}
    .hero-copy{text-align:center;position:relative;z-index:2}
    .overline{display:flex;justify-content:center;align-items:center;gap:9px;margin:0 0 24px;text-transform:uppercase;color:#516071;font-size:11px;font-weight:700;letter-spacing:.12em}
    .overline::before{content:"";width:22px;height:3px;border-radius:10px;background:var(--blue)}
    h1,h2,h3,p{margin-top:0}
    h1{margin-bottom:22px;font-size:clamp(58px,6vw,86px);line-height:.97;letter-spacing:-.075em;font-weight:700}
    h1 em{position:relative;color:var(--blue);font-style:normal}
    h1 em::after{content:"";position:absolute;left:2%;right:0;bottom:-12px;height:12px;border-top:4px solid var(--yellow);border-radius:50%;transform:rotate(-1deg)}
    .hero-intro{max-width:740px;margin:0 auto 28px;color:#545c67;font-size:18px;line-height:1.58}
    .hero-actions{display:flex;align-items:center;justify-content:center;gap:24px}
    .button{display:inline-flex;align-items:center;justify-content:center;gap:11px;padding:14px 20px;border:1px solid transparent;border-radius:13px;font-size:14px;font-weight:600;transition:transform .18s ease,box-shadow .18s ease,border-color .18s ease}
    .button:hover{transform:translateY(-2px)}.button:active{transform:translateY(0) scale(.985)}
    .button-primary{background:var(--blue);color:#fff;box-shadow:0 12px 28px rgba(20,107,255,.2)}
    .button-secondary{border-color:#cbd4df;background:#fff;color:var(--ink)}
    .button-white{background:#fff;color:var(--blue-dark)}
    .text-link{display:inline-flex;align-items:center;gap:7px;padding:8px 0;border-bottom:1px solid #aab4c1;font-size:14px;font-weight:600}.text-link span{transition:transform .18s ease}.text-link:hover span{transform:translateX(4px)}
    .availability{margin:19px 0 0;color:#89919c;font-size:12px}
    .hero-stage{--rx:0deg;--ry:0deg;width:min(1180px,calc(100% - 48px));aspect-ratio:16/9;position:relative;margin:42px auto 72px;overflow:hidden;border:1px solid rgba(20,63,121,.12);border-radius:28px;background:#f7fbff url('/campaign/roles-v1/user-support-promo-desktop-poster.webp') center/cover;box-shadow:0 28px 65px rgba(24,65,112,.13);transform:perspective(1500px) rotateX(var(--rx)) rotateY(var(--ry));transition:transform .25s ease,box-shadow .25s ease;will-change:transform}
    .hero-stage:hover{box-shadow:0 38px 80px rgba(24,65,112,.19)}
    .hero-motion{position:absolute;inset:0;width:100%;height:100%;object-fit:cover}
    .mode-strip{border-block:1px solid var(--line);background:#fff}
    .modes{min-height:136px;display:grid;grid-template-columns:250px 1fr;align-items:start;gap:20px 44px;padding-block:28px}
    .modes>p{width:250px;margin:12px 0 0;font-weight:600;line-height:1.45}
    .mode-list{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));flex:1;gap:28px}
    .mode-list button{position:relative;display:flex;flex-direction:column;gap:5px;padding:12px;border:0;border-radius:12px;background:transparent;color:#77808c;text-align:left;font-size:12px;cursor:pointer;transition:transform .2s ease,background .2s ease,box-shadow .2s ease}.mode-list button:hover{transform:translateY(-3px);background:#f3f7fd}.mode-list button:focus-visible{outline:3px solid rgba(20,107,255,.24);outline-offset:3px}.mode-list button[aria-pressed="true"]{background:#edf4ff;box-shadow:inset 0 0 0 1px #c5d9f7}.mode-list button[aria-pressed="true"]::after{content:"";position:absolute;left:12px;right:12px;bottom:5px;height:2px;border-radius:8px;background:var(--blue)}
    .mode-list b{color:var(--ink);font-size:14px}
    .mode-list button[aria-pressed="true"] b{color:var(--blue-dark)}
    .mode-detail{grid-column:2;display:grid;grid-template-columns:auto auto 1fr auto;align-items:center;gap:14px;padding:14px 16px;border:1px solid #d6e1f0;border-radius:14px;background:#f8fbff}.mode-detail>span:first-child{color:#778493;font-size:10px;font-weight:700;letter-spacing:.1em;text-transform:uppercase}.mode-detail strong{font-size:13px;color:var(--blue-dark)}.mode-detail p{margin:0;color:#5f6976;font-size:12px;line-height:1.45}.mode-status{padding:6px 9px;border-radius:99px;background:#e3f7f0;color:#08795d;font-size:10px;font-weight:700;white-space:nowrap}.mode-detail.is-updating strong,.mode-detail.is-updating p,.mode-detail.is-updating .mode-status{animation:detailSwap .32s ease both}
    .ecosystem{padding-block:120px;background:#111318;color:#fff}
    .ecosystem-head{display:grid;grid-template-columns:1fr 1fr;gap:90px;align-items:end;margin-bottom:58px}
    .ecosystem-head .overline{justify-content:flex-start;color:#9bbdff;margin-bottom:18px}.ecosystem-head .overline::before{background:var(--yellow)}
    .ecosystem-head h2{margin:0;font-size:54px;line-height:1.04;letter-spacing:-.06em}.ecosystem-head>p{margin:0;color:#b7c0cc;line-height:1.7}
    .layer-grid{display:grid;grid-template-columns:repeat(3,1fr);gap:16px}
    .layer{position:relative;min-height:250px;padding:28px;border:1px solid #303743;border-radius:22px;background:#191c22;overflow:hidden;cursor:pointer;transition:transform .3s cubic-bezier(.2,.75,.25,1),border-color .3s ease,background .3s ease,box-shadow .3s ease}.layer:hover{transform:translateY(-8px);border-color:#596271;background:#1d2128}.layer:focus-visible{outline:3px solid #9bbdff;outline-offset:3px}.layer[aria-pressed="true"]{border-color:#6e9ef2;background:#202630;box-shadow:0 0 0 3px rgba(20,107,255,.13)}
    .layer::after{content:"";position:absolute;width:150px;height:150px;right:-60px;bottom:-70px;border-radius:50%;background:var(--layer-glow);filter:blur(45px);opacity:.34}
    .layer-label{display:inline-flex;padding:6px 9px;border-radius:99px;background:#262b34;color:#bfc7d2;font-size:10px;font-weight:700;letter-spacing:.11em;text-transform:uppercase}
    .layer h3{margin:54px 0 12px;color:#fff;font-size:26px;letter-spacing:-.04em}.layer p{margin:0;color:#aab3bf;font-size:14px;line-height:1.65}
    .layer-monique{--layer-glow:#8b5cf6}.layer-runtime{--layer-glow:#16c7ac}.layer-shelldeck{--layer-glow:#146bff}
    .layer-detail{display:grid;grid-template-columns:auto auto 1fr;align-items:center;gap:14px;margin-top:18px;padding:16px 20px;border:1px solid #303743;border-radius:16px;background:#15181d}.layer-detail>span{color:#8290a2;font-size:10px;font-weight:700;letter-spacing:.1em;text-transform:uppercase}.layer-detail strong{color:#fff;font-size:14px}.layer-detail p{margin:0;color:#aeb8c5;font-size:13px}.layer-detail.is-updating strong,.layer-detail.is-updating p{animation:detailSwap .35s ease both}
    .proof-strip{display:grid;grid-template-columns:repeat(4,1fr);margin-top:22px;border:1px solid #303743;border-radius:18px;overflow:hidden;background:#15181d}
    .proof-strip span{display:grid;gap:4px;padding:20px;border-right:1px solid #303743;color:#8f99a7;font-size:11px;transition:background .2s ease,color .2s ease}.proof-strip span:hover{background:#1e2229;color:#b9c2cf}.proof-strip span:last-child{border-right:0}.proof-strip b{color:#f2f5f9;font-size:13px}
    section{scroll-margin-top:72px}
    .story{padding-block:130px 150px}
    .section-head{display:grid;grid-template-columns:1fr 1fr;column-gap:100px;align-items:end;margin-bottom:80px}
    .section-head .overline{grid-column:1/-1;justify-content:flex-start;margin-bottom:18px}
    .section-head h2{margin:0;font-size:54px;line-height:1.04;letter-spacing:-.06em}
    .section-head>p:last-child{max-width:470px;margin:0;color:var(--muted);line-height:1.7}
    .journey{position:relative;padding-block:15px}
    .journey-line{position:absolute;inset:0;width:100%;height:100%;overflow:visible}
    .journey-line path{fill:none;stroke:#8bb9ff;stroke-width:2;stroke-dasharray:3 9;stroke-linecap:round}.journey.in-view .journey-line path{animation:routeFlow 12s linear infinite}
    .scene{height:500px;position:relative;display:flex;align-items:center;margin-bottom:68px}
    .scene-number{position:absolute;z-index:3;width:52px;height:52px;display:grid;place-items:center;border-radius:50%;background:var(--blue);color:#fff;font-size:13px;font-weight:700;box-shadow:0 0 0 8px var(--paper)}
    .scene-copy{position:relative;z-index:2;width:32%;padding:34px;background:rgba(255,253,249,.94)}
    .scene-label{color:var(--blue);font-size:11px;font-weight:700;letter-spacing:.12em;text-transform:uppercase}
    .scene h3{margin-bottom:16px;font-size:35px;line-height:1.08;letter-spacing:-.05em}
    .scene-copy>p:last-child{margin-bottom:0;color:var(--muted);font-size:14px;line-height:1.65}
    .app-window{margin:0;overflow:hidden;border:1px solid rgba(16,34,58,.2);border-radius:18px;background:#f8f8f8;box-shadow:0 30px 70px rgba(15,45,86,.2),0 4px 10px rgba(15,45,86,.12)}
    .app-window img{width:100%;height:auto}
    .scene-window{position:absolute;width:72%}
    .scene-user .scene-copy{margin-left:auto}.scene-user .scene-window{left:0}.scene-user .scene-number{left:calc(67% - 26px);top:64px}
    .scene-support .scene-window{right:0}.scene-support .scene-number{left:calc(28% - 26px);bottom:72px;background:var(--coral)}.scene-support .scene-copy{margin-right:auto}.scene-support{margin-bottom:0}
    .ai-section{position:relative;overflow:hidden;padding-block:125px;background:var(--blue);color:#fff}
    .ai-layout{min-height:610px;display:grid;grid-template-columns:34% 66%;align-items:center;position:relative;z-index:2}
    .ai-copy{padding-right:64px}
    .ai-copy .overline{justify-content:flex-start;color:#cfe0ff}.ai-copy .overline::before{background:var(--yellow)}
    .ai-copy h2{margin-bottom:24px;font-size:56px;line-height:1;letter-spacing:-.06em}
    .ai-copy>p:not(.overline){margin-bottom:28px;color:#dbe7ff;font-size:16px;line-height:1.7}
    .ai-window{width:860px;border-color:rgba(255,255,255,.45);box-shadow:0 34px 80px rgba(4,29,83,.42)}
    .ai-orbit{position:absolute;inset:0;pointer-events:none}
    .ai-orbit svg{position:absolute;right:-20px;top:38px;width:720px;height:520px}.ai-orbit path{fill:none;stroke:rgba(255,255,255,.22);stroke-width:3;stroke-dasharray:5 12;stroke-linecap:round}
    .bubble{position:absolute;padding:11px 15px;border-radius:16px 16px 16px 4px;background:#fff;color:#1952ad;font-size:12px;font-weight:600;box-shadow:0 10px 20px rgba(3,34,92,.18)}
    .bubble-one{right:110px;top:74px;transform:rotate(3deg);animation:bubbleFloat 5s ease-in-out infinite}.bubble-two{right:420px;bottom:58px;border-radius:16px 16px 4px 16px;transform:rotate(-3deg);animation:bubbleFloatAlt 5.8s ease-in-out infinite}
    .dev-section{padding-block:140px 105px}
    .dev-heading{display:grid;grid-template-columns:1fr 1fr;gap:100px;align-items:end;margin-bottom:58px}
    .dev-heading .overline{justify-content:flex-start;margin-bottom:18px}
    .dev-heading h2{margin:0;font-size:54px;line-height:1.04;letter-spacing:-.06em}
    .dev-heading>p{max-width:470px;margin:0;color:var(--muted);font-size:16px;line-height:1.7}
    .tool-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:20px}
    .tool-grid article{min-width:0;padding:12px 12px 24px;border:1px solid var(--line);border-radius:24px;background:#fff;box-shadow:0 18px 42px rgba(19,48,86,.07);transition:transform .28s ease,box-shadow .28s ease,border-color .28s ease}.tool-grid article:hover{transform:translateY(-7px);border-color:#aac6ee;box-shadow:0 28px 58px rgba(19,48,86,.13)}
    .tool-grid .app-window{border-radius:14px;box-shadow:0 14px 32px rgba(15,45,86,.12)}
    .tool-grid article>div{display:flex;gap:13px;align-items:flex-start;margin:20px 10px 0}
    .tool-icon{display:grid;place-items:center;flex:0 0 35px;width:35px;height:35px;border-radius:10px;background:#edf4ff;color:var(--blue)}
    .tool-icon svg{width:17px;height:17px}
    .tool-grid h3{margin:0 0 5px;font-size:17px}.tool-grid p{margin:0;color:var(--muted);font-size:13px;line-height:1.55}
    .inklura-section{padding-block:100px;background:#f0f5ff}
    .inklura-panel{display:grid;grid-template-columns:1fr 1fr;gap:75px;align-items:center;padding:58px;border:1px solid #d5e2f7;border-radius:30px;background:#fff}
    .inklura-copy .overline{justify-content:flex-start;margin-bottom:18px}.inklura-copy h2{margin-bottom:20px;font-size:46px;line-height:1.04;letter-spacing:-.055em}.inklura-copy>p{color:var(--muted);line-height:1.7}
    .inklura-actions{display:flex;flex-wrap:wrap;gap:12px;margin-top:28px}
    .flow-card{display:grid;gap:13px}
    .flow-step{display:grid;grid-template-columns:42px 1fr auto;align-items:center;gap:14px;padding:16px;border:1px solid var(--line);border-radius:16px;background:#fff;transition:transform .24s ease,border-color .24s ease}.flow-step:hover{transform:translateX(6px);border-color:#abc7ef}
    .flow-icon{display:grid;place-items:center;width:42px;height:42px;border-radius:12px;background:#edf4ff;color:var(--blue)}.flow-icon svg{width:19px;height:19px}
    .flow-step b{display:block;font-size:14px}.flow-step span{display:block;margin-top:3px;color:#7b8490;font-size:12px}.flow-arrow{color:#9aa7b8}
    .download-section{padding-block:120px;background:#f0f4f9}
    .download-head{display:flex;align-items:end;justify-content:space-between;gap:40px;margin-bottom:42px}.download-head .overline{justify-content:flex-start;margin-bottom:18px}.download-head h2{margin:0;font-size:54px;line-height:1.04;letter-spacing:-.06em}.version{color:#77808c;font:12px ui-monospace,SFMono-Regular,Menlo,monospace}
    .download-grid{display:grid;grid-template-columns:repeat(3,1fr);gap:16px}
    .download-card{display:flex;flex-direction:column;gap:18px;padding:24px;border:1px solid #d4dce6;border-radius:20px;background:#fff;transition:transform .25s ease,box-shadow .25s ease,border-color .25s ease}.download-card:hover{transform:translateY(-6px);border-color:#afc4df;box-shadow:0 20px 42px rgba(19,48,86,.11)}
    .download-card.detected{border-color:var(--blue);box-shadow:0 0 0 3px rgba(20,107,255,.1)}
    .platform{display:flex;align-items:center;justify-content:space-between;gap:14px}.platform-name{display:flex;align-items:center;gap:11px;font-size:17px}.os-icon{width:26px;height:26px;object-fit:contain}.platform-badge{display:none;padding:5px 8px;border-radius:99px;background:#eaf2ff;color:var(--blue);font-size:10px;font-weight:600}.detected .platform-badge{display:block}
    .download-meta{color:#6c7582;font-size:12px}.download-card .button{margin-top:auto}
    .install-box{display:grid;grid-template-columns:auto 1fr auto;align-items:center;gap:13px;margin-top:18px;padding:16px 18px;border:1px solid #cbd4df;border-radius:15px;background:#fff;color:#38414c}.install-box>span{color:var(--blue);font-weight:700}.install-box code{overflow:auto;font:12px ui-monospace,SFMono-Regular,Menlo,monospace}.copy{display:grid;place-items:center;width:34px;height:34px;border:0;border-radius:9px;background:#edf4ff;color:var(--blue);cursor:pointer}.copy.copied{background:#e3f7f0;color:var(--green)}
    footer{padding-block:38px;background:#111318;color:#fff}.footer-row{display:flex;align-items:center;justify-content:space-between;gap:30px}.footer-links{display:flex;gap:25px;color:#aeb7c4;font-size:12px}.credit{color:#aeb7c4;font-size:11px}.credit:hover{color:#fff}
    @keyframes routeFlow{to{stroke-dashoffset:-120}}
    @keyframes bubbleFloat{50%{transform:translateY(-10px) rotate(1deg)}}
    @keyframes bubbleFloatAlt{50%{transform:translateY(9px) rotate(-1deg)}}
    @keyframes heroRise{from{opacity:0;transform:translateY(20px)}to{opacity:1;transform:none}}
    @keyframes stageFade{from{opacity:0}to{opacity:1}}
    @keyframes detailSwap{from{opacity:.2;transform:translateY(5px)}to{opacity:1;transform:none}}
    .hero-copy>*{animation:heroRise .7s cubic-bezier(.2,.75,.25,1) both}.hero-copy .overline{animation-delay:.05s}.hero-copy h1{animation-delay:.12s}.hero-copy .hero-intro{animation-delay:.2s}.hero-copy .hero-actions{animation-delay:.28s}.hero-copy .availability{animation-delay:.34s}.hero-stage{animation:stageFade .85s .35s ease both}
    .motion-ready [data-reveal]{opacity:0;transform:translateY(28px);transition:opacity .7s cubic-bezier(.2,.75,.25,1),transform .7s cubic-bezier(.2,.75,.25,1);transition-delay:var(--reveal-delay,0ms)}
    .motion-ready [data-reveal].in-view{opacity:1;transform:none}
    .motion-ready .mode-list button.in-view:hover{transform:translateY(-3px)}.motion-ready .layer.in-view:hover{transform:translateY(-8px)}.motion-ready .tool-grid article.in-view:hover{transform:translateY(-7px)}.motion-ready .flow-step.in-view:hover{transform:translateX(6px)}.motion-ready .download-card.in-view:hover{transform:translateY(-6px)}
    @media(max-width:900px){
      .shell{width:min(100% - 28px,720px)}.nav{grid-template-columns:1fr auto}.nav-links{display:none}
      .hero{padding-top:54px}.hero-intro{font-size:16px}.hero-stage{width:calc(100% - 28px);aspect-ratio:4/5;margin:36px auto 48px;border-radius:22px;background-image:url('/campaign/roles-v1/user-support-promo-mobile-poster.webp')}
      .modes{grid-template-columns:1fr;padding:28px 0}.modes>p{width:auto;margin:0}.mode-list{width:100%;grid-template-columns:1fr 1fr;gap:18px}.mode-detail{grid-column:1;grid-template-columns:auto 1fr}.mode-detail p{grid-column:1/-1}.mode-status{justify-self:end}
      .ecosystem{padding-block:85px}.ecosystem-head{display:block}.ecosystem-head h2{margin-bottom:24px;font-size:42px}.layer-grid{grid-template-columns:1fr}.layer{min-height:220px}.layer-detail{grid-template-columns:1fr;gap:7px}.proof-strip{grid-template-columns:1fr 1fr}.proof-strip span:nth-child(2){border-right:0}.proof-strip span:nth-child(-n+2){border-bottom:1px solid #303743}
      .story{padding-block:90px}.section-head{display:block}.section-head h2{margin-bottom:24px;font-size:42px}.scene{height:auto;flex-direction:column;margin-bottom:80px}.scene-copy{width:100%;padding:0 12px 24px}.scene-user .scene-copy,.scene-support .scene-copy{margin:0}.scene-window{position:relative!important;width:100%!important;left:auto!important;right:auto!important}.scene-number,.journey-line{display:none}
      .ai-section{padding-block:85px}.ai-layout{display:flex;flex-direction:column;align-items:flex-start}.ai-copy{padding:0}.ai-copy h2{font-size:43px}.ai-window{width:920px;max-width:calc(100vw - 40px);margin-top:50px}
      .dev-section{padding-block:90px 70px}.dev-heading{display:block;margin-bottom:38px}.dev-heading h2{margin-bottom:24px;font-size:43px}.tool-grid{grid-template-columns:1fr}
      .inklura-panel{grid-template-columns:1fr;padding:36px;gap:42px}.inklura-copy h2{font-size:40px}
      .download-head{align-items:flex-start;flex-direction:column}.download-grid{grid-template-columns:1fr}.download-head h2{font-size:43px}.footer-row{align-items:flex-start;flex-direction:column}
    }
    @media(max-width:500px){
      .site-header,.nav{height:64px}.brand{font-size:17px}.brand-mark{width:28px;height:28px}.nav-cta{padding:10px 14px}
      .hero{padding-top:46px}h1{font-size:50px}.hero-intro{font-size:15px}.hero-actions{flex-direction:column;gap:10px}.button-primary{width:100%}.mode-list{grid-template-columns:1fr}
      .section-head h2,.dev-heading h2,.download-head h2,.ecosystem-head h2{font-size:38px}.scene h3{font-size:30px}.ai-copy h2,.inklura-copy h2{font-size:38px}.ai-window{width:620px;max-width:none}.ai-section{overflow:hidden}.inklura-panel{padding:28px 20px}.inklura-actions .button{width:100%}
      .install-box{grid-template-columns:auto 1fr}.install-box code{font-size:10px}.install-box .copy{grid-column:2;justify-self:end}.footer-links{flex-wrap:wrap}
    }
    @media(prefers-reduced-motion:reduce){html{scroll-behavior:auto}.button,.site-header,.nav-cta,.nav-cta svg,.text-link span,.hero-stage,.mode-list button,.layer,.proof-strip span,.tool-grid article,.flow-step,.download-card{transition:none!important}.hero-copy>*,.hero-stage,.bubble,.journey-line path,.layer-detail.is-updating strong,.layer-detail.is-updating p,.mode-detail.is-updating strong,.mode-detail.is-updating p,.mode-detail.is-updating .mode-status{animation:none!important}.motion-ready [data-reveal]{opacity:1!important;transform:none!important;transition:none!important}.hero-stage{background-image:url('/campaign/roles-v1/user-support-promo-desktop-poster.webp');transform:none!important}.hero-motion{display:none!important}}
    @media(max-width:900px) and (prefers-reduced-motion:reduce){.hero-stage{background-image:url('/campaign/roles-v1/user-support-promo-mobile-poster.webp')}}
  </style>
</head>
<body>
  <header class="site-header">
    <nav class="nav shell" aria-label="Navigation principale">
      <a class="brand" href="#top" aria-label="ShellDeck, accueil">
        <img class="brand-mark" src="/favicon.svg" width="32" height="32" alt="">
        <span class="wordmark">Shell<b>Deck</b></span>
      </a>
      <div class="nav-links">
        <a href="#plateforme">La plateforme</a><a href="#parcours">Cas d’usage</a><a href="#assistant">Monique</a><a href="#dev">Outils Dev</a>
      </div>
      <a class="nav-cta" href="#telecharger">Télécharger <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M12 3v12m0 0 4-4m-4 4-4-4"/><path d="M5 21h14"/></svg></a>
    </nav>
  </header>
  <main id="top">
    <section class="hero">
      <div class="shell hero-copy">
        <p class="overline">ShellDeck + Monique + Automonique</p>
        <h1>Vos agents travaillent.<br><em>Vous gardez la main.</em></h1>
        <p class="hero-intro">Lancez Monique ou un agent local ou SSH, suivez chaque session, approuvez les actions sensibles et passez du ticket au terminal sans changer d’outil.</p>
        <div class="hero-actions">
          <a class="button button-primary" href="#telecharger">Télécharger ShellDeck <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M12 3v12m0 0 4-4m-4 4-4-4"/><path d="M5 21h14"/></svg></a>
          <a class="text-link" href="#plateforme">Découvrir la plateforme <span aria-hidden="true">→</span></a>
        </div>
        <p class="availability">Version ${v} · Gratuit et open source · Linux, macOS et Windows</p>
      </div>
      <div class="hero-stage" aria-label="Démonstration animée : création d’une demande, passage du mode Utilisateur au mode Support et résolution assistée">
        <video class="hero-motion" autoplay muted loop playsinline preload="metadata" disablepictureinpicture poster="/campaign/roles-v1/user-support-promo-desktop-poster.webp" data-desktop-webm="/campaign/roles-v1/user-support-promo-desktop.webm" data-desktop-mp4="/campaign/roles-v1/user-support-promo-desktop.mp4" data-mobile-webm="/campaign/roles-v1/user-support-promo-mobile.webm" data-mobile-mp4="/campaign/roles-v1/user-support-promo-mobile.mp4" aria-hidden="true"></video>
      </div>
    </section>
    <section class="mode-strip" aria-label="Le parcours ShellDeck">
      <div class="shell modes">
        <p>Un même cockpit, du besoin jusqu’au résultat.</p>
        <div class="mode-list" aria-label="Explorer les étapes du parcours">
          <button type="button" aria-pressed="true" aria-controls="mode-detail" data-mode-name="Demander" data-mode-detail="Le besoin, le site et les pièces utiles restent réunis dans une demande claire." data-mode-status="Mode utilisateur"><b>01 · Demander</b>depuis un ticket ou l’application</button>
          <button type="button" aria-pressed="false" aria-controls="mode-detail" data-mode-name="Déléguer" data-mode-detail="Monique reçoit le contexte et les permissions prévues pour ce travail." data-mode-status="Mission encadrée"><b>02 · Déléguer</b>à Monique ou à l’agent choisi</button>
          <button type="button" aria-pressed="false" aria-controls="mode-detail" data-mode-name="Superviser" data-mode-detail="Budget, événements, résultat et reçu restent visibles dans la même session." data-mode-status="Session suivie"><b>03 · Superviser</b>sessions, événements et reçus</button>
          <button type="button" aria-pressed="false" aria-controls="mode-detail" data-mode-name="Intervenir" data-mode-detail="Approuvez, reprenez ou ouvrez le terminal au moment où le travail le demande." data-mode-status="Contrôle humain"><b>04 · Intervenir</b>approuver, reprendre ou ouvrir le terminal</button>
        </div>
        <div class="mode-detail" id="mode-detail" aria-live="polite" aria-atomic="true"><span>Étape active</span><strong data-mode-detail-name>Demander</strong><p data-mode-detail-copy>Le besoin, le site et les pièces utiles restent réunis dans une demande claire.</p><span class="mode-status" data-mode-detail-status>Mode utilisateur</span></div>
      </div>
    </section>
    <section class="ecosystem" id="plateforme">
      <div class="shell">
        <div class="ecosystem-head"><div><p class="overline">Une plateforme, trois responsabilités</p><h2>L’IA agit.<br>L’humain reste aux commandes.</h2></div><p>Chaque couche a un rôle clair. Monique réalise le travail, Automonique exécute et trace, ShellDeck rend les sessions visibles et contrôlables depuis le bureau.</p></div>
        <div class="layer-grid">
          <article class="layer layer-monique" tabindex="0" role="button" aria-pressed="true" data-layer-name="Monique" data-layer-detail="Le travail confié, le contexte compris et le livrable préparé."><span class="layer-label">La collègue IA</span><h3>Monique fait avancer le travail.</h3><p>Elle comprend la demande, utilise le contexte disponible, exécute les étapes et revient avec un résultat à relire.</p></article>
          <article class="layer layer-runtime" tabindex="0" role="button" aria-pressed="false" data-layer-name="Automonique" data-layer-detail="La session, les modèles, le budget, les événements et le reçu."><span class="layer-label">Le moteur souverain</span><h3>Automonique exécute et garde la trace.</h3><p>Sessions durables, modèles interchangeables, budgets, événements ordonnés et reçus restent sous le contrôle de votre infrastructure.</p></article>
          <article class="layer layer-shelldeck" tabindex="0" role="button" aria-pressed="false" data-layer-name="ShellDeck" data-layer-detail="L’état, les validations, les résultats et le point de reprise humain."><span class="layer-label">Le cockpit natif</span><h3>ShellDeck montre tout au bon moment.</h3><p>Vous observez plusieurs sessions, contrôlez une action à la fois et reprenez la main dans les outils techniques quand il le faut.</p></article>
        </div>
        <div class="layer-detail" aria-live="polite"><span>Ce que vous voyez</span><strong data-layer-detail-name>Monique</strong><p data-layer-detail-copy>Le travail confié, le contexte compris et le livrable préparé.</p></div>
        <div class="proof-strip"><span><b>Contexte partagé</b>tickets, sites et outils</span><span><b>Exécution flexible</b>locale, distante ou hébergée</span><span><b>Résultats traçables</b>événements et reçus</span><span><b>Contrôle humain</b>approbations explicites</span></div>
      </div>
    </section>
    <section class="story shell" id="parcours">
      <div class="section-head"><p class="overline">Le contexte reste attaché au travail</p><h2>Du ticket au résultat,<br>sans perdre le fil.</h2><p>ShellDeck donne à chaque rôle la bonne vue. La demande, le site, les échanges et l’intervention technique restent reliés au même travail.</p></div>
      <div class="journey">
        <svg class="journey-line" viewBox="0 0 1200 620" preserveAspectRatio="none" aria-hidden="true"><path d="M70 170 C310 20 430 310 610 300 S880 190 1120 450"/></svg>
        <article class="scene scene-user"><div class="scene-number">01</div><div class="scene-copy"><p class="scene-label">Mode Utilisateur</p><h3>Demander sans chercher le bon canal.</h3><p>Le besoin, le site concerné et les pièces utiles sont réunis dans un formulaire clair. L’IA peut aider à préparer le brouillon.</p></div><figure class="app-window scene-window"><img src="/campaign/roles-v1/user-request.webp" alt="Nouvelle demande en mode Utilisateur avec l’option Préparer avec l’IA" width="1800" height="1000" loading="lazy"></figure></article>
        <article class="scene scene-support"><div class="scene-number">02</div><div class="scene-copy"><p class="scene-label">Mode Support</p><h3>Une file pour les demandes et les tickets.</h3><p>Le Support filtre, assigne, priorise et répond dans le même contexte. Les demandes hébergées et les tickets gardent chacun leur suivi.</p></div><figure class="app-window scene-window"><img src="/campaign/roles-v1/support-request.webp" alt="Détail d’une demande dans le mode Support" width="1800" height="1000" loading="lazy"></figure></article>
      </div>
    </section>
    <section class="ai-section" id="assistant">
      <div class="ai-orbit" aria-hidden="true"><span class="bubble bubble-one">Résumer la demande</span><span class="bubble bubble-two">Préparer une réponse</span><svg viewBox="0 0 760 520"><path d="M70 400 C170 230 260 440 390 280 S610 210 700 90"/></svg></div>
      <div class="shell ai-layout"><div class="ai-copy"><p class="overline">Monique, dans le contexte</p><h2>Une collègue qui agit, pas un chatbot de plus.</h2><p>Monique peut préparer, résumer ou exécuter depuis la surface où se trouve déjà le travail. Les actions sensibles restent relues et confirmées avant leur application.</p><a class="button button-white" href="https://www.automonique.fr" target="_blank" rel="noopener">Découvrir Automonique <span aria-hidden="true">→</span></a></div><figure class="app-window ai-window"><img src="/campaign/roles-v1/ai-support.webp" alt="Monique ouverte avec le contexte d’un ticket Support" width="1800" height="1000" loading="lazy"></figure></div>
    </section>
    <section class="dev-section" id="dev">
      <div class="shell"><div class="dev-heading"><div><p class="overline">Reprendre la main</p><h2>Quand il faut intervenir,<br>les outils sont déjà là.</h2></div><p>Agents locaux ou SSH, terminaux, scripts et redirections de ports restent accessibles dans la même application, avec le contexte de la session.</p></div>
        <div class="tool-grid">
          <article><figure class="app-window"><img src="/campaign/roles-v1/dev-terminal.webp" alt="Terminal ShellDeck en mode Dev" width="1800" height="1000" loading="lazy"></figure><div><span class="tool-icon"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="m4 17 6-5-6-5M12 19h8"/></svg></span><span><h3>Terminal</h3><p>Ouvrir une session locale ou distante.</p></span></div></article>
          <article><figure class="app-window"><img src="/campaign/roles-v1/dev-scripts.webp" alt="Éditeur de scripts ShellDeck en mode Dev" width="1800" height="1000" loading="lazy"></figure><div><span class="tool-icon"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M8 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V8l-5-5h-3"/><path d="M14 3v6h6M8 13h8M8 17h5"/></svg></span><span><h3>Scripts</h3><p>Préparer et suivre les routines utiles.</p></span></div></article>
          <article><figure class="app-window"><img src="/campaign/roles-v1/dev-tunnels.webp" alt="Redirections de ports ShellDeck en mode Dev" width="1800" height="1000" loading="lazy"></figure><div><span class="tool-icon"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M8 7h8M8 17h8M5 12h14"/><path d="m16 4 3 3-3 3M8 14l-3 3 3 3"/></svg></span><span><h3>Tunnels</h3><p>Garder les redirections lisibles et contrôlables.</p></span></div></article>
        </div>
      </div>
    </section>
    <section class="inklura-section" id="inklura">
      <div class="shell inklura-panel"><div class="inklura-copy"><p class="overline">Relié à votre environnement</p><h2>Le même fil, jusqu’à la résolution.</h2><p>Sites, demandes et accès restent reliés à Inklura Manage. Automonique apporte l’exécution souveraine, Monique réalise le travail et ShellDeck garde la supervision au même endroit.</p><div class="inklura-actions"><a class="button button-primary" href="#telecharger">Utiliser ShellDeck</a><a class="button button-secondary" href="https://www.automonique.fr" target="_blank" rel="noopener">Voir Automonique <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M15 4h5v5M20 4l-9 9"/><path d="M18 13v6a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h6"/></svg></a></div></div>
        <div class="flow-card"><div class="flow-step"><span class="flow-icon"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M20 13c0 5-3.5 7-8 7a9 9 0 0 1-4-.9L3 21l1.8-4A8 8 0 1 1 20 13Z"/></svg></span><span><b>Utilisateur</b><span>Crée et suit sa demande</span></span><span class="flow-arrow">→</span></div><div class="flow-step"><span class="flow-icon"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M4 4h16v16H4zM4 9h16M9 9v11"/></svg></span><span><b>Support</b><span>Qualifie, répond et coordonne</span></span><span class="flow-arrow">→</span></div><div class="flow-step"><span class="flow-icon"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4"/><circle cx="12" cy="12" r="4"/></svg></span><span><b>Dev</b><span>Diagnostique et intervient</span></span><span class="flow-arrow">✓</span></div></div>
      </div>
    </section>
    <section class="download-section" id="telecharger">
      <div class="shell"><div class="download-head"><div><p class="overline">Prêt à l’installer ?</p><h2>ShellDeck vient<br>sur votre bureau.</h2></div><div class="version">version ${v}</div></div>
        <div class="download-grid">
          <article class="download-card" data-platform="linux"><div class="platform"><div class="platform-name"><svg class="os-icon" viewBox="0 0 24 24" fill="none" stroke="#111318" stroke-width="2" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="3"/><path d="m7 9 3 3-3 3M13 15h4"/></svg><strong>Linux</strong></div><span class="platform-badge">Votre système</span></div><div class="download-meta">AppImage · x86_64${linuxMeta}</div><a class="button button-primary" href="${linuxUrl}">Télécharger pour Linux</a></article>
          <article class="download-card" data-platform="macos"><div class="platform"><div class="platform-name"><svg class="os-icon" viewBox="0 0 24 24" fill="none" stroke="#111318" stroke-width="2" aria-hidden="true"><rect x="4" y="4" width="16" height="13" rx="2"/><path d="M2 20h20M9 20h6"/></svg><strong>macOS</strong></div><span class="platform-badge">Votre système</span></div><div class="download-meta">DMG · Apple Silicon${macosMeta}</div><a class="button button-primary" href="${macosUrl}">Télécharger pour macOS</a></article>
          <article class="download-card" data-platform="windows"><div class="platform"><div class="platform-name"><svg class="os-icon" viewBox="0 0 24 24" aria-hidden="true"><path fill="#146bff" d="M2 4.45 10.15 3.3v7.83H2V4.45Zm9.25-1.31L22 1.62v9.51H11.25V3.14ZM2 12.25h8.15v7.84L2 18.94v-6.69Zm9.25 0H22v9.51l-10.75-1.52v-7.99Z"/></svg><strong>Windows</strong></div><span class="platform-badge">Votre système</span></div><div class="download-meta">Installeur · x86_64${windowsMeta}</div><a class="button button-primary" href="${windowsUrl}">Télécharger pour Windows</a></article>
        </div>
        <div class="install-box"><span>$</span><code>curl -fsSL https://shelldeck.1clic.pro/install.sh | bash</code><button class="copy" type="button" aria-label="Copier la commande d’installation"><svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M15 9V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h3"/></svg></button></div>
      </div>
    </section>
  </main>
  <footer><div class="shell footer-row"><a class="brand" href="#top"><img class="brand-mark" src="/favicon.svg" width="28" height="28" alt=""><span class="wordmark">Shell<b>Deck</b></span></a><div class="footer-links"><a href="https://www.automonique.fr" target="_blank" rel="noopener">Automonique</a><a href="${GITHUB}" target="_blank" rel="noopener">GitHub</a><a href="${GITHUB_RELEASES}" target="_blank" rel="noopener">Versions</a><span>Licence MIT</span></div><a class="credit" href="https://webdesign29.net" target="_blank" rel="noopener">Conçu par Webdesign29</a></div></footer>
  <script>
    (function(){
      var platform=navigator.platform||'';var ua=navigator.userAgent||'';var os=/Linux/.test(platform)?'linux':/Mac/.test(platform)?'macos':/Win/.test(platform)?'windows':'';
      if(!os&&/Android/.test(ua))os='linux';if(!os&&/iPhone|iPad/.test(ua))os='macos';
      var card=os&&document.querySelector('[data-platform="'+os+'"]');if(card)card.classList.add('detected');
      var reduced=window.matchMedia('(prefers-reduced-motion: reduce)');var compact=window.matchMedia('(max-width: 900px)');var heroVideo=document.querySelector('.hero-motion');
      function syncMotion(){if(!heroVideo)return;if(reduced.matches){heroVideo.pause();heroVideo.removeAttribute('src');while(heroVideo.firstChild)heroVideo.removeChild(heroVideo.firstChild);heroVideo.load();return}var variant=compact.matches?'mobile':'desktop';if(heroVideo.dataset.variant===variant){heroVideo.play().catch(function(){});return}heroVideo.dataset.variant=variant;heroVideo.poster='/campaign/roles-v1/user-support-promo-'+variant+'-poster.webp';while(heroVideo.firstChild)heroVideo.removeChild(heroVideo.firstChild);[['Webm','video/webm'],['Mp4','video/mp4']].forEach(function(item){var source=document.createElement('source');source.src=heroVideo.dataset[variant+item[0]];source.type=item[1];heroVideo.appendChild(source)});heroVideo.load();heroVideo.play().catch(function(){})}
      if(reduced.addEventListener)reduced.addEventListener('change',syncMotion);if(compact.addEventListener)compact.addEventListener('change',syncMotion);syncMotion();
      var header=document.querySelector('.site-header');function syncHeader(){if(header)header.classList.toggle('scrolled',window.scrollY>16)}window.addEventListener('scroll',syncHeader,{passive:true});syncHeader();
      if('IntersectionObserver' in window&&!reduced.matches){
        var groups=['.mode-list button','.mode-detail','.ecosystem-head','.layer-grid .layer','.proof-strip span','.section-head','.scene','.ai-copy','.ai-window','.dev-heading','.tool-grid article','.inklura-copy','.flow-step','.download-head','.download-card','.install-box'];var reveals=[];
        try{
          groups.forEach(function(selector){document.querySelectorAll(selector).forEach(function(el,index){el.setAttribute('data-reveal','');el.style.setProperty('--reveal-delay',Math.min(index,3)*70+'ms');reveals.push(el)})});
          var journey=document.querySelector('.journey');if(journey)reveals.push(journey);
          var observer=new IntersectionObserver(function(entries){entries.forEach(function(entry){if(entry.isIntersecting){entry.target.classList.add('in-view');observer.unobserve(entry.target)}})},{threshold:.12,rootMargin:'0px 0px -7%'});
          function revealVisible(){reveals.forEach(function(el){if(el.classList.contains('in-view'))return;var box=el.getBoundingClientRect();if(box.top<window.innerHeight*1.08&&box.bottom>-80){el.classList.add('in-view');observer.unobserve(el)}})}
          reveals.forEach(function(el){observer.observe(el)});window.addEventListener('scroll',revealVisible,{passive:true});window.setTimeout(revealVisible,1400);document.documentElement.classList.add('motion-ready');requestAnimationFrame(revealVisible);
        }catch(error){document.documentElement.classList.remove('motion-ready');reveals.forEach(function(el){el.classList.add('in-view')})}
      }
      var stage=document.querySelector('.hero-stage');var fine=window.matchMedia('(pointer:fine)');if(stage&&fine.matches&&!reduced.matches){stage.addEventListener('pointermove',function(event){var box=stage.getBoundingClientRect();var x=(event.clientX-box.left)/box.width-.5;var y=(event.clientY-box.top)/box.height-.5;stage.style.setProperty('--ry',x*2.4+'deg');stage.style.setProperty('--rx',y*-2+'deg')});stage.addEventListener('pointerleave',function(){stage.style.setProperty('--ry','0deg');stage.style.setProperty('--rx','0deg')})}
      var modes=document.querySelectorAll('.mode-list button[data-mode-name]');var modeDetail=document.querySelector('.mode-detail');var modeName=document.querySelector('[data-mode-detail-name]');var modeCopy=document.querySelector('[data-mode-detail-copy]');var modeStatus=document.querySelector('[data-mode-detail-status]');function activateMode(mode){modes.forEach(function(item){item.setAttribute('aria-pressed',item===mode?'true':'false')});if(modeDetail)modeDetail.setAttribute('aria-busy','true');if(modeName)modeName.textContent=mode.dataset.modeName||'';if(modeCopy)modeCopy.textContent=mode.dataset.modeDetail||'';if(modeStatus)modeStatus.textContent=mode.dataset.modeStatus||'';if(modeDetail)modeDetail.setAttribute('aria-busy','false');if(modeDetail&&!reduced.matches){modeDetail.classList.remove('is-updating');void modeDetail.offsetWidth;modeDetail.classList.add('is-updating')}}modes.forEach(function(mode){mode.addEventListener('click',function(){activateMode(mode)})});
      var layers=document.querySelectorAll('.layer[data-layer-name]');var layerDetail=document.querySelector('.layer-detail');var layerName=document.querySelector('[data-layer-detail-name]');var layerCopy=document.querySelector('[data-layer-detail-copy]');function activateLayer(layer){layers.forEach(function(item){item.setAttribute('aria-pressed',item===layer?'true':'false')});if(layerName)layerName.textContent=layer.dataset.layerName||'';if(layerCopy)layerCopy.textContent=layer.dataset.layerDetail||'';if(layerDetail&&!reduced.matches){layerDetail.classList.remove('is-updating');void layerDetail.offsetWidth;layerDetail.classList.add('is-updating')}}layers.forEach(function(layer){layer.addEventListener('click',function(){activateLayer(layer)});layer.addEventListener('keydown',function(event){if(event.key==='Enter'||event.key===' '){event.preventDefault();activateLayer(layer)}})});
      var copy=document.querySelector('.copy');if(copy)copy.addEventListener('click',function(){var code=copy.parentElement.querySelector('code').textContent;var original=copy.innerHTML;navigator.clipboard.writeText(code).then(function(){copy.classList.add('copied');copy.setAttribute('aria-label','Commande copiée');copy.innerHTML='<svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m5 12 4 4L19 6"/></svg>';setTimeout(function(){copy.classList.remove('copied');copy.setAttribute('aria-label','Copier la commande d’installation');copy.innerHTML=original},1600)})});
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
