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
| REL-003 | NEXT-005 | SSH session / ProxyJump / tunnels | Les chemins critiques dépendent encore de vrais transports ou de sockets difficiles à piloter. | P0 | Terminé | Session, ProxyJump et les trois directions de tunnel sont prouvés par SDTEST-520/521/524/525, 528/530 et 562/564/565/566/567. Plus aucune ligne P0 ouverte dans `tests-ssh.md` ; les P1/P2 restants (SDTEST-508/509/522/523/527/529/600/601) restent suivis par l'inventaire, et le pool dormant par DEBT-005. |
| REL-004 | NEXT-005 | Terminal / PTY | Sortie, entrée, resize, notifier et destruction du processus manquaient de preuve de cycle de vie. | P0 | Terminé | Contrat de `Drop` arrêté et couvert par SDTEST-967/969 ; sortie, entrée, resize et repaint événementiel par SDTEST-980..983. Plus aucune ligne P0 ouverte dans `tests-terminal.md`. Restent les P1 SDTEST-984..986 et la dette protocolaire suivie par REL-010. |
| REL-005 | — | Jean / état runtime | La concurrence et la réutilisation de l'instance enregistrée ne sont pas verrouillées. | P1 | À vérifier | Contrôler l'implémentation actuelle, puis SDTEST-270 (`runtime_busy`) et SDTEST-271 (persistance `instance_id`) avec faux executor/store. |
| REL-006 | NEXT-005 | IA et branchements GPUI | Les confirmations, cibles, politiques, centre de tâches et pièces jointes ont des scénarios P0 sans harnais d'intégration stable. | P0 | Bloqué | Définir le plus petit harnais GPUI ou extraire des réducteurs purs ; reprendre SDTEST-1365..1377 sans exécuter d'IA réelle. |
| REL-007 | — | Polling réseau | Une surface masquée ne doit pas continuer à interroger Support, Issues, Jean, Fleet ou Bext. | P0 | Terminé | Audit : les quatre gardes étaient correctes, y compris hors session. Elles sont désormais un seul prédicat pur couvert par SDTEST-1059. Le sondage git local est hors périmètre — il ne sort pas sur le réseau et suspend déjà son travail fenêtre masquée. |
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
- **2026-08-18 — Pool SSH requalifié.** `ConnectionPool` n'a aucun appelant de
  production ; les terminaux, scripts, forwards, discovery et sync utilisent
  des `SshClient` dédiés. Les SDTEST-540..546 ne sont donc plus présentés comme
  des risques runtime P0 avant la décision d'intégration ou de suppression.
- **2026-08-18 — REL-003 tunnels P0 couverts.** Le harnais `russh` en mémoire a
  reproduit une fuite réelle : `stop()` arrêtait le listener mais laissait les
  copies des connexions acceptées détachées. Local, remote et SOCKS possèdent
  désormais leurs tâches via `JoinSet`, les annulent et les drainent avant
  `Stopped`. Local echo, compteurs, ports occupés, SOCKS CONNECT/rejets,
  `stop_all` et `cleanup` sont verts.
- **2026-08-18 — REL-003 ProxyJump couvert.** Le canal `direct-tcpip` est
  maintenant ouvert par `SshClient::open_jump_channel`, extrait sans changer le
  comportement, et prouvé par deux serveurs `russh` en mémoire : le bastion
  enregistre la demande, puis exécute le serveur cible *à l'intérieur* du canal.
  Le test échoue bien si la demande vise le mauvais hôte. La sélection du hop
  (`none`, valeur vide, chaîne séparée par virgules) est isolée dans
  `first_jump_hop` et couverte par SDTEST-530.
- **2026-08-18 — Constat sur `_jump_session`.** Une mutation de contrôle
  (remplacer le champ par `None`) laisse le test au vert : c'est le canal ouvert,
  pas le champ, qui maintient le transport du bastion en vie. Le champ reste en
  place — il lie les deux durées de vie — mais son commentaire actuel affirme
  plus que ce qui est démontré. À reprendre avec une preuve dédiée avant de
  reformuler le contrat.
- **2026-08-18 — REL-003 terminé.** Le remote forward est la dernière direction
  couverte : le serveur en mémoire enregistre `tcpip_forward`, puis joue le côté
  distant en ouvrant lui-même le canal `forwarded-tcpip`. L'événement traverse le
  vrai `ClientHandler` jusqu'à `forwarded_tcpip_rx`, ce qui rend SDTEST-602 vert
  du même coup — c'était une ligne rouge en retard sur le code, pas un manque. Les
  mutations de contrôle confirment la preuve : mauvais port local et tâche de
  connexion détachée font échouer le test.
- **2026-08-18 — Asymétrie relevée.** `start_remote_forward` ne conserve pas le
  handle client, contrairement aux forwards local et SOCKS : ses canaux sont
  ouverts par le serveur. En production, la `SshSession` propriétaire maintient le
  transport ; ce n'est donc pas un défaut, mais le test doit tenir ce handle
  explicitement, et c'est commenté à cet endroit.
- **2026-08-18 — REL-004, contrat de `Drop` arrêté.** Le zombie a d'abord été
  reproduit : sur Unix l'enfant de `portable_pty` *est* un `std::process::Child`,
  dont le `Drop` ne tue ni n'attend. Chaque onglet terminal fermé laissait donc
  un processus en état `Z` pour toute la durée de vie de l'application. Contrat
  retenu et implémenté par `ChildReaper` : fermer le master d'abord (SIGHUP),
  laisser un délai de grâce à l'enfant pour sortir de lui-même — c'est ce qui lui
  permet d'écrire son historique et d'exécuter ses traps —, puis tuer et
  moissonner seulement s'il est encore là. Le moissonnage ne bloque jamais le fil
  qui ferme l'onglet.
- **2026-08-18 — Une assertion temporelle remplacée.** La première version
  vérifiait « moissonné avant la fin du délai de grâce » pour prouver que
  l'enfant n'avait pas été tué. Un kill immédiat satisfait aussi cette
  condition : l'assertion ne discriminait rien. Les tests observent désormais la
  branche réellement empruntée, et la mutation « supprimer le délai de grâce »
  fait bien virer SDTEST-967 au rouge.
- **2026-08-18 — REL-004 terminé.** Deux lignes P0 étaient en réalité déjà
  prouvées : SDTEST-980 et 981 découlent de tout test qui relit une sortie de
  commande dans la grille. Elles sont requalifiées plutôt que dupliquées. Les
  deux vraies preuves manquantes étaient le repaint et le resize.
- **2026-08-18 — Le capteur anti-polling pendait au lieu d'échouer.** Sous la
  régression même qu'il doit détecter — une boucle de repaint périodique —
  l'attente de silence bouclait indéfiniment : en CI, cela aurait consommé tout
  le job sans message exploitable. L'attente est bornée et le test échoue
  désormais en 3 s. Règle à retenir : un capteur de régression doit être borné
  par le comportement qu'il surveille, pas par la patience du runner.
- **2026-08-18 — REL-007 terminé, sans défaut trouvé.** L'audit cherchait une
  surface masquée qui continue d'interroger le réseau ; les quatre gardes
  étaient déjà correctes, y compris le cas « déconnecté » qu'on soupçonnait pour
  Jean — `effective_jean_config` court-circuite sur `!signed_in()`. Le risque
  réel n'était donc pas une fuite existante mais la dérive : quatre prédicats
  recopiés, aucun endroit où vérifier la règle commune. Ils sont réunis dans
  `workspace/polling.rs`, pur et testable hors GPUI.
- **2026-08-18 — Un implicite rendu explicite.** « Déconnecté ⇒ aucun polling »
  découlait de trois mécanismes séparés (`resolve_effective` force User,
  `has_jean` et `can_access_mode` exigent un compte). C'était vrai mais nulle
  part énoncé, donc invérifiable. Le prédicat le pose maintenant en tête : le
  comportement à l'exécution est inchangé, la garantie est devenue testable.
