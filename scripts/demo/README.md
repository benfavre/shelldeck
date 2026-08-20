# Environnement de démonstration

Lance ShellDeck avec un profil isolé et des données entièrement inventées.
Sert à produire les captures de la documentation, et à ouvrir les surfaces
Utilisateur et Support sur une machine qui n'a pas accès au portail réel.

```bash
./scripts/demo/run.sh            # lance
./scripts/demo/run.sh --reset    # repart d'un profil neuf
```

## Ce que ça met en place

| Fichier | Rôle |
|---|---|
| `manage_stub.py` | Faux serveur Manage sur `127.0.0.1:8899` — 4 tickets avec leurs fils, 3 demandes, 1 site, tous fictifs |
| `write_config.py` | Écrit la config du profil : thème clair, français, compte fictif, IA activée |
| `profile/` | Squelette du profil — `ssh_config`, `connections.json`, invite de shell neutre |

Le profil vit dans `~/.local/share/shelldeck-demo`, **jamais** dans votre `$HOME`
réel. Aucun hôte, aucun client et aucun ticket réel n'y figure.

## Pourquoi un compte fictif est nécessaire

Sans session, ShellDeck n'affiche que l'écran de bienvenue et aucune surface
n'est atteignable. Le profil déclare donc un compte super-admin fictif, ce qui
ouvre les trois modes.

## Pourquoi un faux serveur plutôt qu'une URL injoignable

Une URL qui ne répond pas laisse bien la session ouverte — seule une erreur
d'*authentification* déconnecte, une erreur *réseau* non — mais les listes
restent vides et un bandeau d'erreur s'affiche en bas de la fenêtre. Le faux
serveur évite les deux.

## Pourquoi l'IA est activée dans le profil

Sans backend déclaré, les affordances IA disparaissent complètement : pas de
bouton **Nommer** sur le formulaire de script, pas de compositeur d'assistant.
La démonstration serait muette sur une bonne part de l'application.

Le backend configuré est `claude_cli`. Si la commande `claude` n'est pas
installée, les boutons s'affichent mais une génération échouera — c'est sans
conséquence pour les captures d'interface.

## Limites

Le serveur ne conserve rien : répondre à un ticket ou changer un statut renvoie
un succès sans effet. Il suffit pour l'affichage, pas pour éprouver un
aller-retour complet.
