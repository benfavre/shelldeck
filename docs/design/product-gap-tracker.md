# Registre des manques produit

Capacités utilisateur réellement absentes ou volontairement incomplètes. Les
défauts purement visuels restent dans le registre UX et les preuves manquantes
dans le registre de fiabilité.

## Statuts

- **Ouvert** : manque confirmé et réalisable avec les contrats actuels.
- **À vérifier** : signal issu d'un audit ; comportement actuel à contrôler.
- **Bloqué** : contrat serveur, autorisation ou décision produit manquante.
- **Terminé** : capacité livrée et vérifiée de bout en bout.

| ID | Origine | Surface | Manque confirmé | Priorité | Statut | Prochaine preuve attendue |
|---|---|---|---|---|---|---|
| PROD-001 | NEXT-002 | Bext / Instance distante | Le bouton par connexion cible encore le loopback local. Un tunnel SSH seul serait rejeté par la wire-auth Bext. | P1 | Bloqué | Définir une identité administrative dédiée ou la remise sécurisée d'un jeton SDK, puis tester tunnel, authentification, erreur et fermeture. |
| PROD-002 | NEXT-003 | Demandes | Les tags ne disposent pas encore de mutation et de filtrage de bout en bout dans l'API Issues et le client. | P1 | Bloqué | Étendre le contrat serveur, puis couvrir création, modification, lecture et filtrage côté ShellDeck. |
| PROD-003 | UX / S-01 | Support / Demandes | La barre de filtres n'affiche aucun compteur, là où celle des Tickets en a un par pastille. L'API Issues ne renvoie pas de décompte par statut, et la liste reçue est déjà filtrée côté serveur : compter localement produirait des chiffres faux. | P2 | Bloqué | Ajouter un objet `counts` à `GET …/issues` côté `inklura-manage-prism`, puis brancher les pastilles sur celui-ci comme le fait déjà la file Tickets. |

## Journal

- **2026-08-18 — PROD-001 confirmé bloqué.** Bext exige en mode
  `BEXT_SDK_WIRE_AUTH=enforce` un `X-Bext-Sdk-Token` HMAC en plus de l'App ID.
  ShellDeck ne reçoit pas ce jeton. Exposer seulement
  `127.0.0.1:<port> → 127.0.0.1:80` produirait donc une interface en 403 ;
  désactiver la garde serveur serait une régression de sécurité.
- **2026-08-18 — PROD-002 conservé bloqué.** Le client ne doit pas inventer
  une gestion locale des tags qui divergerait de la source Issues distante.
- **2026-08-21 — PROD-003 ouvert bloqué.** Relevé pendant l'audit de navigation
  comme un écart d'harmonisation entre deux files sœurs. Le client ne doit pas
  inventer des compteurs : `IssueList` n'expose ni total ni décompte par statut,
  et `refresh_issues` envoie le filtre au serveur, donc `issues_list` ne
  contient déjà que la tranche active.
