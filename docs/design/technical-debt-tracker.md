# Registre de dette technique

Nettoyages qui ne livrent pas directement une nouvelle capacité. Chaque ligne
doit être reproduite, profilée ou reliée à une règle actuelle avant édition :
un ancien audit non coché n'est pas une autorisation de refactorer.

## Statuts

- **Ouvert** : dette actuelle confirmée et résultat mesurable défini.
- **À vérifier** : constat ancien à confronter au code actuel.
- **Bloqué** : dépendance technique préalable absente.
- **Rejeté** : proposition incompatible avec le contrat actuel ; ne pas faire.
- **Terminé** : nettoyage mesuré, testé et sans régression visuelle.

| ID | Origine | Surface | Dette | Priorité | Statut | Garde-fou |
|---|---|---|---|---|---|---|
| DEBT-001 | NEXT-006 | Dialogues | Certaines surfaces peuvent encore contourner les composants de dialogue standard. | P2 | À vérifier | Inventorier les usages actuels avant migration et contrôler focus, fermeture et destructive confirm. |
| DEBT-002 | NEXT-006 | Support / listes | Des identifiants reconstruits et allocations par ligne sont encore signalés dans l'ancien audit. | P2 | Ouvert | Profiler la liste réelle, stabiliser uniquement les chemins mesurés, puis refaire la recette Tickets/Demandes. |
| DEBT-003 | — | Commentaires | Des commentaires narratifs de type historique de commit restent mêlés aux invariants utiles. | P2 | Ouvert | Supprimer ou réduire uniquement ceux qui n'expliquent ni contrainte, ni sécurité, ni choix non évident. |
| DEBT-004 | Audit 2026-07-10 | Terminal / peinture | Remplacer `shape_line` par le fast path `paint_glyph` avait été proposé dans l'ancien audit. | — | Rejeté | `shape_line` est obligatoire : `paint_glyph`/`GlyphCache` ne rend pas fiablement les glyphes gras ou colorés. Ne pas rouvrir sans changement vérifié du fork. |
| DEBT-005 | REL-003 / SDUC-048 | SSH / pool | `ConnectionPool` est exporté mais sans appelant ; son contrat documenté de réutilisation ne correspond ni au code, qui remplace l'entrée, ni aux sessions dédiées actuelles. | P2 | Ouvert | Décider intégration avec politique explicite multi-terminaux, ou suppression. Ne pas ajouter une abstraction de test avant cette décision. |

## Journal

- **2026-08-18 — NEXT-006 éclaté.** Dialogues et coût des lignes Support sont
  désormais suivis séparément ; le nettoyage de commentaires reçoit sa propre
  preuve attendue.
- **2026-08-18 — DEBT-004 rejeté.** L'ancien item non coché contredit la règle
  terminal actuelle d'`AGENTS.md`. Il est conservé ici précisément pour éviter
  qu'un futur passage ne réintroduise cette régression.
- **2026-08-18 — DEBT-005 confirmé.** Recherche de tous les appelants dans le
  workspace : seul `pool.rs` référence `ConnectionPool`; les surfaces actives
  construisent directement `SshClient`.
