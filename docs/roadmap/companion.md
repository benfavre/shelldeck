# ShellDeck — roadmap Companion

> Source canonique des priorités Companion.
>
> Ce document répond uniquement à trois questions : qu'est-ce qui est livré,
> qu'est-ce qui vient ensuite et qu'est-ce qui est bloqué. Les contrats
> techniques détaillés restent dans les documents spécialisés et les preuves de
> tests dans [`docs/testing/`](../testing/).

## État au 2026-07-29

| Bloc | État | Preuve principale |
|---|---|---|
| Autostart et démarrage caché récupérable | Livré | `config::autostart`, `CompanionRoot`, SDTEST-1382/1383/1391 |
| Tray, notifications et fermeture vers le tray | Livré sur Linux, macOS et Windows | `crates/shelldeck/src/tray`, `TrayNotification` |
| Deep links et instance unique | Livré, y compris `shelldeck://assistant` | `config::{deep_link,single_instance}`, SDTEST-1320..1323/1405 |
| Activité récente durable | Livré | `config::activity`, SDTEST-1330..1332 |
| Connexions épinglées | Livré | `AppConfig.pinned_connections`, sidebar et sous-menu tray |
| Onboarding | Livré | `OnboardingView`, `general.onboarding_completed` |
| IA transversale | Livrée dans le périmètre de sécurité actuel | [`ai-companion.md`](ai-companion.md) |
| AI Dock Companion | Phases A–E livrées | [`ai-dock-companion.md`](ai-dock-companion.md) |
| Personnage desktop autonome | Snap magnétique DPI-aware, suivi par ID stable, work areas par écran sur Windows/macOS et EWMH root sur X11, reduced motion OS et physique AABB déterministe livrés | SDUC-445..451, SDTEST-1460..1571 |

## Next

Aucun bloc fonctionnel Companion V1 identifié dans cette roadmap ne reste
ouvert. Les travaux restants sont des durcissements de plateforme : work areas
X11 réellement par moniteur, observation live du reduced motion OS, hit testing
natif par pixel, suppression au-dessus du plein écran, politique batterie,
lock/suspend et télémétrie de performance visible.

La recette sur de vraies sessions Linux/X11, Wayland, macOS et Windows reste
une vérification de release, pas une fonctionnalité produit à maintenir dans
la roadmap.

## Audit des éléments livrés

Vérification effectuée le 2026-07-29 contre le code et les inventaires :

- le démarrage caché conserve un `CompanionRoot` léger et diffère
  `Workspace`, le parsing SSH, le store et ses pollers ;
- `AiCompanionController` sert le Dock et les tâches sans construire
  `Workspace` ;
- le Dock et la palette sont single-instance, se masquent à la perte de focus
  et utilisent des raccourcis configurables ;
- Windows, macOS, Linux/X11 et le portail XDG Wayland possèdent un backend de
  raccourci ; les erreurs restent non fatales et visibles dans Settings ;
- `shelldeck://assistant` suit le hand-off authentifié et ouvre le Dock de façon
  idempotente sans révéler la fenêtre principale ;
- la tray suit la locale FR/EN, le Dock expose des contrôles nommés et la
  palette couvre Tab, flèches, Home/End et Page Up/Page Down ;
- la tray compte les tâches IA en génération/exécution et son indicateur
  ouvre directement leur centre dans le Dock sans révéler la fenêtre principale ;
- macOS reçoit un masque tray Monolith Retina dédié et laisse AppKit gérer ses
  états clair, sombre et pressé ;
- Linux applique les snapshots tray sur son thread GTK ; macOS et Windows les
  appliquent sur l'exécuteur foreground GPUI, sans déplacer les handles natifs
  non-`Send`, de sorte que compteurs, favoris et traductions restent live ;
- une fermeture d'onglet SSH et une sortie propre du shell restent silencieuses ;
  seule une perte de transport inattendue notifie avec le nom exact de la
  connexion et met à jour son état ;
- l'IA transversale possède tâches durables, notifications, policies par
  capacité, plans d'action typés, audit expurgé, triage Support et diagnostics
  Terminal séquentiels bornés ;
- le personnage desktop possède un solveur AABB déterministe dédié, pas un moteur
  de jeu général : gravité, vélocité terminale, drag horizontal, collision
  descendante one-way avec les tops de fenêtre, collisions bornées contre les
  murs latéraux et le plafond, fallback au sol d'écran, aperçu magnétique live
  avec hystérésis et seuils DPI du display cible, validation du même ID natif au
  relâchement, origine de perch partagée entre aperçu/commit/suivi, adoption du
  display porteur, suivi par ID stable, chute à la disparition, nettoyage des
  plateformes si le climbing est désactivé en chute et suppression des boucles
  sous mouvement réduit/off/still.

Les détails exhaustifs de comportement restent dans
[`USE_CASES.md`](../testing/USE_CASES.md). Cette roadmap ne les duplique pas.

## Non-goals

- pas de collecte silencieuse du presse-papiers, de la sélection externe ou du
  contenu utilisateur ;
- pas d'exécution IA cachée sans policy, statut, audit et arrêt ;
- pas de mode invité contournant l'authentification ;
- pas de daemon privilégié pour remplacer le tray ;
- pas de dépendance obligatoire à un provider IA.
