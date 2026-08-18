# Registre fiabilité et tests

Ce registre transforme les inventaires `docs/testing/tests-*.md` en chantiers
actionnables. Il ne remplace pas ces inventaires exhaustifs : il regroupe les
risques par frontière observable et impose une vérification avant correction.

## Statuts

- **Ouvert** : risque confirmé et preuve manquante réalisable maintenant.
- **En cours** : une partie de la frontière est prouvée, mais les invariants P0
  listés ne sont pas encore tous couverts.
- **À vérifier** : inventaire potentiellement en retard ou comportement non
  reproduit ; aucune modification autorisée avant l'audit ciblé.
- **Bloqué** : harnais, environnement ou décision d'architecture manquant.
- **Terminé** : invariant couvert et CI approprié vert.

| ID | Origine | Frontière | Risque / preuve manquante | Priorité | Statut | Références et prochaine preuve |
|---|---|---|---|---|---|---|
| REL-001 | NEXT-001 | Jean / activation | Un runtime désactivé ne doit jamais créer sa boucle, même avec des identifiants valides. | P0 | Terminé | SDTEST-272 couvre les quatre cas activation/identifiants ; la garde par itération reste en place. |
| REL-002 | NEXT-004 | CI multiplateforme | Les branches core macOS et Windows n'étaient pas exécutées avant release. | P1 | Terminé | CI native : 297 tests sur macOS ARM64 et 288 sur Windows x86_64 ; SDTEST-1584/1585 sont exercés. |
| REL-003 | NEXT-005 | SSH session/pool/tunnels | Les chemins critiques dépendent encore de vrais transports ou de sockets difficiles à piloter. | P0 | En cours | Session prouvée sans réseau par SDTEST-520/521/524/525 ; poursuivre avec jump SDTEST-528, pool 540..544 puis tunnels 562/564/566. |
| REL-004 | NEXT-005 | Terminal / PTY | Sortie, entrée, resize, notifier et destruction du processus manquent de preuve de cycle de vie. | P0 | À vérifier | Auditer SDTEST-967 et 980..983 ; décider explicitement le contrat de `Drop` avant toute correction. |
| REL-005 | — | Jean / état runtime | La concurrence et la réutilisation de l'instance enregistrée ne sont pas verrouillées. | P1 | À vérifier | Contrôler l'implémentation actuelle, puis SDTEST-270 (`runtime_busy`) et SDTEST-271 (persistance `instance_id`) avec faux executor/store. |
| REL-006 | NEXT-005 | IA et branchements GPUI | Les confirmations, cibles, politiques, centre de tâches et pièces jointes ont des scénarios P0 sans harnais d'intégration stable. | P0 | Bloqué | Définir le plus petit harnais GPUI ou extraire des réducteurs purs ; reprendre SDTEST-1365..1377 sans exécuter d'IA réelle. |
| REL-007 | — | Polling réseau | Une surface masquée ne doit pas continuer à interroger Support, Issues, Jean, Fleet ou Bext. | P0 | À vérifier | Auditer les gardes existantes puis couvrir leur prédicat commun avec SDTEST-1059. |
| REL-008 | — | Keychain natif | Les implémentations macOS et Windows compilent, mais aucun aller-retour de trousseau natif n'est exécuté en CI. | P0 | À vérifier | Évaluer l'isolation des runners puis SDTEST-121/122 sans secret utilisateur réel ni état persistant. |
| REL-009 | NEXT-005 | Mise à jour | Cadence, HTTP et vérification de hash restent couplés au temps et au réseau réels. | P1 | Ouvert | Injecter horloge et transport HTTP, puis fermer les lignes correspondantes d'`INFRA_BLOCKED.md`. |
| REL-010 | — | Terminal / protocoles | OSC 7/52, modes souris, styles de curseur, longues chaînes OSC, sélection et caractères larges restent partiellement couverts. | P1 | À vérifier | Reproduire chaque comportement avant de reprendre SDTEST-750..755 et 870..874. |
| REL-011 | — | Contrats API | Plusieurs lignes rouges Cloud, Issues, Support et Bext peuvent être réelles ou simplement en retard sur le code. | P1 | À vérifier | Auditer les routes et mocks actuels avant d'ajouter des tests ; corriger l'inventaire quand une preuve existe déjà. |

## Journal

- **2026-08-18 — REL-001 terminé.** La frontière réelle se trouve dans
  `Workspace::sync_runtime_loop`, avant `runtime_tick`; aucune logique réseau
  n'a été modifiée.
- **2026-08-18 — REL-002 terminé.** Le premier runner Windows a révélé que
  SDTEST-1585 attendait à tort des séparateurs Unix sur un hôte Windows. Le test
  utilise désormais les attentes natives ; le second passage macOS, Windows et
  le CI Ubuntu complet sont verts.
- **2026-08-18 — NEXT-005 éclaté.** L'ancien item générique est remplacé par
  REL-003, REL-004, REL-006 et REL-009 afin que SSH, PTY, GPUI et updater
  puissent avancer et être clôturés indépendamment.
- **2026-08-18 — REL-003 session en cours.** Un serveur `russh` en mémoire sur
  `tokio::io::duplex` couvre le vrai échange protocolaire sans socket et sans
  toucher au `known_hosts` utilisateur : PTY initial, commande avec stdout /
  stderr / code de sortie, annulation par EOF et resize sont verts.
