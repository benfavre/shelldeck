# Illustrations d'onboarding — brief de génération

Support de travail pour produire les visuels du parcours de première
connexion, un parcours différent selon le rôle. À donner à Codex avec les
**morceaux** fournis dans [`onboarding/fragments/`](onboarding/fragments/) :
l'idée n'est pas de dessiner une interface, c'est de **composer autour de vrais
morceaux d'interface** pour obtenir une fiche qui donne envie, plutôt qu'une
capture posée dans un rectangle.

---

## 1. Format cible

La zone média de la modale fait **560 × 200** points, en `object_fit: Contain`,
sur un fond `primary` à 6 %.

| | |
|---|---|
| Ratio | **2,8 : 1** — une bannière large, jamais un carré |
| Export | **1120 × 400** (2×), PNG ou WebP |
| Marge de sécurité | 24 px sur les quatre bords : rien de signifiant au ras du cadre |
| Thème | **clair uniquement** — la modale s'affiche sur fond clair dans les deux thèmes |

Le `Contain` signifie qu'une image d'un autre ratio sera centrée avec des
bandes du fond `primary 6 %`. Ce n'est pas un drame, mais une image au bon
ratio remplit toute la zone et rend nettement mieux.

---

## 2. La direction artistique, reprise de la page publique

Elle existe déjà et il faut s'y tenir — c'est ce qui fera que le premier écran
de l'application ressemble au site qui l'a vendue. Valeurs exactes issues de
`cloudflare/update-worker/src/landing.ts` :

```
--blue    #146bff      --coral   #f1644a
--blue-dark #0d42bf    --yellow  #ffc84a
--ink     #111318      --green   #079a74
--muted   #5d6470      --violet  #6d46e7
--line    #dfe4ec      --paper   #fffdf9   (blanc chaud, pas #fff)
--soft    #f5f7fa      --radius  24px
```

Typo : **Inter**, graisses 400/600/700, interlettrage négatif marqué sur les
titres (`-0.05em` à `-0.075em`).

Motifs récurrents de la page, réutilisables tels quels :

- **Le surlignage jaune tracé à la main** sous un mot bleu : un trait de 4 px
  `--yellow`, coins très arrondis, légère rotation `-1deg`. C'est la signature
  de la page.
- **La pastille numérotée** : cercle de 52 px, fond accent, chiffre blanc 13 px
  gras, cerné d'un anneau de 8 px couleur `--paper` qui la détache du fond.
- **La fenêtre applicative** : rayon 18 px, bordure `rgba(16,34,58,.2)`, double
  ombre portée `0 30px 70px rgba(15,45,86,.2)` et `0 4px 10px rgba(15,45,86,.12)`.
- **Le lavis aquarelle** (`campaign/roles-v1/watercolor.webp`) en fond de scène,
  très désaturé, jamais au premier plan.
- **La surlignée en majuscules** : 11 px, `letter-spacing: .12em`, précédée d'un
  bâtonnet bleu de 22 × 3 px.

---

## 3. Une couleur d'accent par rôle

La page publique en fixe déjà deux ; la troisième complète la série sans sortir
de la palette.

| Rôle | Accent | Origine |
|---|---|---|
| Utilisateur | `--blue #146bff` | scène 01 de la page |
| Support | `--coral #f1644a` | scène 02 de la page (`.scene-support .scene-number`) |
| Dev / super-admin | `--violet #6d46e7` | de la palette, encore inutilisé |

L'accent colore la pastille numérotée, le bâtonnet de la surlignée et les
éventuels liserés — **jamais** les morceaux d'interface eux-mêmes, qui gardent
leurs vraies couleurs.

---

## 4. La bibliothèque de morceaux

Découpés dans les captures produit de la campagne (`campaign/roles-v1/`, thème
clair, données fictives : Alex Martin, Studio Cobalt, Atelier Nord,
`camille.bernard@example.test`). **Aucune donnée réelle.**

| Fichier | Taille | Ce que c'est |
|---|---|---|
| `user-request-panel.png` | 480 × 406 | Le panneau « Nouvelle demande » entier : bandeau IA, sélecteurs site/priorité, titre, détails, bouton Créer |
| `user-identity-tabs.png` | 606 × 136 | La carte d'identité du compte et la barre d'onglets Accueil / Mes sites / Mes demandes / Mes informations |
| `support-queue.png` | 338 × 190 | La file : recherche, filtres Toutes / À traiter / En cours / Résolues, une ligne de demande |
| `support-detail-header.png` | 1454 × 84 | L'en-tête d'une demande : titre, statut, priorité, assigné, site, badge |
| `support-reply-composer.png` | 1448 × 116 | Le champ de réponse avec « Proposer une réponse » et le sélecteur de modèle |
| `dev-terminal-output.png` | 1474 × 134 | Sortie terminal verte sur fond sombre, texte fictif « ÉTAT OPÉRATIONNEL » |
| `dev-script-editor.png` | 1224 × 198 | En-tête d'un script + deux lignes de code `systemctl` / `journalctl`, boutons Modifier / Exécuter |
| `dev-script-list.png` | 266 × 494 | La liste des scripts avec ses filtres — **le morceau le moins résolu**, à n'utiliser qu'en petit ou flouté en second plan |
| `dev-tunnel-presets.png` | 1454 × 142 | Les trois cartes de préréglage : OpenCode Web, Chrome DevTools, Serveur de dev |
| `dev-tunnel-table.png` | 1454 × 134 | Le tableau des redirections, colonnes Local / Distant / Trafic |
| `ai-assistant-hero.png` | 542 × 240 | L'accueil de l'assistant : « Bonjour / Sur quoi on travaille ? » et les actions rapides |
| `ai-composer.png` | 538 × 158 | Le composer de l'assistant avec la puce de contexte, `+`, `@` et le sélecteur de modèle |

---

## 5. Le parcours, rôle par rôle

Chaque rôle a son propre enchaînement. Un client n'a pas à voir un terminal, et
un développeur n'a pas besoin qu'on lui explique comment déposer une demande.

### Utilisateur — accent bleu

| # | Écran | Morceaux |
|---|---|---|
| 01 | Bienvenue — votre lien direct avec l'équipe | aucun (marque) |
| 02 | Déposer une demande sans chercher le bon canal | `user-request-panel` |
| 03 | Suivre ses demandes et ses sites | `user-identity-tabs` |
| 04 | L'IA prépare le brouillon | `ai-composer` |

### Support — accent corail

| # | Écran | Morceaux |
|---|---|---|
| 01 | Bienvenue — une file, deux natures de sujets | aucun (marque) |
| 02 | Filtrer, assigner, prioriser | `support-queue` |
| 03 | Répondre dans le contexte | `support-detail-header` + `support-reply-composer` |
| 04 | L'IA prépare la réponse | `ai-assistant-hero` |
| 05 | Basculer de mode | (si le compte peut changer de mode) |

### Dev / super-admin — accent violet

| # | Écran | Morceaux |
|---|---|---|
| 01 | Bienvenue — le poste de pilotage complet | aucun (marque) |
| 02 | Terminaux et SSH | `dev-terminal-output` |
| 03 | Scripts réutilisables | `dev-script-editor` (+ `dev-script-list` en fond) |
| 04 | Tunnels et redirections | `dev-tunnel-presets` + `dev-tunnel-table` |
| 05 | L'assistant et ses mentions | `ai-composer` |
| 06 | Modes et raccourcis | aucun |

---

## 6. Les prompts

À passer un par un. Chacun suppose que les morceaux cités sont fournis en
pièces jointes.

### Commun à tous — contraintes dures

```
Format 1120x400 px (ratio 2.8:1), fond blanc chaud #fffdf9, thème clair.
Marge vide de 24 px sur les quatre bords.

Les captures d'interface fournies sont à COMPOSER, jamais à redessiner :
- ne réécris aucun texte d'interface, ne traduis rien, n'invente aucun libellé ;
- garde-les nettes, non déformées, sans filtre de couleur ;
- tu peux les recadrer, les incliner légèrement (max 3°), les superposer,
  leur ajouter le cadre fenêtre (rayon 18 px, bordure rgba(16,34,58,.2),
  ombres 0 30px 70px rgba(15,45,86,.2) et 0 4px 10px rgba(15,45,86,.12)).

Interdits : logo ou marque inventée, visage ou photo de personne, texte
lorem ipsum, dégradé arc-en-ciel, ombre dure, effet néon, style 3D lustré,
capture d'un autre logiciel.

Palette autorisée uniquement : #146bff #0d42bf #111318 #5d6470 #dfe4ec
#fffdf9 #f5f7fa #f1644a #ffc84a #079a74 #6d46e7.
Typographie si du texte est ajouté : Inter, interlettrage -0.05em.
```

### U-01 · Utilisateur, bienvenue

```
Bannière d'accueil, accent bleu #146bff.
Au centre-gauche, la marque ShellDeck en composition typographique Inter 700,
avec un mot souligné par un trait jaune #ffc84a de 4 px tracé à la main,
légèrement incliné (-1°), coins très arrondis.
À droite, un lavis aquarelle bleu très pâle qui déborde hors cadre, comme sur
la page publique. Pastille ronde 52 px bleue portant « 01 », cernée d'un
anneau 8 px #fffdf9, posée à cheval sur le bord du lavis.
Aucune capture d'interface sur cet écran : c'est l'ouverture.
```

### U-02 · Utilisateur, déposer une demande

```
Morceau : user-request-panel.png
Place-le à droite, incliné de 2° vers la gauche, dans le cadre fenêtre décrit
plus haut, dépassant légèrement du bord droit du cadre (il est coupé, ce qui
donne de la profondeur).
À gauche, un lavis aquarelle bleu pâle et trois petites pastilles flottantes
qui reprennent des éléments du panneau : une puce « site », une puce
« priorité », une puce trombone pour la pièce jointe. Elles sont dessinées,
pas découpées, dans la palette, avec le rayon 12 px.
Pastille « 02 » bleue en bas à gauche du panneau, anneau #fffdf9.
```

### U-03 · Utilisateur, suivre ses demandes

```
Morceau : user-identity-tabs.png
Centré, dans le cadre fenêtre, à plat (pas d'inclinaison) — c'est un écran de
repérage, il doit se lire.
Au-dessus et en dessous, deux rangées de cartes fantômes très pâles
(#f5f7fa, rayon 18 px) suggérant la liste des demandes qui se remplit, sans
aucun texte lisible dedans.
Pastille « 03 » bleue à gauche. Un fil courbe fin #dfe4ec relie la pastille au
premier onglet, comme la ligne de parcours de la page publique.
```

### U-04 · Utilisateur, l'IA prépare le brouillon

```
Morceau : ai-composer.png
Place-le en bas à droite, dans le cadre fenêtre, coupé par le bord bas.
Au-dessus, deux bulles de brouillon dessinées (pas découpées) en #f5f7fa avec
un liseré bleu, suggérant un texte proposé puis accepté — barres grises
anonymes, aucun mot lisible.
Une petite étincelle (le glyphe « sparkles » de l'application, 4 branches, pas
une étoile générique) en #146bff près du composer.
Pastille « 04 » bleue.
```

### S-01 · Support, bienvenue

```
Identique à U-01, mais accent corail #f1644a, lavis corail très pâle, et la
composition typographique évoque la file plutôt que la marque seule :
sous le mot souligné, trois lignes horizontales inégales #dfe4ec suggérant une
liste qui attend. Pastille « 01 » corail.
```

### S-02 · Support, filtrer et prioriser

```
Morceaux : support-queue.png
Place-le à gauche, à plat, dans le cadre fenêtre, coupé par le bord gauche.
À droite, agrandis et redessine — en respectant leurs formes exactes — trois
des puces de filtre du morceau (« À traiter », « En cours », « Résolues ») en
grand, flottantes, avec une ombre douce, comme si on les triait à la main.
L'une d'elles est active, remplie en corail à 12 % avec texte corail.
Pastille « 02 » corail en haut à droite.
```

### S-03 · Support, répondre dans le contexte

```
Morceaux : support-detail-header.png (en haut) et support-reply-composer.png
(en bas), empilés dans le MÊME cadre fenêtre, séparés par un vide gris #f5f7fa
d'environ 90 px qui représente le fil de discussion — sans texte inventé, juste
deux ou trois blocs anonymes gris clair alignés à gauche puis à droite.
Le tout centré, à plat, légèrement coupé en bas.
Pastille « 03 » corail à gauche.
```

### S-04 · Support, l'IA prépare la réponse

```
Morceau : ai-assistant-hero.png
À droite, dans le cadre fenêtre, incliné de 2°.
À gauche, un lavis corail pâle et une flèche fine courbe #5d6470 qui part d'un
bloc de message anonyme vers le panneau de l'assistant, indiquant que la
réponse se prépare à partir du fil.
Pastille « 04 » corail.
```

### D-01 · Dev, bienvenue

```
Identique à U-01, accent violet #6d46e7, lavis violet très pâle.
Sous la composition typographique, une ligne de terminal stylisée :
fond #111318, rayon 12 px, un curseur vert #079a74 clignotant dessiné, aucun
texte réel. Pastille « 01 » violette.
```

### D-02 · Dev, terminaux et SSH

```
Morceau : dev-terminal-output.png
Place-le en grand, centré, dans le cadre fenêtre, incliné de 2°, coupé à droite.
Derrière lui, deux autres fenêtres fantômes décalées (#f5f7fa, rayon 18 px,
vides) suggérant plusieurs sessions ouvertes.
À gauche, trois pastilles rondes vertes #079a74 de 8 px alignées verticalement
avec un fil #dfe4ec : les hôtes connectés.
Pastille « 02 » violette.
```

### D-03 · Dev, scripts réutilisables

```
Morceaux : dev-script-editor.png au premier plan, dev-script-list.png en
second plan à gauche, réduit et légèrement flouté (il est basse résolution,
ne l'agrandis pas).
L'éditeur est dans le cadre fenêtre, incliné de 2°, coupé à droite.
Ajoute une petite étiquette « Exécuter » dessinée en violet #6d46e7 reliée par
un fil fin au bouton correspondant du morceau.
Pastille « 03 » violette.
```

### D-04 · Dev, tunnels et redirections

```
Morceaux : dev-tunnel-presets.png en haut, dev-tunnel-table.png en bas,
empilés dans le même cadre fenêtre avec 16 px d'écart, à plat.
Par-dessus, entre les deux, dessine une flèche horizontale fine allant de
gauche à droite avec la mention « local » à gauche et « distant » à droite en
Inter 600, 11 px, majuscules, interlettrage 0.12em, couleur #5d6470.
Lavis violet pâle derrière. Pastille « 04 » violette.
```

### D-05 · Dev, l'assistant et ses mentions

```
Morceau : ai-composer.png, centré, dans le cadre fenêtre, à plat, agrandi.
Au-dessus, trois pastilles de mention dessinées à la manière de l'application :
texte #146bff sur fond #146bff à 14 %, rayon 4 px, avec un peu d'air de chaque
côté — l'une porte une icône de serveur, l'autre une icône de ticket, la
troisième une icône de tunnel. Elles descendent vers le composer, comme si on
les insérait.
Pastille « 05 » violette.
```

### D-06 · Dev, modes et raccourcis

```
Aucune capture. Composition graphique : trois cartes égales, rayon 18 px, fond
#fffdf9, bordure #dfe4ec, portant chacune un nom de mode en Inter 600 —
Utilisateur, Support, Dev — et un liseré supérieur de 3 px à la couleur du mode
(#146bff, #f1644a, #6d46e7). La carte Dev est légèrement surélevée et porte
une ombre plus marquée.
Sous les cartes, deux touches de clavier dessinées (rayon 6 px, bordure
#dfe4ec, ombre 1 px) portant « Ctrl » et « K ».
Pastille « 06 » violette.
```

---

## 7. Ce qu'il restera à faire côté application

Les visuels seuls ne suffisent pas : l'onboarding actuel
(`crates/shelldeck-ui/src/onboarding_view.rs`) sert **une seule séquence** de
quatre étapes — `Welcome`, `Modes`, `Surfaces`, `Shortcuts` — dont seule
`Modes` est conditionnelle. Il faudra :

1. remplacer cette liste fixe par une séquence choisie selon le mode effectif ;
2. y brancher les nouveaux visuels, un par étape et par rôle ;
3. les clés i18n correspondantes en `fr` puis `en` ;
4. le cas du compte qui peut changer de mode : il voit le parcours de son mode
   courant, plus l'étape « basculer de mode » à la fin.

Les images actuelles (`assets/images/onboarding/*.png`, juillet) datent d'avant
plusieurs refontes et sont à remplacer, pas à compléter.
