# Suivi UX ShellDeck

Ce document est la source de vérité pour la passe de réparation UX commencée
le 17 août 2026. Les anciennes notes et leurs captures ont été déplacées dans
`docs/design/local-reviews/archive/`, un espace de travail local ignoré par
Git.

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
| UX-011 | Support / Compositeurs | Tickets et Demandes utilisent le `Composer` partagé ; les placeholders sont visibles et les outils de pièces jointes suivent la même géométrie. | P0 | À valider | Répondre, ajouter une note et ouvrir les pièces jointes sur les deux surfaces. |
| UX-012 | Support / En-têtes | Statut, priorité et assignation sont modifiables directement depuis l'en-tête des Tickets et Demandes. | P1 | À valider | Changer chaque valeur puis rafraîchir le détail. |
| UX-013 | Support / Fils | Les messages, pièces jointes, notes et brouillons utilisent les primitives de fil partagées sans superposition observée. | P0 | À valider | Tester un fil long avec images, note système et brouillon IA. |

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
