# ShellDeck — « Companion » roadmap

> Backlog produit destiné à transformer ShellDeck d'un « power tool que
> j'ouvre » en un vrai **companion desktop** qui vit à côté du flux de
> travail : présence système, raccourcis globaux, mémoire de ce que
> j'ai fait, et bientôt une couche IA transversale à la Warp.
>
> Ce document est vivant — chaque item devient un ticket / une branche
> quand on l'attaque, et la case correspondante est cochée ici après
> merge.

## Statut au 2026-07-15

| # | Item | Statut |
|---|------|--------|
| 1 | Autostart au login | ✅ landed 2026-07-15 (`shelldeck-core::config::autostart` + Settings toggle + startup reconcile) |
| 2 | Tray icon + notifications OS | ⏳ à faire |
| 3 | Deep links `shelldeck://` | ⏳ à faire |
| 4 | Recent activity | ⏳ à faire |
| 5 | Pin / favoris rapides | ⏸ bloqué sur choix de catégorie |
| 6 | Onboarding first-run | ⏳ à faire (attendre stabilisation des surfaces) |
| 7 | Couche IA transversale | ⏳ à faire (dépend de la stabilisation des surfaces ci-dessus) |

## 1. Onboarding first-run

**Objectif :** après la première connexion réussie (post `LoginForm`
→ `apply_login`), présenter un tour rapide qui explique les trois
modes (User / Support / Dev pour un super-admin), les surfaces
principales (Mes sites, Mes demandes, palette, sync), et les
raccourcis clavier utiles.

**Portée :**

- Distinct du **welcome landing** pré-login (`render_welcome_screen`
  dans `workspace/mod.rs`) — celui-là est le gate pour les utilisateurs
  non authentifiés, il reste.
- Déclenché sur `AccountInfo.first_login: bool` (nouveau champ,
  `#[serde(default = "true")]`) ou une clé locale
  `onboarding_completed: bool` dans `AppConfig`.
- Skippable, replayable depuis Settings.

## 2. Tray icon / présence menu-bar

**Objectif :** ShellDeck vit dans le tray/menu-bar en permanence,
même fenêtre fermée. Statut vivant + actions rapides + notifications
OS pour les événements qui comptent.

**Statut affiché :**

- Nombre de connexions SSH actives
- Nombre de tunnels ouverts
- Confirmations Jean en attente
- Tickets support non lus / demandes assignées

**Actions rapides depuis le menu tray :**

- Ouvrir la fenêtre principale sur une vue précise
- Ouvrir la palette de commandes
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

## 3. Deep links `shelldeck://`

**Objectif :** couture entre le desktop et les autres surfaces
(Manage, Slack, JeanClaude, e-mails). Un clic sur un lien
`shelldeck://…` doit ouvrir ShellDeck sur la bonne vue / lancer la
bonne action.

**Schéma proposé :**

| Lien | Action |
|------|--------|
| `shelldeck://open/connection/<uuid>` | Ouvre la fenêtre + focus sur la connexion |
| `shelldeck://ssh/connect/<uuid>` | Ouvre + lance SSH |
| `shelldeck://tunnel/<site>/<port>` | Ouvre + démarre le tunnel |
| `shelldeck://open/site/<uuid>` | Bascule sur le site + affiche User home |
| `shelldeck://issue/<uuid>` | Ouvre la demande en Support |
| `shelldeck://ticket/<uuid>` | Ouvre le ticket en Support |
| `shelldeck://jean/confirm/<job_id>` | Ouvre Jean Console sur la validation |

**Techniquement :**

- Enregistrement OS du scheme (Linux `.desktop`, macOS
  `Info.plist` `CFBundleURLTypes`, Windows registry `URL Protocol`).
- Router dans `main.rs` qui parse le `Arg::from_env` et dispatche vers
  `Workspace::open_deep_link(url, cx)`.
- Cible côté Manage : PR séparée qui ajoute les boutons « Ouvrir dans
  ShellDeck » aux endroits stratégiques (page site, page ticket, page
  connexion).

## 4. Pin / favoris rapides

**Objectif :** un « fast lane » pour les éléments les plus utilisés.
Contrainte forte de Karim : **une seule catégorie d'éléments à la
fois**, pas un système universel de pin sur n'importe quoi.

**Décision restant à prendre :** quelle catégorie ?

- **Sites** — cohérent avec le titlebar site chip et le multi-tenant.
- **Connexions** — cohérent avec le sidebar (déjà l'objet le plus
  manipulé).
- **Scripts** — cohérent avec l'idée « boîte à outils personnelle ».

À trancher avant implémentation.

**UI proposée (indépendante de la catégorie choisie) :**

- Étoile / icône pin sur chaque row de la catégorie (hover-action).
- Section « Épinglés » en haut du sidebar / du menu tray.
- Persisté dans `AppConfig` (nouveau champ `pinned_<category>: Vec<Uuid>`).

## 5. Recent activity

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

- Event bus léger dans `shelldeck-core` (`ActivityEvent` +
  `ActivityStore` avec cap FIFO ~500 entrées).
- Persisté dans `~/.local/share/ShellDeck/activity.jsonl` (append-only,
  rotate à N Mo).
- UI : nouvelle vue « Recent » (sidebar item ou palette).
- Restore : bouton « Reprendre la session » sur les entrées terminal.

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

**Objectif :** ShellDeck permet de brancher **une fois** un backend
IA, et des boutons IA-powered apparaissent partout dans l'app
(intégration à la Warp).

### Deux modes de backend

- **CLI local** — `claude` (Claude Code), `codex`, `aider`, etc.
  Réutilise le pattern déjà éprouvé dans `jean_fleet::ClaudeExecutor`
  (shell-out `claude -p --output-format stream-json …`). Zéro
  gestion de clé API, utilise l'abonnement existant de l'utilisateur.
- **API keys** — Anthropic / OpenAI / autres. Clés stockées dans le
  **keychain OS** (jamais en TOML clair), via le wrapper `keychain`
  déjà présent dans `shelldeck-core`.

### Abstraction

Une trait `AiClient` unique dans `shelldeck-core::ai` :

```rust
trait AiClient: Send + Sync {
    fn complete(&self, prompt: &str, ctx: AiContext) -> Result<AiResponse>;
    fn stream(&self, prompt: &str, ctx: AiContext) -> Result<AiStream>;
}
```

Une seule implémentation active à la fois ; plusieurs configurables.
Chaque surface IA-powered passe par ce trait — pas de shell-out
direct disséminé.

### Config

- Nouveau tab **Settings → IA** (entre Éditeur et Apparence).
- Sélection backend + saisie clé API masquée.
- Toggles par surface (« activer le bouton IA dans Support », etc.)
  pour opt-out granulaire.

### Boutons IA à débloquer (liste initiale, à étendre)

**Support**

- « Proposer une réponse IA » sur les tickets — draft depuis le fil.
- « Résumer ce ticket » — long thread → 3 lignes en tête de ticket.
- « Suggérer catégorie / priorité » sur incoming tickets.

**Demandes / issues**

- « Créer une demande depuis cette erreur » depuis une sortie terminal.
- « Auto-tag / auto-priorité » sur nouvelles issues.

**Scripts**

- « Générer un script depuis instructions » (naturel → bash/python/…).
- « Expliquer ce script » avant exécution (footguns, secrets, `rm -rf`).
- « Convertir Bash ↔ Python » et autres paires de `ScriptLanguage`.
- « Reviewer avant exécution » — safety pass.

**Terminal (Warp-style copilot)**

- « Générer une commande » depuis prompt naturel (Cmd+K in-terminal).
- « Expliquer cette erreur » sur exit non-zéro.

**Jean**

- « Draft un prompt Jean depuis mon intention »
  (« rejoue X sur tous les sites Paillard » → job Jean structuré).

**Nommage auto**

- Suggérer nom pour Connection / Tunnel / Script depuis contexte.

**Recent activity summary**

- « Résumé de ma semaine » depuis le stream d'activité (voir §5).

## Non-goals explicites

- **Pas de mode invité / guest** — ShellDeck exige un compte pour
  démarrer (voir `.agents/roles.md`). Rien de ce roadmap ne
  contourne ça.
- **Pas de pin universel** — la catégorie est unique, choisie en
  amont (voir §4).
- **Pas de collecte télémétrie IA silencieuse** — chaque appel IA est
  déclenché par une action utilisateur explicite ; pas d'auto-suggest
  invisible côté serveur.
- **Pas de dépendance obligatoire à un provider IA** — la couche IA
  est opt-in ; ShellDeck reste utilisable sans backend configuré, les
  boutons IA sont simplement masqués.
