#!/usr/bin/env python3
"""Écrit la config du profil de démonstration.

ShellDeck refuse une config partielle : on part donc de celle qu'il a générée
lui-même au premier lancement et on n'y injecte que ce qui rend la
démonstration possible — le thème clair, le français, un compte fictif et le
faux serveur Manage.

Le compte est indispensable : sans session, ShellDeck n'affiche que l'écran de
bienvenue et aucune surface n'est visible.
"""
import re
import subprocess
import sys
from pathlib import Path

home, port = Path(sys.argv[1]), sys.argv[2]
cfg = home / ".config" / "shelldeck" / "config.toml"

if not cfg.exists():
    # Premier lancement : laisser l'application écrire ses défauts, puis
    # la refermer. C'est le seul moyen d'obtenir un fichier complet et valide.
    root = Path(__file__).resolve().parents[2]
    binary = root / "target" / "debug" / "shelldeck"
    env = {"HOME": str(home), "XDG_CONFIG_HOME": str(home / ".config"),
           "PATH": "/usr/bin:/bin", "DISPLAY": ":0"}
    proc = subprocess.Popen([str(binary)], env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(60):
        if cfg.exists():
            break
        subprocess.run(["sleep", "0.5"])
    proc.terminate()
    proc.wait(timeout=10)

s = cfg.read_text()
s = re.sub(r'^theme = ".*"$', 'theme = "Light"', s, count=1, flags=re.M)
s = re.sub(r'^ui_language = ".*"$', 'ui_language = "fr"', s, count=1, flags=re.M)

if "[account]" not in s:
    s = s.rstrip("\n") + """

[account]
email = "support@exemple.test"
name = "Équipe de démonstration"
is_superadmin = true
is_admin = true
is_inklura_support = true
roles = ["superadmin", "inklura_support"]
"""

if "[ai]" not in s:
    # Sans backend déclaré, les affordances IA (bouton « Nommer », compositeur
    # de l'assistant) n'apparaissent pas du tout : la démonstration serait
    # muette sur une bonne part de l'application.
    s = s.rstrip("\n") + """

[ai]
enabled = true
backend = "claude_cli"
model = ""

[ai.surfaces]
support = true
issues = true
scripts = true
terminal = true
monique = true
naming = true
recent = true
"""

base = f'"http://127.0.0.1:{port}"'
if "[cloud_sync]" in s:
    def patch(block):
        b = block.group(1)
        b = re.sub(r"^enabled = .*$", "enabled = true", b, flags=re.M)
        b = re.sub(r"^base_url = .*$", f"base_url = {base}", b, flags=re.M)
        b = re.sub(r"^token = .*$", 'token = "demo-token"', b, flags=re.M)
        return b
    s = re.sub(r"(\[cloud_sync\][^\[]*)", patch, s, count=1)
else:
    s += f'\n[cloud_sync]\nenabled = true\nbase_url = {base}\ntoken = "demo-token"\nsync_on_startup = true\nmode = "Support"\n'

cfg.write_text(s)
print(f"config écrite : {cfg}")
