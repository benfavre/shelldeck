// ShellDeck update worker (shelldeck.1clic.pro): the update API and the
// install scripts. The product site moved to https://shelldeck.bext.dev (a
// bext PRISM site that relays this API); "/" redirects there.

const SITE_URL = "https://shelldeck.bext.dev";

const FALLBACK_VERSION = "0.5.3";

export interface Env {
  SHELLDECK_KV: KVNamespace;
  ASSETS: Fetcher;
}

interface PlatformRelease {
  url: string;
  sha256: string;
  size: number;
  signature: string;
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

async function renderInstallSh(env: Env): Promise<Response> {
  let version = FALLBACK_VERSION;
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
      if (p["macos-aarch64"]) {
        darwinArm64Url = p["macos-aarch64"].url;
        darwinArm64Sha = p["macos-aarch64"].sha256;
      }
      if (p["macos-x86_64"]) {
        darwinX86Url = p["macos-x86_64"].url;
        darwinX86Sha = p["macos-x86_64"].sha256;
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
# https://shelldeck.bext.dev

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

# Register the shelldeck:// URL scheme handler so deep links open the app.
# Best-effort: writes a per-user .desktop with the MimeType and points the
# default handler at it. No-op on headless boxes without xdg tooling.
if [ "$OS" = "Linux" ]; then
  APPS_DIR="$HOME/.local/share/applications"
  mkdir -p "$APPS_DIR"
  cat > "$APPS_DIR/shelldeck.desktop" <<DESKTOP
[Desktop Entry]
Name=ShellDeck
Comment=GPU-accelerated terminal and SSH companion
Exec=$INSTALL_DIR/shelldeck %u
Icon=shelldeck
Terminal=false
Type=Application
Categories=System;TerminalEmulator;
Keywords=terminal;ssh;shell;
StartupWMClass=shelldeck
MimeType=x-scheme-handler/shelldeck;
DESKTOP
  if command -v xdg-mime &>/dev/null; then
    xdg-mime default shelldeck.desktop x-scheme-handler/shelldeck 2>/dev/null || true
  fi
  if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$APPS_DIR" 2>/dev/null || true
  fi
  info "Registered shelldeck:// deep-link handler"
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
  let version = FALLBACK_VERSION;
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
# https://shelldeck.bext.dev

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

# Register the shelldeck:// URL protocol so deep links open the app (per-user).
try {
    $Exe = Join-Path $InstallDir "shelldeck.exe"
    $Root = "HKCU:\\Software\\Classes\\shelldeck"
    New-Item -Path $Root -Force | Out-Null
    Set-ItemProperty -Path $Root -Name "(default)" -Value "URL:ShellDeck Protocol"
    Set-ItemProperty -Path $Root -Name "URL Protocol" -Value ""
    New-Item -Path "$Root\\DefaultIcon" -Force | Out-Null
    Set-ItemProperty -Path "$Root\\DefaultIcon" -Name "(default)" -Value "$Exe,0"
    New-Item -Path "$Root\\shell\\open\\command" -Force | Out-Null
    $Cmd = '"' + $Exe + '" "%1"'
    Set-ItemProperty -Path "$Root\\shell\\open\\command" -Name "(default)" -Value $Cmd
    Write-Host "==> Registered shelldeck:// deep-link handler" -ForegroundColor Blue
} catch {
    Write-Host "warn: could not register shelldeck:// handler: $_" -ForegroundColor Yellow
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
      return new Response(null, {
        status: 301,
        headers: { Location: `${SITE_URL}/`, "Cache-Control": "public, max-age=3600" },
      });
    }

    if (
      url.pathname.startsWith("/campaign/") ||
      url.pathname.startsWith("/brand/") ||
      url.pathname === "/favicon.svg"
    ) {
      return env.ASSETS.fetch(request);
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
        platform,
        version: manifest.version,
        url: platformData.url,
        sha256: platformData.sha256,
        size: platformData.size,
        pub_date: manifest.pub_date,
        signature: platformData.signature,
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
