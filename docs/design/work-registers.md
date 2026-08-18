# Registres de travail ShellDeck

Ce fichier est le point d'entrée des suivis durables. Chaque chantier possède
un seul registre propriétaire afin d'éviter les listes contradictoires.

| Registre | Périmètre | Document |
|---|---|---|
| Expérience utilisateur | Défauts visuels ou d'interaction reproduits et recette utilisateur. | [`ux-repair-tracker.md`](./ux-repair-tracker.md) |
| Produit | Capacités réellement absentes ou incomplètes pour l'utilisateur. | [`product-gap-tracker.md`](./product-gap-tracker.md) |
| Fiabilité | Invariants de sécurité, tests, plateformes et infrastructure de validation. | [`reliability-test-tracker.md`](./reliability-test-tracker.md) |
| Dette technique | Nettoyages sans nouvelle capacité produit, à justifier par reproduction ou mesure. | [`technical-debt-tracker.md`](./technical-debt-tracker.md) |

## Règles communes

1. Vérifier le comportement et le code actuels avant de modifier quoi que ce
   soit ; une ligne rouge d'inventaire n'est pas automatiquement un bug.
2. Relier les changements observables aux `SDUC-*` et les preuves automatisées
   aux `SDTEST-*` existants, sans réutiliser leurs identifiants.
3. Conserver les dépendances externes en **Bloqué** plutôt que de simuler une
   fonctionnalité qui échouera en production.
4. Une correction possède son commit et sa PR propres. Le statut ne devient
   **Terminé** qu'après les contrôles adaptés et un CI vert.
5. Un chantier qui change de registre garde son ancien identifiant dans la
   colonne « Origine » afin que l'historique reste retrouvable.
