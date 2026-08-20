# Correction 01 — trois scènes à refaire

À donner tel quel. Le reste du lot est validé et ne doit pas bouger.

---

```
Le lot de 15 visuels d'onboarding est bon : format, direction artistique,
accent par rôle, fragments intacts, pipeline HTML/CSS + export. On garde tout.

Trois scènes sont à refaire, pour une seule et même raison.

## Le critère qui manquait

Ces images s'affichent dans une modale de 560 x 200 points, soit la MOITIÉ de
leur taille d'export. Le lot a été jugé en 1120 x 400, où tout se lit. À la
taille réelle, trois scènes ne communiquent plus rien.

Nouveau critère d'acceptation, à appliquer à chaque scène : réduis l'export à
560 x 200 et regarde-le à cette taille. Si l'élément central n'est pas
identifiable en une seconde, la scène est à revoir. Un fragment large et dense
réduit de moitié devient une texture grise ; il faut alors montrer UN détail
signifiant en grand plutôt que le panneau entier.

## Scène 1 — dev-04-tunnels

Symptôme : à 560 px, le tableau des redirections est une bouillie grise. Les
trois cartes de préréglage sont également illisibles.

Cause : `dev-tunnel-table.png` fait 1454 px de large et `dev-tunnel-presets.png`
contient trois cartes ; réduits, aucun des deux ne se lit.

Correction : abandonne ces deux fragments et compose sur le principe « une
redirection, en grand ».
- `dev-tunnel-card-single.png` (272 x 142) : UNE carte de préréglage
  (OpenCode Web), placée à gauche, dans le cadre fenêtre, agrandie d'environ
  1,6x pour que `localhost:4096 --> remote:4096` soit franchement lisible.
- `dev-tunnel-row-single.png` (750 x 84) : l'en-tête du tableau et UNE ligne,
  placés en bas à droite, coupés par le bord droit.
- Entre les deux, garde la flèche LOCAL -> DISTANT : c'est l'élément le plus
  lisible de la scène actuelle et il porte l'idée.
- Pastille « 04 » violette, lavis violet pâle.

## Scène 2 — support-03-context

Symptôme : à 560 px, la scène est quasi vide — un en-tête illisible, trois
blocs gris anonymes, un bloc rose. Rien à comprendre.

Cause : la consigne d'origine demandait un fil de discussion représenté par des
blocs anonymes. Sans un seul message réel, il ne reste aucune information.

Correction : montre un vrai échange, court.
- `support-message-single.png` (830 x 52) : le message de camille.bernard,
  placé en haut à gauche, dans une bulle claire, agrandi d'environ 1,3x.
- `support-reply-composer.png` : conserve-le, en bas, coupé par le bord bas —
  c'est l'action de la scène.
- Entre les deux, UN seul bloc anonyme corail très pâle aligné à droite, qui
  suggère la réponse en cours. Pas trois, un seul.
- Supprime `support-detail-header.png` : illisible à cette taille et il ne
  porte rien que le reste ne dise déjà.
- Pastille « 03 » corail.

## Scène 3 — user-03-follow

Symptôme : la carte d'identité forme un pavé SOMBRE au milieu d'une composition
claire et aérée. Elle attire l'œil pour rien et jure avec tout le lot. Les deux
barres fantômes en haut ressemblent à des artefacts.

Cause : `user-identity-tabs.png` inclut le fond sombre de la carte de compte.

Correction :
- Remplace-le par `user-tabs-only.png` (606 x 46) : la barre d'onglets seule,
  sans la carte sombre. Place-la en haut, centrée, dans le cadre fenêtre,
  agrandie d'environ 1,4x — « Mes demandes » est l'onglet actif, c'est le sujet.
- Sous les onglets, deux ou trois cartes de demande fantômes en #f5f7fa,
  rayon 18 px, alignées et régulières — elles doivent lire comme une liste qui
  se remplit, pas comme des barres flottantes.
- Garde le fil courbe fin #dfe4ec qui relie la pastille à l'onglet actif.
- Pastille « 03 » bleue.

## Contraintes inchangées

Mêmes que le brief initial : 1120 x 400, thème clair, palette fermée, fragments
jamais redessinés ni retraduits, aucun logo inventé, aucun visage. Les nouveaux
fragments sont dans `docs/design/onboarding/fragments/` à côté des autres.

Réexporte uniquement ces trois slugs, les noms de fichiers ne changent pas :
dev-04-tunnels, support-03-context, user-03-follow.
```
