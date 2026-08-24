# Suivi UX ShellDeck

Ce document est la source de vérité pour la passe de réparation UX commencée
le 17 août 2026. Les anciennes notes et leurs captures ont été déplacées dans
`docs/design/local-reviews/archive/`, un espace de travail local ignoré par
Git. Les manques produit, la fiabilité et la dette technique vivent dans les
registres séparés référencés par [`work-registers.md`](./work-registers.md).

## Statuts

- **Ouvert** : défaut reproduit, aucune correction terminée.
- **En cours** : correction en développement.
- **À valider** : correction présente et vérifiée techniquement ; recette
  visuelle utilisateur encore attendue.
- **Validé** : recette visuelle terminée.

## Suivi

| ID | Surface | Problème | Priorité | Statut | Vérification attendue |
|---|---|---|---|---|---|
| UX-001 | Support / Tickets et Demandes | Les titres Slack affichaient encore `<url\|libellé>` ou `<url>` dans les listes et les détails. | P0 | Validé | Contrôlé dans la liste et le détail d'une vraie demande Slack. |
| UX-002 | User / Détail demande | L'ancien audit signalait la disparition du compositeur pendant le défilement, alors que la correction du 12 août n'y avait pas été reportée. | P0 | Validé | Fil long parcouru jusqu'en bas puis jusqu'en haut ; le compositeur est resté visible. |
| UX-003 | Support / Accueil | Les compteurs sont désormais actionnables et l'espace libre présente les tickets prioritaires ainsi que les demandes récentes. | P1 | Validé | Chaque compteur et les deux types de ligne ont été contrôlés dans l'application reconstruite. |
| UX-004 | User et Support / Chrome | La barre d'état technique (`connections`, `forwards`, `scripts`, branche Git, palette) est désormais réservée au mode Dev authentifié. | P1 | Validé | User et Support récupèrent l'espace sans perdre leurs coins arrondis ; Dev conserve la barre complète. |
| UX-005 | Support / Tickets et Demandes | Les deux listes partagent désormais la même action `Actualiser`, avec icône visible et composant standard. | P1 | Validé | Les deux en-têtes et leurs actualisations ont été contrôlés dans l'application reconstruite. |
| UX-006 | Support / Listes | Les colonnes suivent 38 % de l'espace avec des bornes de 280 à 440 px ; sous 760 px mis à l'échelle, liste et détail se remplacent avec un retour explicite. | P2 | Validé | Liste, détail et retour contrôlés sur Tickets et Demandes à 600 puis 1210 px. |
| UX-007 | Dock IA | Le mode Markdown compact utilise désormais une échelle H1–H6 relative au corps, sans modifier les tailles document. | P1 | Validé | H1 à H4 contrôlés dans un rendu compact isolé de 480 px. |
| UX-008 | Dock IA | Les titres d'historique ont une largeur définie avec ellipse et le rail distingue l'activité active. | P1 | Validé | Trois titres longs et la sélection active ont été contrôlés dans un profil isolé. |
| UX-009 | Sheets | Les couches qui peignent les quatre coins possèdent désormais le même rayon hors mode maximisé. | P0 | Validé | Déjà contrôlé sur les quatre coins et en mode maximisé. |
| UX-010 | Conversations | Markdown, liens HTTP(S), libellés d'e-mail `[alt]<URL>` et confirmation d'ouverture partagent le même rendu sécurisé. | P0 | Validé | Le vrai e-mail Outlook a été contrôlé, puis son libellé a ouvert la confirmation externe. |
| UX-011 | Support / Compositeurs | Tickets et Demandes utilisent le `Composer` partagé ; les placeholders sont visibles et les outils de pièces jointes suivent la même géométrie. | P0 | Validé | Réponse, note interne et panneau de pièces jointes contrôlés sans envoi sur les deux surfaces. |
| UX-012 | Support / En-têtes | Statut, priorité et assignation sont modifiables directement depuis l'en-tête des Tickets et Demandes. | P1 | Validé | Les six menus ont été contrôlés ; les mutations ticket ont été exercées en mémoire et les écritures HTTP sont couvertes par les mocks. |
| UX-013 | Support / Fils | Les messages, pièces jointes, notes et brouillons utilisent les primitives de fil partagées sans superposition observée. | P0 | Validé | Deux fils longs contrôlés avec images, fichiers, lien, notes, citations et brouillons. |
| UX-014 | IA / Feuilles d'assistant | Toute feuille IA était **inerte** dès qu'un formulaire modal était ouvert : `render.rs` ajoutait les feuilles avant le `modal_layer`, un `occlude()` plein écran. Le nommage IA d'un script ou d'un tunnel, dont le seul point d'entrée est un bouton *dans* ce modal, était donc inutilisable — aucun clic n'atteignait Envoyer, Accepter, ni même la croix. | P0 | À valider | Ouvrir « Nouveau script » → **Nommer** → la feuille répond, l'IA génère, Accepter renseigne le champ Nom. Vérifier aussi les quatre coins de la fenêtre feuille ouverte. |
| UX-015 | Toute l'application / Calques | Les fonds plein cadre (palette, modales, feuilles) ne portaient pas le rayon de la fenêtre : ouvrir n'importe lequel carrait les quatre coins. Sept des neuf sites avaient oublié la condition, recopiée à la main. | P0 | À valider | Rampes des quatre coins mesurées, calque par calque, contre la fenêtre au repos. |
| UX-016 | User / Support / Bienvenue | UX-004 a retiré la barre d'état de ces trois surfaces sans transférer la propriété des coins bas. Leur racine opaque est devenue la couche la plus basse et carrait le bas de la fenêtre — visible en permanence, et flagrant sous un fond assombri. | P0 | À valider | Les trois surfaces doivent produire les mêmes rampes basses que le mode Dev, qui n'était pas touché. |
| UX-017 | Toute l'application / Calques | Échap ne fermait aucun calque et les flèches ne déplaçaient pas la sélection de la palette : le champ focalisé déclare un contexte `Input` qui lie ces touches à des actions, que GPUI résout sans jamais délivrer l'événement clavier aux ancêtres. | P0 | À valider | Échap sur la palette et sur la modale de connexion ; flèches dans la palette. |
| UX-018 | Palette de commandes | « Quitter » ouvrait la liste en position présélectionnée, et les commandes Dev restaient offertes en mode Support faute de reconstruction au changement de mode. | P0 | À valider | Ouvrir la palette en Support après un passage par Dev ; vérifier la première ligne. |
| UX-019 | User / Accueil | La pastille de comptage de la bannière tombait sur l'illustration selon le recadrage et devenait illisible, avec un accord au pluriel faux. | P0 | À valider | Bannière lue avec un seul site, à plusieurs largeurs de fenêtre. |
| UX-020 | User / Nouvelle demande | « Créer » restait en bleu plein avec un titre vide alors que `create_issue_now` retournait en silence : le clic ne produisait rien et rien ne l'expliquait. | P0 | À valider | Bouton grisé à vide, plein dès que le titre est saisi. |
| UX-021 | Connexion | Un refus d'identifiants affichait « Votre session a expiré », message d'un jeton périmé et non d'un mot de passe erroné, et il était répété à l'identique dans une bulle en bas à droite. | P0 | À valider | Échec de connexion : message unique, sous les champs, qui parle d'identifiants. |
| UX-022 | User / Mes demandes | Les lignes n'affichaient ni date de mise à jour ni nombre de commentaires, là où la même demande côté Support porte les deux. Un client ne pouvait pas voir que le support avait répondu. | P1 | À valider | Ligne client et ligne Support côte à côte sur la même demande. |
| UX-023 | User / Détail demande | Le fil est ancré en haut, contre celui des tickets qui est ancré en bas : plusieurs centaines de pixels de vide séparent le dernier message du compositeur. | P1 | Ouvert | Fil court : le dernier message doit toucher le compositeur. |
| UX-024 | Support / États vides | Le volet de détail annonçait « Aucun ticket ouvert » au-dessus d'une liste qui en contenait quatre — le sens réel étant « rien de sélectionné » — et son corps de texte tutoyait, seul de toute l'application. | P1 | À valider | Les deux files, sans sélection. |
| UX-025 | Paramètres / Raccourci global | Le `Debug` Rust d'une erreur X11 s'affichait tel quel, tronqué en plein milieu, dès qu'une autre application détenait déjà la combinaison — le cas courant. | P1 | À valider | Raccourci déjà pris : message en français, pastille contenue. |
| UX-026 | Dev / Barre latérale | Cinq activités sur huit ouvraient un panneau qui répétait la liste que leur propre vue affichait juste à côté — Scripts en donnait le cas d'école, six lignes identiques côte à côte, 570 px de navigation avant le premier pixel de contenu. | P1 | À valider | Scripts, Redirections, Éditeur, Terminaux, Activité et Sites doivent occuper toute la largeur ; seules Connexions gardent leur panneau. |
| UX-027 | Transitions de mode | Trois secondes pleines à chaque changement de mode, alors que rien ne charge au retour : les entités Dev sont masquées et non détruites. Un agent qui fait des allers-retours Support ↔ Dev payait six secondes par aller-retour. | P1 | À valider | Première entrée dans un mode : palier long conservé. Retour : voile déjà refermé sous la seconde. |
| UX-028 | Dev / Sites | Le panneau listait les sites du locataire Manage pendant que la vue annonçait « Aucun site découvert » : deux collections différentes, un seul mot, côte à côte. « Tout effacer », rouge, restait par ailleurs cliquable au-dessus d'un écran vide. | P1 | À valider | L'activité ne doit désigner qu'une chose ; l'action destructive ne doit apparaître que s'il y a quelque chose à effacer. |
| UX-029 | Dev / Terminal | L'état vide peignait le fond de la palette *terminal* : en thème clair, un panneau noir plein cadre avec une carte claire posée dessus. La carte « Lancer Claude Code » affichait par ailleurs `claude --dangerously-skip-permiss`, coupé en plein mot — précisément sur l'information qui compte. | P1 | À valider | Thème clair : l'écran suit le thème. Carte Claude : légende lisible, commande exacte au survol. |
| UX-030 | User / Feuilles de demande | La feuille de nouvelle demande ou de détail couvrait toute la barre de titre, y compris réduire, agrandir et fermer : aucune action de fenêtre ne restait accessible à la souris. | P1 | À valider | Feuille ouverte, la barre de titre reste entièrement visible et ses trois boutons répondent. |
| UX-031 | Toute l'application / Texte | Le moteur de retour à la ligne considérait l'apostrophe typographique `U+2019` comme une ponctuation, contrairement à l'apostrophe ASCII, et pouvait donc commencer une ligne par `’échange` ou `’importe`. | P1 | À valider | À largeur contrainte, `l’`, `d’`, `qu’` et `n’` restent attachés aux deux côtés de l'élision. |
| UX-032 | Onboarding / Dernière étape | L'audit signalait encore la dernière ligne de raccourcis coupée par le pied de page, alors que le correctif du parcours par rôle avait déjà plafonné la carte et rendu son corps défilant. | P1 | À valider | À 800×650, le pied de page reste fixe et le corps défile jusqu'à « Ouvrir les paramètres ». |
| UX-033 | Dev / Navigation et barre d'état | Les statuts/priorités inconnus reprenaient le jeton serveur brut, les trois compteurs fuyaient en anglais et confondaient activité en cours et inventaire, tandis que deux destinations changeaient de nom selon le rail, le menu ou leur titre. | P2 | À valider | En français puis en anglais, contrôler les trois compteurs à 1 et plusieurs ; « Synchronisation serveur » et « Activité récente » gardent leur nom entre le rail, Aller et le titre. |

## Règle de mise à jour

Une ligne ne passe à **À valider** qu'après correction et tests ciblés. Elle ne
passe à **Validé** qu'après la recette visuelle de l'utilisateur. Toute nouvelle
régression reçoit un nouvel identifiant au lieu d'écraser l'historique d'une
ligne existante.

## Journal
- **2026-08-17 — UX-001 → À valider.** Adaptateur de présentation appliqué aux
  listes et détails des tickets/demandes, aux confirmations de suppression et
  aux demandes récentes des modes User et Support. Les trois cas unitaires
  SDTEST-1610..1612 sont verts ; la recette dans l'application reconstruite
  reste à faire.
- **2026-08-17 — UX-001 → Validé.** ShellDeck reconstruit et relancé ; une vraie
  demande Slack a été contrôlée dans la liste puis dans son détail. Les chevrons
  mrkdwn ont disparu et l'URL nue reste lisible lorsque Slack ne fournit aucun
  libellé.
- **2026-08-17 — UX-002 → Validé sans nouvelle correction.** Le pied de page
  fixe était déjà présent depuis `65c5c89` (12 août), mais l'ancien audit était
  resté en retard. Sur la version reconstruite, un fil long a été parcouru dans
  les deux sens : seul le fil défile et le compositeur reste ancré en bas.
- **2026-08-17 — UX-003 → À valider.** Les quatre compteurs ouvrent maintenant
  la file et le filtre qu'ils annoncent, après suppression des anciennes
  recherches ou contraintes invisibles. Deux listes compactes rendent l'accueil
  utile : SLA/urgences/non-assignés d'un côté, demandes récemment mises à jour de
  l'autre. Chaque ligne ouvre directement son détail.
- **2026-08-17 — UX-003 → Validé.** Recette effectuée dans ShellDeck reconstruit :
  Ouverts, SLA, Non attribués et Demandes ont chacun activé la bonne file et le
  bon filtre ; un ticket prioritaire et une demande récente ont ouvert leur
  détail réel. Aucune écriture réseau n'est liée à ces navigations.
- **2026-08-17 — UX-004 → À valider.** La barre d'état n'est plus montée dans
  l'arbre de rendu des modes User, Support et de l'écran de bienvenue. Son état,
  ses abonnements et les notifications de mise à jour restent actifs ; seul le
  chrome technique est masqué. SDTEST-1616 verrouille les quatre cas de rendu.
- **2026-08-17 — UX-004 → Validé.** ShellDeck reconstruit puis contrôlé en
  Support, User et Dev : les deux modes produit récupèrent les 28 px et gardent
  leurs coins inférieurs arrondis, tandis que Dev conserve compteurs, branche,
  palette et version. Le cas bienvenue reste vérifié sans révoquer la session.
- **2026-08-17 — UX-005 → À valider.** Le bouton invisible de Demandes
  référençait `refresh`, absent de l'inventaire Lucide. Tickets et Demandes
  utilisent maintenant un helper commun fondé sur `Button`, le glyph
  `refresh-cw` existant et le libellé localisé ; leurs événements réseau restent
  distincts.
- **2026-08-17 — UX-005 → Validé.** Les deux en-têtes affichent la même action
  dans ShellDeck reconstruit. Les deux clics ont rafraîchi leur file respective
  sans modifier les filtres ou la sélection ; le survol et la géométrie sont
  identiques.
- **2026-08-17 — UX-006 → À valider.** Le défaut initial de composition des
  lignes avait déjà été corrigé dans `c822b6b`, mais la largeur restait figée.
  Tickets et Demandes partagent maintenant une colonne proportionnelle bornée.
  L'état sans sélection ne choisit volontairement aucun item : cela pourrait
  marquer un ticket lu, et l'accueil fournit déjà la file prioritaire attendue.
- **2026-08-17 — UX-006, garde-fou étroit.** La première recette à 600 px a
  révélé que le détail devenait trop étroit. Sous un seuil de 760 px sensible à
  l'échelle UI, ShellDeck affiche donc soit la liste pleine largeur, soit le
  détail plein largeur avec `Retour à la liste`. Les deux états vides partagent
  aussi un confinement qui empêche leur texte de déborder.
- **2026-08-17 — UX-006 → Validé.** Dans ShellDeck reconstruit à 600 px,
  Tickets et Demandes remplacent chacun leur liste par le détail sélectionné ;
  leurs boutons Retour distincts restaurent la bonne file. À 1210 px, la
  colonne proportionnelle et l'état vide restent contenus sans sélection
  implicite.
- **2026-08-17 — UX-010 → Validé.** Deux tickets réels ont confirmé une origine
  e-mail Postmark/Outlook pour la forme non standard `[alt]<URL>`. Le parseur
  limité aux autolinks HTTP(S) adjacentes affiche maintenant seulement le
  libellé. Sur le ticket `RE: CORRECTION JEU RENTREE`, l'URL brute et les
  crochets ont disparu ; un clic sur le libellé conserve la confirmation
  externe et affiche le domaine cible.
- **2026-08-17 — UX-007 → Validé.** Le chemin compact réutilisait encore la
  rampe document 32/28/24/20 px sur un corps de 12,5 px. H1 à H6 suivent
  maintenant 1,44/1,32/1,20/1,12/1,06/1× le corps. Un rendu isolé de 480 px a
  confirmé la hiérarchie H1–H4 et le retour à la ligne ; le test SDTEST-1621
  verrouille aussi les tailles document inchangées.
- **2026-08-17 — UX-008 → Validé sans modification.** Le correctif existant
  donne déjà une largeur définie aux deux lignes de texte avant les boutons.
  Trois conversations aux titres et contextes longs ont été chargées dans un
  profil temporaire : chaque ligne affiche une ellipse et la conversation
  sélectionnée reçoit bien le fond actif attendu.
- **2026-08-17 — UX-011 → Validé sans modification.** Tickets et Demandes
  montent déjà le même `Composer` avec les mêmes bornes de hauteur, le même
  bouton `+` et le même sélecteur de pièces jointes. La recette dans
  l'application reconstruite a confirmé leur géométrie, leurs placeholders et
  le panneau complet. Le basculement ticket vers `Note interne` modifie bien le
  placeholder et l'action finale sans envoyer de contenu au client.
- **2026-08-17 — UX-012 → Validé sans modification.** Les six badges d'en-tête
  ouvrent directement leurs sélecteurs complets. Sur la fixture Ticket en
  mémoire, statut, priorité et assignation ont chacun mis à jour le détail et
  les compteurs, puis le rafraîchissement a réinstallé les valeurs de la
  fixture. Pour ne pas écrire sur Manage, la demande fictive réelle a servi à
  contrôler visuellement les trois menus sans choix ; les 16 tests `issues` et
  22 tests `manage_support`, dont les corps d'actions staff, restent verts.
- **2026-08-17 — UX-013 → Validé sans modification.** Les fixtures Ticket et
  Demande ont été activées uniquement le temps de la recette, puis remises à
  `false`. Sur les deux fils longs, image, fichier, lien, citation, Markdown
  riche, séparateurs de jour, notes de statut/GitHub/système, indicateur de
  saisie, brouillon IA, brouillon local et échec d'envoi restent contenus. Le
  compositeur demeure ancré et aucune superposition n'a été observée.

- **2026-08-21 — UX-015 → À valider.** Les neuf fonds plein cadre passent par
  `shelldeck_ui::overlay::window_backdrop`, seul appelant restant de
  `ShellDeckColors::backdrop()` : la condition « arrondi sauf si maximisé » ne
  peut plus être oubliée par un dixième calque. `SDPATCH-032` a par ailleurs été
  élargi — le rayon du fond de la feuille adabraka était conditionné à la
  variante Assistant, alors que le fond couvre la fenêtre quelle que soit la
  variante ; une feuille `Default` (détail d'un job Fleet) carrait donc la
  fenêtre. Mesures : palette, formulaire de connexion, nouveau script,
  navigateur de modèles, nouvelle redirection, formulaire de connexion d'hôte et
  feuille de demande produisent tous les rampes de la fenêtre au repos ;
  maximisée, les quatre coins restent pleins avec et sans calque.
- **2026-08-21 — UX-016 → À valider.** Cause distincte de UX-015, révélée par
  celle-ci : une fois les fonds correctement arrondis, le bas restait carré
  parce que la surface *sous* le fond l'était. `round_window_bottom` donne les
  deux coins bas à la couche opaque la plus basse — racine de
  `render_user_home`, racine de `SupportView`, racine de
  `render_welcome_screen` — conformément à la règle 10 de
  `.agents/window-rounding.md`. Dev n'était pas concerné : sa barre d'état
  portait déjà ses coins. Avant : `bl [7,1,1,1,1,0,0,0]`, `br
  [7,1,1,1,1,1,1,0]`. Après : `bl [7,5,4,3,2,0,0,0]`, `br [7,5,4,3,2,1,1,0]`,
  identiques au mode Dev sur la même capture.

- **2026-08-21 — UX-017 → À valider.** L'`Input` adabraka déclare
  `key_context("Input")` et lie `escape`, `up`, `down`, `home`, `end`, `tab` à
  des actions ; GPUI résout l'action et ne délivre jamais la touche aux
  ancêtres, si bien que le `capture_key_down` de la palette et les
  `on_key_down` des sept modales étaient du code mort pour exactement les
  touches qu'ils visaient. Les calques écoutent désormais les actions
  elles-mêmes, en phase de capture pour passer avant que le champ ne déplace
  son curseur. `crate::overlay` réexporte ces actions avec l'explication, pour
  que le prochain calque ne retombe pas dedans.
- **2026-08-21 — UX-018 → À valider.** `activate_current_mode` ne reconstruisait
  pas la palette : les entrées sont filtrées par mode à la construction, donc
  passer de Dev à Support laissait « Nouveau terminal », « Basculer barre
  latérale », « Fermer l'onglet » et « Onglet suivant » dans une surface sans
  terminal, sans barre latérale et sans onglets. « Quitter » passe par ailleurs
  en dernière position ; SDTEST-1700 verrouille l'invariant sur les quatre
  combinaisons de rôle.
- **2026-08-21 — UX-019 → À valider.** Le champ bleu de la bannière est peint
  dans l'illustration, dont le recadrage `Cover` déplace le bord avec la
  largeur de fenêtre : la pastille retombait sur la pâte à modeler claire. Fond
  sombre plein au lieu d'une teinte à 22 %, colonne de texte écartée de 44 px,
  et le compteur suit la convention `.one` / `.many` déjà en place pour les
  compteurs de la zone de notification.
- **2026-08-21 — UX-020 → À valider.** `commit_enabled` sur le compositeur de
  création, aligné sur le compositeur de commentaire qui l'utilisait déjà — et
  sur la modale de connexion, qui désactivait correctement son action
  principale.
- **2026-08-21 — UX-021 → À valider.** `login_error_message` traite le 401 du
  formulaire comme un refus d'identifiants, là où `api_error_message` le traite
  — correctement pour ses propres appelants — comme une session expirée.
  L'échec ne s'affiche plus qu'à un seul endroit : sous les champs, où se porte
  le regard ; la bulle ne sert plus que si la modale n'est pas montée.

- **2026-08-21 — UX-022 → À valider.** Les lignes de « Mes demandes » portent le
  compteur de commentaires et la date relative, avec le libellé partagé
  `issue_comment_count_label` pour que la même donnée se lise pareil des deux
  côtés. `support.meta.comments` abandonne au passage l'abréviation « comm. »,
  ambiguë en français, et suit la convention `.one` / `.many`.
- **2026-08-21 — UX-023 conservé Ouvert.** Une première tentative d'ancrage par
  intercalaire extensible n'a rien changé et a été retirée plutôt que laissée
  en place : le conteneur n'a pas de hauteur de référence dans un parent
  `overflow_y_scroll`. Côté Support le fil est une `uniform_list` virtualisée
  qui se place en fin de liste ; ici c'est un bloc défilant ordinaire. Le vrai
  correctif passe par un `ScrollHandle` positionné à l'ouverture de la feuille.
- **2026-08-21 — UX-024 → À valider.** Deux clés dédiées
  (`support.empty.no_ticket_selected`, `no_request_selected`) remplacent des
  titres qui annonçaient l'inverse de ce qu'ils voulaient dire, et les deux
  textes passent au vouvoiement comme le reste de l'application. L'emoji « 💬 »
  du volet Tickets devient `messages-square` du lot Lucide embarqué, aligné sur
  le volet Demandes.
- **2026-08-21 — UX-025 → À valider.** `classify_shortcut_error` distingue le
  portail absent, la combinaison déjà prise et le reste ; le détail part dans
  les journaux. SDTEST-1701 pinne les trois cas, dont la forme exacte relevée
  sur cette machine.

- **2026-08-21 — UX-026 → À valider.** `has_panel` répondait à la question
  « cette activité a-t-elle une liste ? » au lieu de « le panneau montre-t-il
  ce que la vue ne montre pas ? ». Seules Connexions — aucune vue principale ne
  liste les hôtes — et Sites — les sites du locataire Manage, que la vue des
  sites *découverts* ne connaît pas — en gardent un. La fonction porte
  désormais la raison de chaque réponse, et SDTEST-1702 ferme la liste au lieu
  de tolérer une exception nommée.
- **2026-08-21 — UX-027 → À valider.** Le palier devient variable : complet à
  la première entrée dans un mode depuis le lancement, 420 ms ensuite. La
  personnalité de la mascotte, spécifiée dans `mode-transitions.md`, est donc
  vue en entier une fois par mode et par session, sans être refacturée à chaque
  aller-retour. La courbe du voile suit la durée réellement en cours, sinon il
  disparaissait avant la fin ou restait après ; SDTEST-1703 le vérifie pour les
  deux durées.

- **2026-08-21 — UX-028 → À valider.** Renommer les deux surfaces ne suffisait
  pas : la première tentative a donné un panneau intitulé « Sites détectés » qui
  listait des sites Manage, soit la contradiction aggravée. L'activité du rail
  porte sur les sites *détectés* par un scan ; les sites Manage ont déjà leur
  sélecteur dans la barre de titre et leur entrée « Changer de site actif » dans
  la palette. Le panneau disparaît donc, et `has_panel` se réduit à Connexions
  — seule activité dont le panneau montre ce qu'aucune vue ne montre.
- **2026-08-21 — UX-029 → À valider.** L'état vide du terminal est du chrome
  applicatif et suit désormais `ShellDeckColors` et non la palette du terminal
  (`.agents/theming.md`). Les légendes des cartes CLI disent ce que le bouton
  fait — « Sans confirmation des actions », « Bac à sable, accord à la
  demande » — et la commande exacte passe en infobulle : tronquée dans la
  carte, elle déformait l'avertissement au lieu de le porter.

- **2026-08-24 — UX-030 → À valider.** Le fond occlusif et le panneau commencent
  désormais sous les 40 px de la barre de titre, en partageant la même constante
  scale-aware que le chrome. Leur bord haut devient donc interne et carré ; seul le
  panneau continue de posséder le coin bas droit de la fenêtre. Recette X11 : feuille
  de détail ouverte, le bouton d'agrandissement a effectivement fait passer la
  fenêtre de 1210×810 à 1920×1048, puis retour à l'état flottant.
- **2026-08-24 — UX-031 → À valider.** `LineWrapper::is_word_char` classe
  maintenant `U+2019` comme l'apostrophe ASCII. `SDPATCH-116` inventorie le correctif
  et son garde-fou sur `l’échange`, `d’ici`, `qu’importe` et `n’importe`. À 900 puis
  800 px, l'état vide Support garde `l’échange` entier et les Paramètres gardent
  `n’importe` entier.
- **2026-08-24 — UX-032 → À valider sans nouvelle correction.** Le code de
  `21de839` portait déjà la structure prescrite par `.agents/overflow.md` : carte
  plafonnée à 90 % de la fenêtre, rangées fixes, corps `flex_grow + min_h(0)`
  défilant. Le rapport était resté en retard. Rejoué à 800×650 sur l'étape
  Support 5/5 : le pied de page reste visible et le défilement atteint les quatre
  raccourcis, jusqu'à « Ouvrir les paramètres ».
- **2026-08-24 — UX-033 → À valider.** Les replis de statut et de priorité
  deviennent des libellés génériques traduits au lieu d'exposer le vocabulaire
  du protocole. La barre d'état reçoit maintenant le vrai nombre de connexions
  actives et accorde « connexion active », « redirection active » et « script
  en cours » ; elle ne peut plus faire lire « 0 scripts » comme un inventaire
  vide. Le rail, le menu Aller et les titres partagent enfin
  « Synchronisation serveur » et « Activité récente ». SDTEST-1704 couvre les
  deux locales dans le scénario séquentiel imposé par `rust_i18n`. Recette X11
  sur le profil isolé : les trois zéros portent leur activité exacte ; les deux
  entrées du menu Aller ont ensuite ouvert une vue au titre strictement identique.
