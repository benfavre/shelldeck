# ShellDeck — AI Dock Companion

> Contrat et plan de finition du Companion desktop.
>
> La priorité globale vit dans [`companion.md`](companion.md). Ce document ne
> répète plus le journal détaillé des phases terminées.

## État vérifié au 2026-07-24

Les phases A à D sont livrées :

- Dock single-instance depuis le tray et raccourci configurable ;
- démarrage caché récupérable ;
- `CompanionRoot` et `AiCompanionController` utilisables sans `Workspace` ;
- Dock et palette autonomes, multi-écran et masqués à la perte de focus ;
- backends Windows, macOS, Linux/X11 et portail XDG Wayland ;
- capture, reset, conflits et états natifs visibles dans Settings ;
- `shelldeck://assistant` via le hand-off single-instance ;
- permissions macOS documentées dans
  [`docs/macos-permissions.md`](../macos-permissions.md).

La phase E reste ouverte sur cinq finitions.

## Contrat d'expérience

- ShellDeck peut vivre dans le tray sans fenêtre principale visible.
- Un échec du tray force une fenêtre principale récupérable.
- Le Dock s'ouvre depuis le tray, le raccourci global ou
  `shelldeck://assistant`.
- Une invocation tray/raccourci bascule visibilité ; un deep link affiche de
  façon idempotente.
- Une seule fenêtre Dock existe par processus.
- Ouvrir le Dock ne construit pas `Workspace`.
- Le composer reçoit le focus ; Escape et la perte de focus masquent le Dock.
- Une requête continue lorsque le Dock est masqué.
- Ouvrir ShellDeck initialise ou réactive la surface principale unique.
- Aucun contenu externe n'est capturé automatiquement.

Sous Wayland, l'overlay ne garantit pas un niveau `always-on-top` sans
`layer-shell`. Le tray reste le chemin de récupération portable.

## Architecture actuelle

```text
CompanionRoot
├── CompanionRuntime
│   ├── tray et raccourcis globaux
│   ├── handles Dock et palette
│   └── AiCompanionController
└── Workspace optionnel
```

`CompanionRuntime` vit au niveau application et ne dépend pas de `Workspace`.
Il possède le routage du tray, des raccourcis et des fenêtres auxiliaires.

`AiCompanionController` possède l'assistant global, les conversations et les
tâches nécessaires au Dock. Une action ciblant un terminal, script ou ticket
peut demander ensuite la construction de `Workspace`.

Le chargement SSH, le store, les vues, les pollers et le Cloud Sync sont
différés jusqu'à cette première demande de surface principale.

## Configuration

```toml
[companion]
start_hidden = false
global_shortcut_enabled = true
global_palette_shortcut_enabled = true
global_shortcut = "ctrl-shift-space"
global_palette_shortcut = "ctrl-alt-space"
hide_dock_on_escape = true
hide_dock_on_focus_loss = true
always_on_top = false
```

Sur macOS, les defaults utilisent `cmd` à la place de `ctrl`. Les anciennes
configurations sans section `[companion]` restent compatibles.

## Phase E — finitions

- [x] **Accessibilité clavier et i18n FR/EN**
  - tray entièrement traduite, pluriels compris, avec mise à jour live Linux ;
  - Dock nommé au clavier et palette dotée d'un champ accessible ainsi que de
    la navigation Tab/flèches/Home/End/Page Up/Page Down.
- [x] **État des tâches IA dans le tray**
  - compteur live Linux limité aux tâches `Generating`/`Executing` ;
  - indicateur cliquable ouvrant directement l'onglet Tâches du Dock unique.
- [x] **Icône tray template macOS**
  - masque Monolith noir + alpha 36 px exporté depuis le SVG canonique ;
  - `with_icon_as_template(true)` activé uniquement dans le backend macOS.
- [ ] **Géométrie persistante**
  - restaurer l'écran et des dimensions valides ;
  - migrer proprement si l'écran sauvegardé n'existe plus.
- [ ] **Validation comportementale sur les trois OS**
  - `autostart + start_hidden` ;
  - raccourcis réels macOS/Windows ;
  - portail Wayland réel ;
  - mises à jour live du tray macOS/Windows.

## Déjà validé

- SDTEST-1380/1381 : menu tray et fenêtre single-instance ;
- SDTEST-1382/1383/1391 : config rétrocompatible et démarrage récupérable ;
- SDTEST-1393/1398/1400..1404 : routage, persistance et résultats des
  raccourcis ;
- SDTEST-1397 : conversion et retours du portail Wayland ;
- SDTEST-1405 : deep link Assistant idempotent ;
- SDTEST-1300/1302/1406 : traductions tray et navigation clavier bornée ;
- SDTEST-1407..1409 : comptage des tâches IA et ouverture du centre ;
- SDTEST-1410 : dimensions, monochromie, transparence et couverture du masque
  tray macOS ;
- smoke Linux : Dock seul sans initialisation de `Workspace` ;
- benchmark Linux debug : démarrage caché environ 28 % plus rapide, RSS
  pratiquement inchangé.

Les preuves exhaustives et les lacunes de harnais GPUI restent dans
[`docs/testing/`](../testing/).

## Limites connues

- aucun test d'enregistrement global réel sur macOS/Windows ;
- aucun smoke du portail sur la machine X11 actuelle ;
- pas de garantie `layer-shell` sous Wayland ;
- mutations live du menu tray non câblées sur macOS/Windows ;

## Hors scope V1

- observer la sélection ou le presse-papiers d'autres applications ;
- plusieurs fenêtres Assistant simultanées ;
- exécuter directement une commande depuis une réponse libre ;
- remplacer le tray par un daemon privilégié ;
- garantir un ancrage sous l'icône tray sur tous les environnements desktop.
