# ShellDeck — roadmap Companion

> Source canonique des priorités Companion.
>
> Ce document répond uniquement à trois questions : qu'est-ce qui est livré,
> qu'est-ce qui vient ensuite et qu'est-ce qui est bloqué. Les contrats
> techniques détaillés restent dans les documents spécialisés et les preuves de
> tests dans [`docs/testing/`](../testing/).

## État au 2026-07-24

| Bloc | État | Preuve principale |
|---|---|---|
| Autostart et démarrage caché récupérable | Livré | `config::autostart`, `CompanionRoot`, SDTEST-1382/1383/1391 |
| Tray, notifications et fermeture vers le tray | Livré sur Linux ; mutation live limitée sur macOS/Windows | `crates/shelldeck/src/tray`, `TrayNotification` |
| Deep links et instance unique | Livré, y compris `shelldeck://assistant` | `config::{deep_link,single_instance}`, SDTEST-1320..1323/1405 |
| Activité récente durable | Livré | `config::activity`, SDTEST-1330..1332 |
| Connexions épinglées | Livré | `AppConfig.pinned_connections`, sidebar et sous-menu tray |
| Onboarding | Livré | `OnboardingView`, `general.onboarding_completed` |
| IA transversale | Livrée dans le périmètre de sécurité actuel | [`ai-companion.md`](ai-companion.md) |
| AI Dock Companion | Phases A–D et finition accessibilité/i18n livrées | [`ai-dock-companion.md`](ai-dock-companion.md) |

## Next

Ordre recommandé pour terminer la V1 :

1. **État des tâches IA dans le tray**
   - publier le nombre de tâches `Generating`/`Executing` ;
   - ouvrir le centre de tâches depuis l'indicateur ;
   - conserver les notifications de fin existantes.
2. **Icône tray template macOS**
   - fournir un asset monochrome transparent dédié ;
   - activer `with_icon_as_template(true)` uniquement sur macOS.
3. **Géométrie persistante du Dock**
   - restaurer l'écran et les dimensions valides ;
   - retomber sur le placement courant si l'écran a disparu ou changé.
4. **Validation comportementale multiplateforme**
   - tester `autostart + start_hidden` sur Linux, macOS et Windows ;
   - tester les raccourcis réels sur macOS/Windows et le portail sur Wayland ;
   - valider les mises à jour live du tray hors Linux.

## Bloqué

- **Tags des demandes par l'IA** : l'API Issues n'expose pas encore de mutation
  dédiée. Aucun état local divergent ne doit être ajouté en attendant.
- **Boutons “Ouvrir dans ShellDeck” côté Manage** : travail dans le dépôt
  serveur/Manage, pas dans ShellDeck.
- **Tests GPUI d'intégration** : plusieurs scénarios P0/P1 restent `Red` dans
  [`tests-ui-and-app.md`](../testing/tests-ui-and-app.md). Les contrats purs
  associés sont testés, mais le câblage de vues n'a pas encore de harnais
  maintenable.

## Later

- rendre les compteurs tray cliquables vers leur surface ;
- mettre à jour les compteurs et favoris tray en direct sur macOS/Windows ;
- enrichir la notification SSH avec l'identité de la connexion ;
- ajouter reconnexion rapide, déconnexion et changement de mode au tray ;
- ajouter un spotlight facultatif à l'onboarding ;
- reprendre exactement une ancienne session terminal lorsque son contenu sera
  sérialisable.

## Audit des éléments livrés

Vérification effectuée le 2026-07-24 contre le code et les inventaires :

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
- l'IA transversale possède tâches durables, notifications, policies par
  capacité, plans d'action typés, audit expurgé, triage Support et diagnostics
  Terminal séquentiels bornés.

Les détails exhaustifs de comportement restent dans
[`USE_CASES.md`](../testing/USE_CASES.md). Cette roadmap ne les duplique pas.

## Non-goals

- pas de collecte silencieuse du presse-papiers, de la sélection externe ou du
  contenu utilisateur ;
- pas d'exécution IA cachée sans policy, statut, audit et arrêt ;
- pas de mode invité contournant l'authentification ;
- pas de daemon privilégié pour remplacer le tray ;
- pas de dépendance obligatoire à un provider IA.
