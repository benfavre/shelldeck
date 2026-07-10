# Tests bloqués par du design d'infra

> Cet inventaire liste les `SDTEST-…` marqués **Red** qui n'ont pas
> été écrits parce qu'ils exigent un pass de conception (fake
> transport, injectable clock, matrix CI, …) trop lourd pour être
> mêlé aux clusters de couverture par ajout de tests. Chaque bloc
> décrit ce qu'il faut construire pour débloquer les tests, une
> estimation grossière du temps de design + d'écriture, et le
> périmètre couvert derrière.

Rules générales dans [`.agents/testing.md`](../../.agents/testing.md).
Les IDs `SDUC-…` renvoient à [`USE_CASES.md`](./USE_CASES.md).

Format : chaque section titre le bloqueur, résume ce qu'il produit,
et liste les SDTEST déblo­qués.

---

## 1. SSH `FakeTransport` (session / pool / tunnel)

**Ce qu'il faut construire.** Un double du transport SSH côté `russh`
qui permet de :
- Contrôler ce que le serveur envoie (Data / ExtendedData(1) pour
  stderr / ExitStatus / Eof) sans passer par un vrai `sshd`.
- Enregistrer ce que le client pousse (pty-request, window-change,
  exec, direct-tcpip, cancel).
- Fournir un `SharedHandle` compatible avec les signatures actuelles
  de `SshSession::open_shell` / `exec` / `exec_streaming` /
  `exec_cancellable` / `disconnect`.

**Piste concrète.** Extraire un trait interne `SessionTransport`
avec les opérations que `SshSession` consomme aujourd'hui côté
`russh::client::Handle`. `russh` reste l'implémentation par défaut ;
les tests fournissent une `FakeTransport` avec des channels
`tokio::mpsc` pour piloter la conversation.

**Estimation.** ~2h de design + ~1h d'écriture des faux + ~2h de
tests = 5h. Une session dédiée.

**SDTEST débloqués** :

| ID | Fichier / cible | Priorité |
|---|---|---|
| SDTEST-520 | `session.rs::open_shell` — pty-request + window size | **P0** |
| SDTEST-521 | `session.rs::exec` — stdout + stderr + exit code | **P0** |
| SDTEST-522 | `session.rs::exec` — `success()` matche le code | P1 |
| SDTEST-523 | `session.rs::exec_streaming` — chunks avant exit | P1 |
| SDTEST-524 | `session.rs::exec_cancellable` — cancel côté client | **P0** |
| SDTEST-525 | `session.rs::resize` — propagation window-change | P1 |
| SDTEST-526 | `session.rs::open_shell::read` → None sur EOF | P2 |
| SDTEST-527 | `session.rs::disconnect` drain les événements | P1 |
| SDTEST-528 | `session.rs::new_with_jump` wire le ProxyJump | **P0** |
| SDTEST-529 | `ExecResult::stdout_string` non-utf8 no-panic | P1 |
| SDTEST-540 | `pool.rs::connect` retourne un UUID | **P0** |
| SDTEST-541 | `pool.rs::connect` réutilise la session pour la même Connection | **P0** |
| SDTEST-542 | `pool.rs::disconnect` ferme et clear `connected_ids` | **P0** |
| SDTEST-543 | `pool.rs::disconnect_all` idempotent | P1 |
| SDTEST-544 | `pool.rs::with_session`/`with_session_mut` pas de deadlock | **P0** |
| SDTEST-545 | `pool.rs::take_session`/`return_session` round-trip | P1 |
| SDTEST-546 | `pool.rs::is_connected` false après disconnect distant | P1 |
| SDTEST-560 | `tunnel.rs::validate_port` boundaries | **P0** *(pure fn, testable sans fake — pas vraiment bloqué)* |
| SDTEST-561 | `tunnel.rs::check_port_available` sur port libre / occupé | P1 |
| SDTEST-562 | `tunnel.rs::start_local_forward` bind + forward + counters | **P0** |
| SDTEST-563 | `tunnel.rs::start_local_forward` Err quand bind fail | P1 |
| SDTEST-564 | `tunnel.rs::stop` drain les connections | **P0** |
| SDTEST-565 | `tunnel.rs::start_remote_forward` routing ForwardedTcpIp | P1 |
| SDTEST-566 | `tunnel.rs::start_socks_forward` handshake SOCKS5 | **P0** |
| SDTEST-567 | `tunnel.rs::stop_all` | P1 |
| SDTEST-568 | `tunnel.rs::cleanup` retire les stopped | P2 |
| SDTEST-569 | `TunnelHandle::total_bytes` monotone | P2 |
| SDTEST-600 | `handler.rs::ClientHandler` emit Connected / Disconnected | P1 |
| SDTEST-601 | idem | P1 |
| SDTEST-602 | `handler.rs` forwarded_tcpip → rx | P1 |
| SDTEST-620 | Live sshd container (`SHELLDECK_LIVE_SSH=1`, `#[ignore]`) | P2 |

**Total débloqué** : ~24 tests P0/P1 + 6 P2.

---

## 2. `AutoUpdater` — clock / HTTP injectables

**Ce qu'il faut construire.** Deux petites abstractions :
- `trait Clock { fn now() -> Instant; async fn sleep(Duration); }`
  pour piloter la cadence de poll sans attendre 1h de vrai temps.
- `trait UpdateHttp { async fn get(url: &str) -> Result<Bytes>; }`
  pour servir des réponses canned + injecter des erreurs.

Le module `lib.rs` reste par défaut sur `tokio::time` + `reqwest`.

**Estimation.** ~1h de design + ~2h de tests = 3h. Une session.

**SDTEST débloqués** :

| ID | Cible | Priorité |
|---|---|---|
| SDTEST-1220 | `lib.rs::AutoUpdater::start_polling` — first check immediate puis hourly | **P0** |
| SDTEST-1221 | `lib.rs::AutoUpdater::set_enabled(false)` cancel le poll | **P0** |
| SDTEST-1222 | `lib.rs::ReleaseInfo` parse la JSON contract | **P0** |
| SDTEST-1223 | `lib.rs::ReleaseInfo` Err sur URL manquante | P1 |
| SDTEST-1224 | `lib.rs::AutoUpdateEvent` transitions Idle → Checking → … | P1 |
| SDTEST-1240 | `installer.rs::download_and_verify` Err sur mismatch SHA-256 | **P0** *(sécurité — release-critique)* |
| SDTEST-1241 | `installer.rs::download_and_verify` streaming (pas de buffer complet) | P1 |
| SDTEST-1244 | `installer.rs::install` fail cleanly sur archive corrompue | P1 |

**Total débloqué** : ~5 tests P0 + 3 P1.

---

## 3. CI matrix — macOS et Windows runners

**Ce qu'il faut construire.** Étendre `.github/workflows/ci.yml`
(aujourd'hui `ubuntu-latest` uniquement) à une matrix
`[ubuntu, macos, windows]` avec cache par OS. Coût cargo : ×3 en
temps d'exécution, cache réduit le linéaire au bout d'un ou deux
runs.

`release.yml` a déjà cette matrix pour build ; le levier ici est de
la propager à `cargo test`.

**Estimation.** ~30min de rédaction du yaml + ~30min de shakedown
sur les premiers runs (deps système différentes par OS —
`libdbus-1-dev` sur Linux, `openssl` via Homebrew sur macOS,
équivalent Windows). ~1h totale, plus temps de CI réel.

**SDTEST débloqués** :

| ID | Cible | Priorité |
|---|---|---|
| SDTEST-121 | `keychain.rs` — round-trip Keychain macOS | **P0** |
| SDTEST-122 | `keychain.rs` — round-trip Credential Manager Windows | **P0** |
| SDTEST-960..968 | PTY (portable_pty) — spawn / resize / echo / kill sur macOS + Windows | **P0** (déjà Green sur Linux via cluster K) |
| SDTEST-1201 | `platform.rs::macos_uses_macos_prefix_never_darwin` — s'exécute enfin sur un runner macOS | **P0** (déjà écrit, gate `cfg(target_os = "macos")`) |
| SDTEST-1202 | `platform.rs::windows_uses_windows_prefix` — idem Windows | **P0** |
| SDTEST-1242 | `installer.rs::install` — atomic replace Unix vs pending-replace Windows | **P0** |
| SDTEST-1243 | idem Windows | **P0** |

**Total débloqué** : ~10 tests P0, tous cross-platform-critiques.

---

## 4. Divers — ROI négatif ou bloqué par une décision d'impl

Ces SDTEST restent Red mais **ne devraient pas être écrits en l'état** — soit
parce que le contrat qu'ils affirment n'existe pas côté code (ce serait
un test-avant-la-feature), soit parce qu'ils enferment une décision d'impl
non tranchée. Ils sont documentés pour ne pas être ré-inventés à chaque
pass d'inventaire.

| ID | Statut | Note |
|---|---|---|
| SDTEST-508/509 | `parse_jump_spec` invalid ports / empty host | L'impl actuelle est permissive (`host:99999` → hostname `host:99999`, port 22 ; `user@:22` → hostname vide → Err). Pinner comme-est n'apporte rien de blocking ; si on décide un jour de rejeter proprement les ports > 65535 le test viendra avec le fix. |
| SDTEST-586 | `add_known_host` atomic write | Couvert en pratique par SDTEST-585 (append préserve les entries antérieurs, jamais de truncation). Rename-in-place pour power-loss = nice-to-have, jamais entré dans un incident. |
| SDTEST-964 | resize triggers SIGWINCH | portable_pty fait déjà le syscall ; vérifier la délivrance à travers un `stty size` dans un shell fils ajoute de la fragilité pour une assertion low-value. |
| SDTEST-967 | Drop `PtyMaster` kill le child | Requiert une décision : `portable_pty` ne kill pas par défaut au drop. C'est une **impl decision** (SIGKILL au drop de `LocalPty` ?) avant d'écrire le test. |
| SDTEST-035 | extract_variables skip fenced code | Green pour le moment (pin la limitation, cf. cluster F) ; un fix demanderait un mini-parser markdown, décision produit. |
| SDTEST-1080..1082 | `EditorBuffer` + syntax highlighter | La surface `file_editor` bouge encore trop (WIP i18n + tree-sitter). SDUC à créer quand elle stabilise. |

---

## 5. Comment reprendre un cluster

1. **Choisir un cluster** (SSH FakeTransport, AutoUpdater clock, CI
   matrix) — un seul par session.
2. **Faire un pass de design** (30min-1h) : nommer les traits/fakes,
   écrire un exemple de test bidon qui l'utilise, valider la forme
   avant d'écrire toute l'infra.
3. **Extraire les tests** un à un depuis ce doc, marquer chacun
   Green + description factuelle dans son fichier d'inventaire, et
   déplacer la ligne hors de ce doc.
4. **Commiter par sous-cluster** (session / pool / tunnel côté SSH,
   par exemple) plutôt qu'un mega-commit — reviewer plus facile.
5. **Mettre à jour le change log** de `USE_CASES.md` avec l'entrée
   `2026-MM-DD (X)` correspondante.

---

## 6. Ce qui a été fait pendant les clusters I → M

Les commits de cette session (161 → 295 tests) ont **volontairement
laissé de côté** tout ce qui est listé plus haut, pour maximiser le
volume à design-cost zero. Résumé :

- **Cluster I** — `known_hosts` refactoré en pure fns (14 tests
  Green, aucun russh en jeu).
- **Cluster J** — `platform.rs` + parity `include_str!` (7 tests,
  aucun clock / HTTP en jeu).
- **Cluster K** — `pty.rs` gated `#[cfg(all(test, unix))]` (4 tests
  Linux, macOS/Windows attend § 3).
- **Cluster L** — `keychain.rs` pure builders + live gated par
  `SHELLDECK_LIVE_KEYCHAIN=1` (3 pure + 3 ignored, matrix macOS/
  Windows attend § 3).
- **Cluster M** — long tail themes / ssh_config / merge / store /
  ManagedSite (12 tests).

Si vous reprenez SSH FakeTransport / AutoUpdater clock / matrix CI,
partez de ce doc, pas de l'inventaire — les priorités sont plus
faciles à voir ici que noyées dans les 4 fichiers `tests-*.md`.
