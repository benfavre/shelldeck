# ShellDeck — AI Dock Companion

> Roadmap produit et technique pour rendre l'assistant IA de ShellDeck
> accessible depuis le système, sans devoir afficher ni initialiser toute
> l'interface principale.

## Statut vérifié au 2026-07-23

🚧 Phases A et B livrées le 2026-07-21 : Dock single-instance depuis le tray,
conversation globale séparée, focus composer, fermeture-vers-tray, retour vers
ShellDeck et démarrage caché récupérable.

La phase D est **partielle** :

- le raccourci fixe fonctionne dans le code sur Windows, macOS et Linux/X11
  avec `Ctrl+Shift+Space` (`Cmd+Shift+Space` sur macOS) ;
- le portail Global Shortcuts Wayland reste à réaliser ;
- l'enregistrement/désenregistrement dynamique n'est pas câblé : les toggles
  Settings prennent effet au prochain lancement ;
- il n'existe pas encore de capture de combinaison ni d'état d'erreur visible
  dans Settings.

La première tranche de la phase C est livrée : avec `start_hidden`, la fenêtre
principale possède un `CompanionRoot` léger et ne construit plus immédiatement
le `Workspace`, ses vues ou ses pollers. `main.rs` charge encore les connexions,
le store et peut lancer le Cloud Sync avant GPUI. Le Dock et la palette
initialisent encore le `Workspace` à leur première ouverture tant que
`AiCompanionController` n'en a pas été extrait.

La phase E est **partiellement livrée** : le Dock et la palette se masquent
déjà à la perte de focus et leur placement multi-écran est câblé. Restent le
deep link Assistant, l'icône tray macOS template, l'état visuel des tâches IA
dans le tray, la géométrie persistante et les finitions d'accessibilité/i18n.

La palette de commandes possède aussi une fenêtre compagnon autonome, ouverte
par `Ctrl+Alt+Space` (`Cmd+Alt+Space` sur macOS) sans afficher la fenêtre
principale avant la sélection d'une commande de navigation.

## Vision

ShellDeck doit pouvoir vivre discrètement dans le tray et ouvrir, à la demande,
un Dock latéral dédié à l'assistant IA. L'utilisateur peut l'afficher depuis le
menu tray ou avec un raccourci global, poser une question, puis le masquer sans
ouvrir la fenêtre principale de ShellDeck.

Le Dock est une surface ShellDeck à part entière : il réutilise le backend IA,
les conversations, les tâches persistantes, les réglages et les garde-fous
existants. Ce n'est ni un second processus IA ni une copie simplifiée de
`AiAssistantView`.

## Expérience cible

### Démarrage

- Une option permet de lancer ShellDeck automatiquement à la connexion OS.
- Une option `Démarrer dans le tray` empêche l'affichage de la fenêtre
  principale au lancement.
- Le tray reste disponible même lorsqu'aucune fenêtre ShellDeck n'est visible.
- Un échec du tray ne doit jamais laisser un processus invisible impossible à
  rouvrir : dans ce cas, ShellDeck ouvre sa fenêtre principale.

### Ouverture du Dock

Le Dock peut être affiché par :

- l'entrée tray `Ouvrir l'assistant IA` ;
- un raccourci global configurable ;
- le bouton IA de la fenêtre principale ;
- à terme, le deep link `shelldeck://assistant`.

Une seconde invocation masque le Dock. Une seule instance du Dock peut exister
par processus : il faut réactiver la fenêtre existante, jamais en créer une
nouvelle à chaque raccourci.

### Fenêtre

- Panneau de 480 px ancré au bord droit et haut comme la surface d'affichage.
- Aucun titre ni contrôle natif du système.
- Panneau non déplaçable, non redimensionnable et non minimisable.
- Surimpression au-dessus des fenêtres normales sans réserver l'espace bureau.
- Conversation active, historique et composer issus de `AiAssistantView`.
- Historique replié par défaut pour préserver la largeur du chat.
- Focus automatique dans le composer à l'ouverture.
- `Échap` masque le Dock si aucun dialogue interne n'est ouvert.
- Fermeture de la fenêtre = masquage vers le tray, pas arrêt du processus.
- Action explicite `Ouvrir ShellDeck` pour afficher l'application complète.
- Police UI et facteur d'échelle identiques à la fenêtre ShellDeck principale.

Sous Wayland, GPUI ne peut pas garantir un vrai « toujours au-dessus » sans le
protocole compositor `layer-shell`; le tray reste néanmoins le chemin de
réouverture portable.

## État existant réutilisable

ShellDeck possède déjà :

- `TrayService`, son menu et le routage `TrayCommand` ;
- `close_to_tray` et l'autostart multiplateforme ;
- le garde single-instance et les deep links `shelldeck://` ;
- `AiAssistantView` et les conversations persistantes locales ;
- le client provider-neutral `shelldeck_core::ai::AiClient` ;
- les secrets IA dans le keychain OS ;
- les tâches IA persistantes et les notifications de fin ;
- l'action interne `OpenAiAssistant` liée à `Cmd/Ctrl+Shift+K`.

Le raccourci actuel est local à la fenêtre GPUI. Il ne fonctionne pas lorsque
ShellDeck n'a pas le focus et ne doit pas être présenté comme un raccourci
global.

## Contrat de sécurité et de confidentialité

Les règles de [`.agents/ai.md`](../../.agents/ai.md) restent applicables au
Dock :

- aucun appel IA sans action explicite de l'utilisateur ;
- aucune commande exécutée par l'envoi d'un message ;
- les réponses restent des brouillons tant qu'une action typée et confirmée ne
  les applique pas ;
- aucune clé API dans la configuration, les logs ou l'état de fenêtre ;
- aucune capture automatique du presse-papiers, de la fenêtre active, de la
  sélection ou des frappes globales ;
- le contexte par défaut du Dock est `AiSurface::Global`, borné et sans données
  provenant silencieusement des terminaux, tickets ou scripts ;
- une pièce de contexte ne peut être ajoutée que par une action explicite
  depuis ShellDeck.

Le gestionnaire du raccourci global doit uniquement recevoir la combinaison
enregistrée. Il ne doit jamais devenir un keylogger généraliste.

## Architecture cible

### Principe

Le tray et le Dock appartiennent au runtime de l'application, pas au
`Workspace`. Le `Workspace` devient un consommateur optionnel du même état IA.

```text
Processus ShellDeck
├── CompanionRuntime
│   ├── TrayService
│   ├── GlobalShortcutService
│   ├── configuration + keychain
│   ├── AiCompanionController
│   └── fenêtre AiDock (optionnelle)
└── fenêtre principale + Workspace (optionnels et créés à la demande)
```

### `CompanionRuntime`

Le runtime applicatif doit connaître les handles des fenêtres ouvertes et
centraliser les commandes système :

- `ShowMainWindow` ;
- `ToggleAiDock` ;
- `OpenPalette` ;
- `ConnectPinned(Uuid)` ;
- `Quit`.

Il garantit les invariants suivants :

- une seule fenêtre principale ;
- un seul Dock ;
- une commande reçue depuis un thread tray ou raccourci est toujours remontée
  sur le foreground executor GPUI ;
- quitter appelle le shutdown du `Workspace` s'il existe ;
- masquer une fenêtre ne détruit pas une requête IA en cours.

### `AiCompanionController`

L'orchestration aujourd'hui attachée au `Workspace` doit être séparée par
étapes :

- configuration effective du backend et du modèle ;
- chargement/sauvegarde des conversations et tâches ;
- traitement de `AiAssistantEvent::Submit` ;
- mise à jour du résultat et notification de fin ;
- création du contexte global sûr.

Les workflows profondément contextuels — terminal, Support, scripts, Fleet —
restent orchestrés par le `Workspace`. Le contrôleur commun ne doit pas recevoir
leurs permissions ou leurs cibles par défaut.

### Fenêtres GPUI

La fenêtre principale ne doit plus être ouverte inconditionnellement dans
`main.rs`. Le bootstrap doit pouvoir choisir entre :

- démarrage normal : création immédiate du `Workspace` ;
- démarrage caché : runtime + tray seulement ;
- fallback sans tray : création immédiate du `Workspace`.

`show_main_window()` crée le `Workspace` au premier appel puis réactive la même
fenêtre aux appels suivants. `toggle_ai_dock()` suit le même principe avec une
vue `AiDockView` légère qui héberge l'assistant partagé.

## Configuration proposée

Une section dédiée évite de mélanger le cycle de vie du compagnon avec les
catégories de notifications du tray :

```toml
[companion]
enabled = true
start_hidden = false
global_shortcut_enabled = true
global_palette_shortcut_enabled = true
hide_dock_on_escape = true
hide_dock_on_focus_loss = true
always_on_top = false
```

Tous les champs doivent être `#[serde(default)]` afin que les anciennes
configurations continuent à être chargées. Le raccourci global est activé par défaut :
son enregistrement peut entrer en conflit avec une autre application ou
demander une autorisation spécifique à l'OS.

Le raccourci effectif est `Ctrl+Shift+Space` sur Windows/Linux et
`Cmd+Shift+Space` sur macOS. L'option existante `general.autostart` reste responsable du lancement à la
connexion. `companion.start_hidden` décide seulement si la fenêtre principale
est affichée durant ce lancement.

## Menu tray cible

Ordre proposé :

1. `Ouvrir l'assistant IA`
2. `Ouvrir ShellDeck`
3. `Palette de commandes`
4. `Connexions épinglées`
5. compteurs d'état
6. `Quitter`

L'entrée assistant doit être masquée ou désactivée avec une explication si
aucun backend IA utilisable n'est configuré ou si `AiSurface::Global` est
désactivée.

## Raccourci global multiplateforme

Le service choisi doit être isolé derrière une petite interface interne afin
de pouvoir remplacer son backend sans toucher au Dock :

```rust
trait GlobalShortcutService {
    fn register(&mut self, shortcut: &str) -> Result<()>;
    fn unregister(&mut self);
}
```

Contraintes :

- Windows : enregistrer une combinaison système et router l'événement vers
  GPUI sans bloquer la boucle native.
- macOS : respecter les règles de la main queue et expliquer toute permission
  OS réellement nécessaire.
- Linux X11 : enregistrement global classique lorsque disponible.
- Linux Wayland : privilégier le portail Global Shortcuts lorsqu'il est
  supporté ; sinon afficher clairement `Raccourci global indisponible` et
  conserver le menu tray comme fallback.

Un échec d'enregistrement n'empêche jamais ShellDeck de démarrer. Settings doit
afficher l'erreur et permettre de choisir une autre combinaison.

## Phasage

### Phase A — Dock depuis le tray

- [x] Ajouter `TrayCommand::ToggleAiDock`.
- [x] Ajouter l'entrée assistant au menu tray.
- [x] Créer une fenêtre GPUI compacte et single-instance.
- [x] Réutiliser `AiAssistantView` et le contexte global existant.
- [x] Ajouter `Ouvrir ShellDeck` et fermeture-vers-tray.
- [x] Masquer le Dock avec `Échap` hors dialogue interne.
- [x] Conserver temporairement le `Workspace` existant en mémoire si l'extraction
  du contrôleur est trop large pour cette phase.

Cette phase valide l'expérience produit. Elle ne prétend pas encore réduire
fortement la mémoire ou le temps de démarrage.

### Phase B — Démarrage silencieux

- [x] Ajouter `CompanionConfig` avec des defaults rétrocompatibles.
- [x] Ne pas afficher la fenêtre principale lorsque `start_hidden` est actif et que
  le tray est disponible.
- [x] Différer la création du `Workspace` en démarrage caché derrière un
  `CompanionRoot` léger. La fenêtre native existe encore cachée ; le
  `Workspace` est construit à la première commande qui nécessite son état.
- [x] Ajouter le fallback visible lorsque le tray échoue.
- [ ] Tester autostart + start hidden sur les trois OS. La CI habituelle teste
  Linux et la matrice de release compile macOS/Windows, mais aucun test de
  comportement Companion ne s'exécute encore sur ces deux plateformes.

### Phase C — Runtime réellement léger

- [x] Ne plus construire `Workspace`, ses vues et ses pollers au démarrage
  caché.
- [ ] Extraire `AiCompanionController` du `Workspace`.
- [ ] Introduire le `CompanionRuntime` applicatif qui possède le tray, les
  raccourcis et les handles de fenêtres sans dépendre d'un `Workspace`.
- [ ] Charger uniquement config, keychain, conversations/tâches et services
  compagnon au démarrage caché.
- [ ] Construire les vues SSH/terminal/Support/Fleet et leurs pollers uniquement à
  l'ouverture de la fenêtre principale.
- [ ] Repousser le parsing SSH, le chargement du store et le Cloud Sync au
  premier besoin de la fenêtre principale, ou documenter les données minimales
  réellement nécessaires au runtime compagnon.
- [ ] Mesurer le RSS et le temps de démarrage avant/après.
- [ ] Vérifier qu'aucun poll réseau propre au `Workspace` ne démarre en mode Dock
  seul.

### Phase D — Raccourci global

- [x] Implémenter les backends Windows, macOS et Linux/X11.
- [ ] Implémenter le portail Global Shortcuts sous Wayland ; le backend actuel
  renvoie explicitement « Global hotkeys not supported on Wayland ».
- [ ] Ajouter l'enregistrement/désenregistrement dynamique.
- [ ] Ajouter la capture de combinaison et l'état d'erreur dans Settings.
- [x] Toggle du Dock, focus composer et restauration de fenêtre.
- [x] Échec d'enregistrement non fatal avec fallback tray.
- [ ] Afficher le fallback Wayland dans Settings et documenter les permissions
  macOS réellement nécessaires.

### Phase E — Finitions

- [ ] Deep link `shelldeck://assistant` et hand-off single-instance.
- [ ] Icône tray template macOS (`tray-icon` expose
  `with_icon_as_template`, mais ShellDeck ne l'utilise pas encore).
- [ ] Restauration portable/persistante de la géométrie du Dock. Le placement
  sur l'écran contenant le pointeur et la migration multi-écran sont déjà
  fonctionnels, mais aucune géométrie n'est persistée.
- [x] Masquage du Dock et de la palette à la perte de focus.
- [ ] État visuel d'une tâche IA en cours dans le tray. Les notifications de
  fin existent déjà.
- [ ] Accessibilité clavier complète et traductions FR/EN. Le Dock est localisé
  et Escape/focus sont câblés ; le menu tray conserve encore plusieurs
  libellés français codés en dur.

## Critères d'acceptation V1

- ShellDeck peut être lancé puis rester utilisable avec sa fenêtre principale
  cachée.
- Le tray ouvre et masque une unique fenêtre Assistant.
- Ouvrir le Dock ne rend pas visible la fenêtre principale.
- Le composer reçoit le focus et une conversation peut être menée normalement.
- Une requête continue si le Dock est masqué et sa fin peut déclencher la
  notification IA existante.
- `Ouvrir ShellDeck` affiche ou réactive une unique fenêtre principale.
- `Quitter` arrête proprement les sessions et tâches détenues par le processus.
- Si le tray ne démarre pas, ShellDeck ouvre une fenêtre récupérable.
- Les anciennes configurations sans `[companion]` continuent à fonctionner.
- Aucun contenu externe n'est capturé automatiquement pour enrichir le prompt.

## Vérification et documentation de tests

Au début de l'implémentation, allouer les nouveaux IDs conformément à
[`.agents/testing.md`](../../.agents/testing.md) :

- SDUC pour démarrage caché récupérable ;
- SDUC pour toggle single-instance du Dock depuis le tray ;
- SDUC pour continuité d'une tâche lorsque le Dock est masqué ;
- SDUC pour raccourci global et fallback indisponible ;
- SDTEST unitaires du routage des commandes et de la réduction d'état des
  fenêtres ;
- SDTEST de compatibilité serde de `CompanionConfig` ;
- entrées de compile-check Linux/macOS/Windows pour tout backend spécifique.

Les vues GPUI ne doivent pas recevoir de tests artificiels. Extraire le routage,
le parsing du raccourci et la machine d'état des fenêtres en fonctions pures,
puis tester ces contrats.

### Couverture constatée le 2026-07-23

Les tests ciblés existants passent :

- décision create/show/hide du Dock ;
- ancrage à droite de l'écran ;
- parsing des raccourcis Dock et palette ;
- routage des entrées tray ;
- fallback visible de `start_hidden` sans tray ;
- compatibilité serde de `CompanionConfig`.

Les lacunes restantes sont :

- le test de politique de boot prouve que le démarrage caché diffère
  `Workspace`, mais aucun harnais GPUI ne vérifie encore l'absence effective de
  chaque poller ;
- aucun test d'enregistrement global réel sur macOS/Windows ;
- aucun test du fallback Wayland visible dans l'interface ;
- aucun test GPUI de perte de focus ;
- aucune couverture des mises à jour live du tray macOS/Windows.

## Hors périmètre initial

- Capturer automatiquement le texte sélectionné dans les autres applications.
- Observer le presse-papiers en arrière-plan.
- Exécuter une commande directement depuis une réponse du Dock.
- Afficher plusieurs fenêtres Assistant simultanées.
- Remplacer le tray par un daemon ou service système privilégié.
- Garantir un ancrage pixel-perfect sous l'icône tray sur tous les desktop
  environments Linux.

## Points à décider pendant l'implémentation

- Combinaison globale par défaut après tests de conflits OS.
- Le Dock et la palette se masquent façon Spotlight à la perte de focus.
- Conservation d'une fenêtre Dock cachée ou reconstruction de sa vue après une
  longue inactivité.
- Niveau de contexte explicite exposé par le Dock : contexte global seulement
  en V1, puis pièces jointes ShellDeck dans une phase ultérieure.
