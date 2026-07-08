---
name: write-sdtest
description: Écrit ou met à jour des tests ShellDeck selon SDUC/SDTEST, mocks TcpListener, inventaires docs/testing. Utiliser quand on ajoute un test, une feature testable, ou qu'on met à jour docs/testing/.
---

# Écrire un test ShellDeck (SDTEST)

Lire d'abord `.agents/testing.md` (source canonique). Ce skill résume le workflow.

## Checklist avant d'écrire

1. **SDUC** — le comportement observable existe dans `docs/testing/USE_CASES.md` ?
   - Non → ajouter le prochain `SDUC-NNN` d'abord.
2. **SDTEST** — ajouter une ligne Red/Green dans le bon `docs/testing/tests-*.md`
   avec au moins un SDUC lié, statut et priorité si Red.
3. **Nommage Rust** — commentaire `// SDTEST-NNN` au-dessus, ou préfixe `sdtest_NNN_` dans le nom de fonction.

## Patterns autorisés

| Cas | Pattern |
|-----|---------|
| Client HTTP Manage/Jean/Bext | `TcpListener` dans un thread — copier `cloud_sync.rs`, `issues.rs`, etc. |
| Subprocess (`claude`, `ssh`) | trait fake (`JobExecutor`) — jamais le vrai binaire en unit test |
| Logique UI | extraire helper pur hors `Render`, tester le helper |
| Réseau live | `#[ignore]` + `SHELLDECK_LIVE=1`, entrée Yellow dans l'inventaire |

## Interdits

- Tests qui ne font qu'atteindre le mock sans assert sur le code testé
- Serde round-trip sur structs internes sans contrat wire
- `wiremock` / `mockito` — rester sur `TcpListener` std
- Tests GPUI `Render` direct

## Vérification

```bash
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig cargo test -p <crate> -- <fragment_nom_test>
grep -rn SDTEST-<id> crates/ docs/testing/
```
