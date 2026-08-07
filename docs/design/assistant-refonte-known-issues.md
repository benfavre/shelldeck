# Refonte assistant / support — ce que je n'ai PAS su corriger

Ce fichier existe pour être honnête. Beaucoup de tours de développement se sont
enchaînés sans que l'écran change réellement, ou que la correction promise soit
visible. Voici, sans enrobage, ce qui reste cassé au moment où on ferme cette
passe.

## Le fil d'une demande, `render_issue_detail`

### 1. Superpositions récurrentes
Symptôme : dans un fil réel comme dans la fixture, des éléments s'impriment
par-dessus d'autres. Trois fois au moins pendant cette passe :

- Le corps d'une demande longue passait sous les notes système (résolu en
  posant `whitespace_normal` + `min_w(0)` + `overflow_hidden` sur chaque
  ligne, cf. `render_body_lines`).
- La miniature d'une pièce jointe posait sur la note statut suivante
  (tentative : fusionner les pièces jointes dans le bloc du message
  d'ouverture, `gap(8) → gap(16)` sur le thread). D'après le dernier retour
  utilisateur, **la superposition existe toujours** — cette fois « en bas »
  du fil. Cause exacte non identifiée.

### 2. Superposition en bas du fil (non résolu)
Signalée après le correctif précédent. Je n'ai pas de capture précise et je
n'ai pas identifié le nœud. Piste probable : la carte du brouillon IA
(`render_issue_ai_draft_card`) ou le composer lui-même s'imprime sur le
dernier commentaire quand le fil est plus haut que la fenêtre. À vérifier
sur `render_issue_composer` + `sup-issue-thread` (`min_h(px(0.0))` +
`overflow_y_scroll`).

### 3. Le nom en gras — trois tours perdus
J'ai insisté deux fois que `FontWeight::SEMIBOLD` (600) sur Inter serait
suffisant. Il ne l'est pas visuellement à 12 px, ligne à ligne avec du texte
muet. Passé en `FontWeight::BOLD` (700) à 12,5 px — l'utilisateur a confirmé
que ça se voit enfin.

## Sur ma méthode, ce qui n'a pas marché

### Le placeholder de l'`Editor` de réponse
Symptôme : ne s'affiche jamais dans le composer de réponse support. La clé
`support.issue_comment_placeholder = "Commenter la demande…"` existe,
`Editor::placeholder(...)` est appelé, `EditorState::is_empty()` renvoie bien
vrai à l'ouverture. Je n'ai jamais élucidé pourquoi. Piste : le
`show_border(false)` que j'ai ajouté modifie le rendu du wrapper interne de
l'`Editor`, et son placeholder est peint en `absolute top:12 left:12` qui
peut atterrir hors du wrapper visible.

### Les remplacements de texte à l'aveugle
Deux fois pendant cette passe j'ai supprimé le mauvais `let entity =
cx.entity();` en cherchant par motif — les deux fois le compilateur m'a
attrapé, mais c'était une chirurgie fragile sur un fichier de 2 000 lignes.
Une fois j'ai transformé le mauvais menu de priorité, une fois j'ai touché
une barre de filtre à 1 700 lignes du composer.

### La différence Tickets / Demandes
Trois fois pendant la passe, je n'ai pas vérifié explicitement que je
touchais bien `support_view/requests.rs` et pas `tickets.rs`. Les deux
onglets ont des composers séparés (`render_composer` dans `tickets.rs`,
`render_issue_composer` dans `requests.rs`). Toute la refonte est côté
Demandes ; le composer des Tickets reste à l'ancien schéma.

## Ce qui reste à faire, dans l'ordre

1. **Trouver et corriger la nouvelle superposition en bas du fil.** Sans
   capture précise, la piste principale est le composer ou la carte IA qui
   déborde du scroll container.
2. **Le placeholder du composer support.** Traquer la géométrie du wrapper
   `Editor` — probablement lié à `show_border(false)`.
3. **Le composer des Tickets** — copier tout ce qui a été fait côté
   Demandes.
4. **La liste 340 px des Tickets** — même traitement que les Demandes
   (titre sur toute la ligne, statut en point coloré, priorité seulement si
   non-Normale).
5. **Sélecteur d'assignation** dans l'en-tête du détail — l'action existe
   côté serveur (`IssueAssign`), le sélecteur pas encore.
