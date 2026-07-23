# ShellDeck — « Companion » roadmap

> Backlog produit destiné à transformer ShellDeck d'un « power tool que
> j'ouvre » en un vrai **companion desktop** qui vit à côté du flux de
> travail : présence système, raccourcis globaux, mémoire de ce que
> j'ai fait, et bientôt une couche IA transversale à la Warp.
>
> Ce document est vivant — chaque item devient un ticket / une branche
> quand on l'attaque, et la case correspondante est cochée ici après
> merge.

## Statut au 2026-07-23

| # | Item | Statut |
|---|------|--------|
| 1 | Autostart au login | ✅ landed 2026-07-15 (`shelldeck-core::config::autostart` + Settings toggle + startup reconcile) |
| 2 | Tray icon + notifications OS | ✅ landed 2026-07-15 (4 phases : fondation, compteurs live, notifs OS delta, opt-in Settings + close_to_tray) |
| 3 | Deep links `shelldeck://` | ✅ landed 2026-07-15 (parser `deep_link` + single-instance loopback hand-off + `Workspace::open_deep_link` + OS scheme registration Linux/macOS/Windows) |
| 4 | Recent activity | ✅ landed 2026-07-15 (`shelldeck-core::config::activity` + vue Dev Activité + hooks scripts/tunnels/support/issues/Jean/sites) |
| 5 | Pin / favoris rapides | ✅ landed 2026-07-15 (connexions : persistance + sidebar + tray dynamique) |
| 6 | Onboarding first-run | ✅ landed 2026-07-15 (`onboarding_view` + `general.onboarding_completed` + replay Settings) |
| 7 | Couche IA transversale | 🚧 phases 0 a 4 livrees dans le perimetre de securite: taches, notifications, policies, triage Support explicite/automatique, activite contextuelle et diagnostics PTY sequentiels. Restent les tags bloques par l'API et la couverture GPUI dans [`ai-companion.md`](ai-companion.md). |
| 8 | AI Dock Companion | 🚧 phases A/B livrées ; phase C avancée avec création différée du `Workspace` et `AiCompanionController` autonome pour ouvrir le Dock et converser sans initialiser les vues/pollers principaux ; phase D partielle sur Windows/macOS/Linux X11, sans portail Wayland ni configuration dynamique. Le masquage à la perte de focus et le placement multi-écran sont déjà livrés. La suite du runtime léger et les finitions sont détaillées dans [`ai-dock-companion.md`](ai-dock-companion.md). |

## 1. Onboarding first-run — ✅ livré 2026-07-15

**Objectif :** après la première connexion réussie (post `LoginForm`
→ `apply_login`), présenter un tour rapide qui explique les trois
modes (User / Support / Dev pour un super-admin), les surfaces
principales (Mes sites, Mes demandes, palette, sync), et les
raccourcis clavier utiles.

**Livré :**

- Distinct du **welcome landing** pré-login (`render_welcome_screen`)
  — celui-là reste le gate pour les utilisateurs non authentifiés.
- Déclenché quand `AppConfig.general.onboarding_completed` est
  `false` (défaut) : après `apply_login` et au prochain démarrage si
  l'utilisateur était déjà connecté mais n'avait pas terminé le tour.
- Module `onboarding_view.rs` : modal multi-étapes (Welcome → Modes*
  → Surfaces → Raccourcis), skippable (Escape / Passer / ✕), clavier
  ←/→/Entrée. L'étape Modes est omise quand `!can_switch_mode()`.
- **Zone média hero** par étape (560×200) : placeholder stylé par défaut
  (marque / icône Lucide + légende i18n « Aperçu à venir ») ; un GIF ou
  WebP par étape s'active via `media_asset()` + `include_bytes` dans
  `main.rs` — pas obligatoire sur toutes les étapes.
- Pastilles de progression entre la zone média et le corps texte.
- Persistance : skip ou fin → `onboarding_completed = true` + save.
- **Settings → Général → « Revoir le guide »** relance le tour sans
  réinitialiser la config (replay explicite).

**Non fait / follow-ups :**

- Pas de `AccountInfo.first_login` côté serveur — la clé locale
  `onboarding_completed` suffit pour v1.
- Pas de spotlight / surlignage in-app des widgets réels (tour
  modal-only pour l'instant).

## 2. Tray icon / présence menu-bar — ✅ livré 2026-07-15

Livré en 4 sous-commits :

- **Phase A — fondation** : `tray-icon` + `notify-rust` + `image` en
  workspace deps, `gtk` en Linux-only dep sur `shelldeck`. Nouveau
  module `crates/shelldeck/src/tray/` avec `TrayService`. Sur Linux
  un thread dédié `shelldeck-tray` init GTK + park sur `gtk::main()`
  (adabraka-gpui ne bootstrap pas GTK, appeler le tray depuis la
  closure GPUI panic sinon). Menu statique : Ouvrir / Palette /
  Quitter, routing via `MenuEvent::set_event_handler` + mpsc canal.
- **Phase B — compteurs live** : `TrayState` snapshot + second canal
  `state_tx`. Sur Linux, `glib::timeout_add_local` inside la GTK
  loop draine le state et call `MenuItem::set_text` seulement sur
  les rows qui ont changé (dedup). 4 rows désactivées : SSH actives
  / tunnels ouverts / tickets non lus / validations Jean en attente.
  `Workspace::publish_tray_state` appelé depuis `update_dashboard_stats`,
  `refresh_support`, et chaque mutation de `runtime_awaiting`.
- **Phase C — notifs OS sur delta** : `TrayNotification` enum
  (NewTickets/JeanPending/SshDisconnected/FleetJobDone). Delta
  computed vs `last_tray_counters` (skip first publish pour éviter
  la salve au démarrage). `main.rs` wrap `notify-rust` sur un thread
  détaché pour ne jamais bloquer si le daemon de notif traîne.
- **Phase D — Settings + close_to_tray** : nouvelle section `[tray]`
  dans `AppConfig` (`close_to_tray` défaut false, 4 opts-out
  notifications défaut true). 5 nouveaux toggles dans Settings →
  Général (i18n fr + en). `close_to_tray` intercepte
  `on_window_should_close` : si tray up + opt in, `window.hide_window()`
  et retour `false` au lieu de fermer.

**Non fait dans cette livraison** (follow-ups) :

- **macOS/Windows** : sur ces plateformes, le tray se construit
  (menu statique) mais les compteurs live ne s'updatent pas — il
  faut un bridge équivalent à `glib::timeout_add_local` (respectivement
  `dispatch_async(main_queue)` et `PostMessage`+`WndProc`). `TODO` en
  place dans `spawn_tray_backend`.
- **Compteurs cliquables** pour focus sur la vue correspondante
  (SSH → Connexions, tickets → Support, etc.). Facile à ajouter,
  juste 4 variantes `TrayCommand` supplémentaires.
- **Icône template** monochrome pour macOS dark/light adaptatif.
- **Notification riche** avec l'identité de la connexion perdue au
  lieu du compte agrégé (`SshDisconnected { count }`).
- **État visuel des tâches IA** dans le tray — la notification de fin existe,
  mais aucun compteur ou indicateur de tâche active n'est rendu dans le menu.
- **i18n complète du menu tray** — le Dock réutilise les traductions FR/EN,
  mais plusieurs actions, compteurs et libellés du menu restent codés en
  français dans `tray/mod.rs`.

**Statut affiché :**

- Nombre de connexions SSH actives
- Nombre de tunnels ouverts
- Confirmations Jean en attente
- Tickets support non lus / demandes assignées

**Actions rapides livrées depuis le menu tray :**

- Ouvrir la fenêtre principale
- Ouvrir la palette de commandes
- Ouvrir une connexion épinglée
- Quitter

**Actions rapides encore prévues :**

- Rendre les quatre compteurs cliquables vers leur vue précise
- Reconnecter les sessions sauvegardées
- Se déconnecter / basculer de mode

**Notifications OS (opt-in par catégorie) :**

- Nouveau ticket support / nouvelle demande
- Job Jean qui attend une confirmation
- Connexion SSH qui tombe
- Job Fleet terminé (succès / échec)

**Techniquement :**

- Crate `tray-icon` (adabraka-gpui n'a pas de wrapper natif → cas légitime
  de dépendance nouvelle, à valider comme step 4 de la ladder `patches.md`).
- Crate `notify-rust` pour l'envoi des notifications OS.
- Icônes tray dédiées par plateforme (macOS template image noir/blanc,
  Windows/Linux couleur).
- Un `TrayService` dans le crate `shelldeck` (main) qui s'abonne aux
  événements pertinents et pousse dans le tray.

## 3. Deep links `shelldeck://` — ✅ livré 2026-07-15

**Objectif :** couture entre le desktop et les autres surfaces
(Manage, Slack, JeanClaude, e-mails). Un clic sur un lien
`shelldeck://…` ouvre ShellDeck sur la bonne vue / lance la bonne
action.

**Grammaire livrée** (parseur pur `shelldeck_core::config::deep_link`,
verbes insensibles à la casse, IDs serveur sensibles à la casse,
query/fragment/slash final ignorés) :

| Lien | Action |
|------|--------|
| `shelldeck://open/connection/<uuid>` | Focus la connexion (sans SSH) — Dev + section Connexions |
| `shelldeck://ssh/connect/<uuid>` | Ouvre + lance SSH |
| `shelldeck://tunnel/start/<uuid>` | Démarre le port-forward enregistré (par UUID) |
| `shelldeck://open/site/<id>` | Bascule sur le site + User home |
| `shelldeck://issue/<id>` | Ouvre la demande (Support si staff, sinon User) |
| `shelldeck://ticket/<id>` | Ouvre le ticket en Support (staff only) |
| `shelldeck://jean/confirm/<job_id>` | Ouvre la vue Fleet (validations en attente) |

**Livré :**

- **Parseur** `deep_link::DeepLink::parse` — pur, std-only, unit-testé
  (SDTEST-1320). UUID validés au parse ; verbe/scheme inconnus → `None`
  (le routeur no-op au lieu de deviner).
- **Single-instance + hand-off** `config::single_instance` : le primary
  bind un listener loopback TCP éphémère + écrit `instance.json`
  (`{port, token}`, 0600 sur Unix) ; un second lancement forwarde son
  URL via le socket (poignée de main token) puis quitte — jamais de
  fenêtre dupliquée. Fichier périmé (primary crashé) → repris au
  lancement suivant ; token invalide → drop. Portable (même pattern que
  `browser_connect_listen` OIDC, zéro `cfg`). Unit-testé (SDTEST-1321..1323).
- **Routeur** `Workspace::open_deep_link(link, cx)` — bascule le mode si
  besoin, résout l'ID vers la surface, toast si introuvable / non
  autorisé (tickets = staff).
- **Wiring** `main.rs` : `single_instance::acquire(arg)` au boot →
  `AlreadyRunning` (exit) ou `Primary::listen(initial)` → canal drainé
  dans une boucle GPUI (miroir de la boucle tray) qui parse + dispatche
  + `activate_window`.
- **Enregistrement OS du scheme** : `.desktop` `MimeType=x-scheme-handler/shelldeck;`
  + `Exec=shelldeck %u` (packaging + AppImage) ; enregistrement runtime
  dans `install.sh` (write `~/.local/share/applications/shelldeck.desktop`
  + `xdg-mime default` + `update-desktop-database`) ; macOS
  `CFBundleURLTypes` dans `build-dmg.sh` ; Windows `URL Protocol` dans le
  NSIS + `install.ps1` (HKCU `Software\Classes\shelldeck`).

- **Ciblage fin d'un job Jean livré 2026-07-21** :
  `jean/confirm/<job_id>` conserve l'identifiant pendant le refresh Fleet puis
  ouvre directement la sheet du job exact, y compris s'il est seulement dans
  la file locale des validations.

**Non fait / follow-ups :**

- **Cible côté Manage** : PR séparée qui ajoute les boutons « Ouvrir
  dans ShellDeck » aux endroits stratégiques (page site, ticket,
  connexion). Audit du dépôt Bext le 2026-07-23 : aucun lien
  `shelldeck://` n'y est encore présent.
- **`tunnel/<site>/<port>`** (schéma initial) non retenu — pas d'API
  pour résoudre `(site, port)` → tunnel ; on cible le `PortForward.id`.
- **`shelldeck://assistant`** n'est pas encore reconnu par le parseur ni routé
  vers le Dock.

## 4. Pin / favoris rapides — ✅ livré 2026-07-15

**Objectif :** un « fast lane » pour les éléments les plus utilisés.
Contrainte forte de Karim : **une seule catégorie d'éléments à la
fois**, pas un système universel de pin sur n'importe quoi.

**Décision :** la catégorie unique est **Connexions**.

**Livré :**

- Action pin Lucide au hover sur chaque connexion, toujours visible lorsque
  la connexion est épinglée, avec tooltip localisé.
- Sections distinctes « Épinglés » et « Autres hôtes » en haut de la liste du
  sidebar, filtrées avec la recherche et le site actif, sans doublon entre elles.
- Sous-menu tray « Connexions épinglées » alimenté dynamiquement ; un clic
  restaure ShellDeck et ouvre la connexion SSH.
- Ordre et UUID persistés dans `AppConfig.pinned_connections: Vec<Uuid>` ; les
  anciennes configurations chargent une liste vide et la suppression d'une
  connexion nettoie aussi son favori.

**Limite plateforme existante :** comme les compteurs tray, les mises à jour
live du sous-menu utilisent actuellement le bridge GTK Linux. Le canal est
cross-platform, mais le bridge de mutation du menu macOS/Windows reste le
follow-up déjà documenté dans `tray/mod.rs`.

## 5. Recent activity — ✅ livré 2026-07-15

**Objectif :** « qu'est-ce que j'ai fait récemment ? » cross-crate,
searchable, restorable. Alimente aussi le résumé IA « ma semaine ».

**Contenu à tracker :**

- Sessions terminal ouvertes (host, ordre, timestamp)
- Scripts exécutés (script, target, exit code, timestamp)
- Commandes SSH remote (commande, target, timestamp)
- Interactions support / demandes (ticket / demande, action)
- Jobs Jean lancés
- Ouvertures de sites / bascules tenant

**Techniquement :**

- `shelldeck-core::config::activity` : `ActivityEntry` / `ActivityKind`
  / `ActivityAction` + `ActivityStore`, persisté en JSONL local
  (`activity.jsonl`), newest-first au chargement, cap 500 entrées, skip
  des lignes corrompues.
- UI Dev : nouvelle vue sidebar/palette « Activité » avec recherche,
  filtres par type, timestamps relatifs, badges, et action contextuelle
  quand l'entrée porte une cible.
- Dashboard : le panneau « Activité récente » lit les mêmes entrées
  durables (top 8) au lieu d'un feed mémoire séparé.
- Hooks v1 : terminaux locaux, connexions SSH, scripts (run + exit
  code), port forwards, tickets support, demandes/issues, site actif,
  Jean `say`, validations/exécutions Fleet.
- Restore v1 : action `OpenTerminal` rouvre la surface Terminal ; les
  entrées ciblées rouvrent leur surface (connexion/script/tunnel/ticket/
  demande/site/Jean/Fleet/bext). La reprise exacte d'un ancien PTY fermé
  reste hors scope tant que le contenu de session n'est pas sérialisé.

## 6. Autostart au login (configurable) — ✅ livré 2026-07-15

**Objectif :** ShellDeck se lance automatiquement à l'ouverture de
session (opt-in via Settings → Général).

**Livré :**

- Crate `auto-launch` v0.5 en workspace dep (cross-plateforme :
  `.desktop` sur Linux, `launchd` sur macOS, `HKCU\…\Run` sur Windows).
- Nouveau champ `AppConfig.general.autostart: bool` (défaut `false`,
  `#[serde(default)]`).
- Module `shelldeck-core::config::autostart` (helper + erreurs typées).
- Toggle « Lancer au démarrage » dans Settings → Général.
- Path bidirectionnel via `SettingsEvent::AutostartRequested(bool)`
  → `Workspace::apply_autostart_request` (fait le call OS sur
  `background_executor`, ne persiste qu'en cas de succès, toast sinon).
- Réconciliation au démarrage (`main.rs`) : si le fichier config
  dit `true` mais l'OS n'a rien, on répare ; si `false` et un
  résidu traîne, on nettoie.
- i18n fr + en (`settings.general.autostart.*`, `toast.autostart.*`).
- Test unitaire (`SDTEST` à référencer) : la construction du handle
  passe ; enable/disable **non-testés** en unit (pollueraient l'autostart
  du dev).

## 7. Couche IA transversale

**Objectif :** brancher un backend une fois, puis rendre l'IA native dans
les écrans. La sheet générale reste disponible, mais les workflows principaux
passent par des boutons contextuels, des suggestions éditables, des brouillons
en attente et des actions applicables ou exécutables.

La cible inclut plusieurs niveaux d'autonomie: suggestion, préparation,
confirmation et exécution automatique bornée par capacité. Les permissions,
la cible, le risque, l'audit et l'arrêt manuel restent explicites.

Le cadrage complet, la matrice par surface, les contrats techniques et le
phasage sont dans [`docs/roadmap/ai-companion.md`](ai-companion.md).

## Non-goals explicites

- **Pas de mode invité / guest** — ShellDeck exige un compte pour
  démarrer (voir `.agents/roles.md`). Rien de ce roadmap ne
  contourne ça.
- **Pas de pin universel** — la catégorie est unique, choisie en
  amont (voir §4).
- **Pas de collecte télémétrie IA silencieuse** — une policy d'automatisation
  peut déclencher une capacité autorisée, mais elle ne permet jamais une
  collecte générale ou invisible du contenu utilisateur.
- **Pas de dépendance obligatoire à un provider IA** — la couche IA
  est opt-in ; ShellDeck reste utilisable sans backend configuré, les
  boutons IA sont simplement masqués.
