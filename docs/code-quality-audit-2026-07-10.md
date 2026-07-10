# Code quality audit — 2026-07-10

Audit lancé sur le diff non-committé de la branche `dev` (30 fichiers, ~4400
insertions). Trois passes en parallèle : réutilisation, qualité, efficacité.

Ce fichier est un **backlog**, pas une règle. Cocher / rayer au fur et à mesure.
Chaque finding référence `file:line` pour retrouver la zone rapidement.

## High — impact perf ou bug

- [ ] **`file_editor/view.rs:2117-2149`** — le fast path `paint_glyph` /
  `GlyphCache` a été retiré ; chaque cellule alloue un `String` + shape par
  caractère. **⚠️ À revérifier avant patch** : AGENTS.md dit *l'inverse* (« The
  main grid paint loop must keep using `shape_line`; the `paint_glyph` fast
  path silently fails to render »). L'agent efficiency a peut-être mal lu la
  règle. Vérifier si la régression est réelle avant d'agir.
- [x] **`workspace/mod.rs:3775` (`refresh_bext_cloud`)** — 4 appels HTTP
  (whoami, list_sites, dashboard, list_instances) séquentiels dans un même
  bg spawn. Paralléliser (`try_join!` ou 4 threads scatter/gather).
  Impact : ~4× la latence à chaque poll 15 s tant que la vue Bext est
  visible.
  (Fan-out sur 3 `std::thread` pour whoami/sites/dashboard ; instances
  reste sériel après whoami car super-admin seulement. Client bext est
  reqwest-blocking donc pas de futures::join possible sans refonte.)
- [x] **`workspace/mod.rs:3405` + `support_view.rs:290`** —
  `SupportView` cache `account_name` / `account_email` alors que
  `AppConfig.account` est l'unique source. Violation directe de
  `.agents/session-state.md` (« Did I add a new view that holds its own
  AppConfig copy? → don't »). `is_my_issue` est aussi dupliqué byte-à-byte
  entre workspace et support_view.
  (Fields renommés `account_name_lc` / `account_email_lc`,
  pré-normalisés à `set_account` (trim+lowercase — combine H3 + M11).
  Nouveau helper `push_account_to_support` appelé depuis `apply_login`
  et `invalidate_cloud_session` — le cache ne survit plus au logout.
  La duplication du `is_my_issue` workspace-side reste — sémantique
  différente : c'est un `&self` sur `Workspace` et l'appelant User
  mode n'a pas de `SupportView` sous la main.)
- [x] **`workspace/mod.rs:3554` + `support_view.rs:3520`** — deux copies
  ~120 lignes de `render_delete_issue_modal`. Extraire un helper commun,
  ou mieux, passer à `adabraka_ui::overlays::alert_dialog::AlertDialog`
  (le primitif prévu pour les confirms destructifs — cf.
  `.agents/ui-components.md`).
  (Extrait `render_issue_delete_dialog` free fn dans `support_view.rs`
  — les deux callers passent leurs closures `on_close` / `on_confirm`.
  Migration vers adabraka `AlertDialog` reste ouverte : elle demande une
  Entity + `cx.new(...)` + gestion escape/focus — plus lourd, à faire
  quand on migre l'ensemble des confirms destructifs.)
- [x] **`workspace/mod.rs:3500` (`delete_issue_now`)** — la branche Err
  utilise `t!("toast.issue.staff_failed", …)` mais `delete_issue` est
  owner-or-staff. Un owner non-staff verra un message trompeur. Ajouter
  `toast.issue.delete_failed` dans `fr.toml` + `en.toml`.

## Medium — dette / smell

- [/] **`settings.rs:1443-1611`** — 6 `build_*_select` quasi-identiques
  (~150 lignes). Extraire un helper générique
  `bind_select<T>(cx, options, current_idx, apply)`.
  (4/6 : `build_string_field_select` couvre editor_font, terminal_font,
  terminal_cursor_style, ui_font — les 3 font selects passent de ~30 à
  ~15 lignes chacun. `build_editor_tab_size_select` (Select<usize>) et
  `build_general_language_select` (Select<UiLanguage> + `select_ui_language`
  custom on-change) restent : types/comportements différents.)
- [x] **`settings.rs:770-825`** — 6 blocs `Toggle::new(...)` copy-paste
  alors que `bind_toggle` (défini ligne 350) est le helper prévu et
  déjà utilisé dans `render_general_settings`. À réécrire pour appeler
  le helper. (5 toggles Editor + `terminal-cursor-blink` migrés.)
- [ ] **`settings.rs:277` + `workspace/mod.rs:1690`** — `sync_selects`
  rebâtit les 6 `Select` entities à chaque `sync_settings_config`, même
  quand seul le thème ou `cloud_sync` a changé. Ferme les popovers
  ouverts. Ne rebuild que le Select dont le champ backing a réellement
  changé (comparer old vs new snapshot).
- [/] **`file_editor/view.rs`** — menu contextuel hand-rolled au lieu
  d'`adabraka_ui::overlays::ContextMenu` (violation
  `.agents/ui-components.md`) + raccourcis `"Ctrl+..."` hardcodés cassés
  sur macOS (violation `.agents/cross-platform.md`). Réutiliser le
  helper de switch modifier déjà présent dans `terminal_view.rs:2254`
  (à extraire dans `crate::platform` — le pattern se duplique).
  (Modificateur → helper local `primary_modifier()` (⌘ sur macOS, Ctrl+
  ailleurs) ; à promouvoir vers `crate::platform` si un 3ᵉ caller
  arrive. Migration adabraka `ContextMenu` reste ouverte.)
- [x] **`settings.rs:1545` + :1191** — `"Bloc" / "Souligné" / "Barre"`
  et `"System Default"` hardcodés au lieu de `t!` (violation
  `.agents/i18n.md`). Ajouter les clés
  `settings.terminal.cursor_style.*` + `settings.general.font.system`.
  Note : `"System Default"` reste comme **sentinelle** stockée dans
  `AppConfig.general.ui_font_family` (compat config) — seul le label
  est traduit via `settings.general.font.system_default`.
- [x] **`file_editor/view.rs:2828-2865`** —
  `hsla(0.08, 0.7, 0.5, …)` amber + `hsla(0.0, 0.7, 0.6, 1.0)` rouge
  dans la barre d'unsaved changes. Remplacer par
  `ShellDeckColors::warning().opacity(0.2)` /
  `ShellDeckColors::error().opacity(0.4)` (violation
  `.agents/theming.md` rule 3). Le badge PDF (:3574-3575) est laissé —
  couleur sémantique pour "fichier PDF", pas un état d'erreur.
- [x] **`workspace/mod.rs`** — mutations issues (`create/comment/delete/
  staff_action`) qui ré-appellent `refresh_issues` alors que la mutation
  a déjà retourné l'issue muté. Round-trip HTTP redondant à chaque
  écriture. Splicer par id dans la liste locale et sauter le refetch.
  (Helpers `upsert_issue_in_list` / `remove_issue_from_list` — les 4
  mutations passent par eux au lieu de `refresh_issues`. Le poll 15 s
  attrape toujours la dérive sur d'autres lignes.)
- [x] **`workspace/mod.rs:2241` (`select_support_ticket`)** — appelle
  `refresh_support` immédiatement après `set_detail`, alors que le poll
  30 s couvre déjà les unread flags. Deux round-trips par sélection.
- [ ] **`file_editor/view.rs:1837` (`paint_editor`)** — 17 paramètres
  positionnels + une prépaint tuple monstrueuse (~1770-1810). Bundler
  dans un `struct PaintCtx { … }`.
- [x] **`support_view.rs:3068-3200`** — `render_issue_popover_items`
  avec un `if issues_staff` non-indenté à l'intérieur ⇒ les `items.push`
  staff-only ressemblent visuellement à du code inconditionnel. Extraire
  `staff_triage_items(...) -> Vec<PopoverMenuItem>` et
  `items.extend(...)` dans le guard.
  (Extrait en `Self::staff_triage_items(iss, id, include_dispatch,
  entity)` — la gate `!issue_instances.is_empty()` reste côté appelant
  car le helper n'a pas `&self`.)
- [x] **`support_view.rs:2828+`** — `is_my_issue` +
  `render_issue_comment` re-trim + re-lowercase le nom et l'email du
  compte à chaque issue/comment pendant le render (list de 50 issues ×
  20 comments = 1000 alloc pairs par paint). Cacher les copies
  normalisées (`account_name_lc`, `account_email_lc`) mises à jour dans
  `set_account`. À faire en même temps que le fix session-state du
  point session-state ci-dessus. (Fait avec H3.)
- [x] **`support_view.rs:700-770`** — `iso_since` / `time_of_epoch`
  fns imbriquées dans `render_issues_filter_modal` qui réimplémentent
  du Gregorian. `chrono` est déjà dep workspace. Remplacer par
  `chrono::Utc::now() - chrono::Duration::hours(h)` +
  `.format("%Y-%m-%dT%H:%M:%SZ")`. Sauf si on veut réduire le pull de
  `chrono` — dans ce cas noter le choix.
  (`time_of_epoch` supprimé, `iso_since` réduit à 2 lignes chrono.)
- [ ] **`support_view.rs:2822 + workspace/mod.rs:6180`** — chaque
  render de la liste requests alloue 3 `SharedString::from(format!(...))`
  par ligne (row id, area id, del id). Pré-construire les IDs stables
  quand la liste est set (petit struct `IssueRow { iss: Issue,
  ids: RowIds }`).
- [x] **`workspace/mod.rs:2109`** (`set_app_mode`) — reset manuel de 6
  champs de sélection issue. Chaque nouveau champ sera oublié. Extraire
  `Workspace::reset_issue_selection(cx)` (déjà répété dans
  `delete_issue_now:3496`). Note : le reset conditionnel dans
  `delete_issue_now` (2 champs, gated sur `deleted_id == selected`)
  reste à part — sémantique différente du wipe complet.
- [x] **`settings.rs:573-640`** — l'onglet Editor duplique inline le
  stepper (IconButton minus/plus + label) alors que
  `render_number_stepper` (:505-545) est le helper à appeler.

## Low — polish

- [x] **`fleet_view.rs:511` + `support_view.rs:4267`** — thin wrappers
  `fn rel_time(at_ms) { crate::i18n::rel_time(at_ms) }`. Supprimer, les
  appelants utilisent directement `crate::i18n::rel_time`.
  (Remplacés par `use crate::i18n::rel_time;` en haut du fichier — les
  call-sites restent inchangés.)
- [x] **`bext_cloud_view.rs:382`** — `t!("user.account.logout")` réutilisé
  pour un bouton **bext** disconnect. Clé sémantiquement fausse. Ajouter
  `bext_view.disconnect`.
- [x] **`workspace/mod.rs:6280-6425`** — `render_user_requests` : 145
  lignes de chaîne `div().flex()` avec 4 niveaux de nesting dans la kebab
  au hover. Extraire `render_user_request_row(iss, cx)`.
  (Row extraite en `Self::render_user_request_row(iss, cx)` — le corps
  de `render_user_requests` retombe à ~25 lignes.)
- [ ] **Commentaires narratifs** style commit-message dans plusieurs
  fichiers :
  - `workspace/mod.rs:2107-2114` (« Cross-mode selection carry-over: … »)
  - `support_view.rs:2955-2960` (« Chat-style bubble … »)
  - `settings.rs:268` (contexte historique du padding)
  Garder le *why*, retirer la narration diff.
- [x] **`core/config/issues.rs:262-296` (`list_issues`)** — 9 branches
  copy-paste `if !x.is_empty() { push!("&x={}", enc(...)) }`. Petit
  helper local `push_kv(&mut q, "status", &filter.status)` — ou éventuel
  `config::query::Builder` partagé avec `manage_sites::manage_area_url`
  et `cloud_account::browser_connect_url` qui font la même danse.
  (Extrait `push_query_kv(&mut String, &str, &str)` local — les autres
  callers pourront le reprendre s'ils bougent.)

## Non-findings (vérifiés, RAS)

- Pas de `self.app_config = config.clone()` depuis Settings (règle
  session-state respectée).
- Pas de nouveaux `unwrap()` sur chemins de faillite réaliste.
- Startup path `main.rs` : seulement 3 nouveaux slugs d'icônes, aucun
  travail bloquant ajouté.
- `Path::exists` avant open : aucun.
- Poller early-returns : les 4 pollers stoppent correctement quand la
  vue n'est pas visible.
- Caches unbounded : `issues_list`, `runtime_awaiting`, `activity`
  refresh en full-list replacement, pas de growth.
- Toggle `.muted = ShellDeckColors::selected_bg()` dans `theme.rs` :
  fix correct, bien commenté.
- Migration i18n de `variable_prompt.rs`, `workspace/{scripts,forwards,
  ssh,discovery}.rs`, `terminal_view.rs`, `status_bar.rs`,
  `bext_cloud_view.rs`, `jean_view.rs` : mécanique et correcte.
- Dashboard : `stat cards` correctement migrés à adabraka `Card`.
- `EditorConfig` (`app_config.rs`) : struct légitime + round-trip test.
- Markdown highlighter (`highlighter.rs`) : wiring légitime.
- `glyph_cache.rs` `line_height` plumb-through : clean.
