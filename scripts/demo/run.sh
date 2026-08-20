#!/usr/bin/env bash
# Lance ShellDeck dans un environnement de démonstration isolé.
#
#   ./scripts/demo/run.sh          construit si besoin, puis lance
#   ./scripts/demo/run.sh --reset  repart d'un profil neuf
#
# Rien de ce que fait cette commande ne touche votre profil réel : un HOME
# dédié est créé sous ~/.local/share/shelldeck-demo, avec ses propres
# ~/.ssh/config, connexions et réglages. Le faux serveur Manage
# (scripts/demo/manage_stub.py) fournit des tickets et des demandes inventés.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
DEMO_HOME="${SHELLDECK_DEMO_HOME:-$HOME/.local/share/shelldeck-demo}"
PORT="${SHELLDECK_DEMO_PORT:-8899}"

if [ "${1:-}" = "--reset" ]; then
  rm -rf "$DEMO_HOME"
  echo "profil de démonstration réinitialisé"
fi

mkdir -p "$DEMO_HOME/.ssh" "$DEMO_HOME/.config/shelldeck" "$DEMO_HOME/projets/boutique"
cp -n "$HERE/profile/ssh_config"       "$DEMO_HOME/.ssh/config" 2>/dev/null || true
cp -n "$HERE/profile/connections.json" "$DEMO_HOME/.config/shelldeck/connections.json" 2>/dev/null || true
cp -n "$HERE/profile/zshrc"            "$DEMO_HOME/.zshrc" 2>/dev/null || true

# Le faux Manage doit répondre avant le démarrage : sans lui, ShellDeck
# affiche une erreur de connexion au lieu des surfaces Support et Utilisateur.
if ! curl -sf "http://127.0.0.1:$PORT/api/manage/shelldeck/auth?action=whoami" >/dev/null 2>&1; then
  python3 "$HERE/manage_stub.py" --port "$PORT" >/dev/null 2>&1 &
  STUB_PID=$!
  trap 'kill "$STUB_PID" 2>/dev/null || true' EXIT
  for _ in $(seq 1 40); do
    curl -sf "http://127.0.0.1:$PORT/api/manage/shelldeck/auth?action=whoami" >/dev/null 2>&1 && break
    sleep 0.25
  done
fi

BIN="$ROOT/target/debug/shelldeck"
[ -x "$BIN" ] || (cd "$ROOT" && cargo build)

# La config est écrite à chaque lancement : elle porte le port du faux serveur
# et un compte fictif, et on ne veut pas qu'une session précédente la fige.
python3 "$HERE/write_config.py" "$DEMO_HOME" "$PORT"

echo "profil : $DEMO_HOME"
HOME="$DEMO_HOME" XDG_CONFIG_HOME="$DEMO_HOME/.config" "$BIN" "$@"
