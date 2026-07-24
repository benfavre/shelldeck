# ShellDeck AI Companion — contrat produit et technique

> Document spécialisé de référence pour l'IA transversale.
>
> Les priorités sont suivies dans [`companion.md`](companion.md). Ce fichier
> décrit les invariants à préserver ; ce n'est plus un journal de commits.

## État vérifié au 2026-07-24

Les fondations et les phases fonctionnelles prévues sont livrées dans le
périmètre de sécurité actuel :

- assistant général multi-tour et conversations locales persistantes ;
- suggestions intégrées dans Support, Demandes, Scripts et Terminal ;
- brouillons et tâches durables avec reprise, arrêt et suppression ;
- résultats structurés validés avant application ;
- actions exécutables via `AiActionPlan`, confirmation et audit expurgé ;
- policies persistées par capacité ;
- triage Support explicite ou automatique après clic ;
- diagnostics Terminal bornés et séquentiels via OSC 133 ;
- actions contextuelles depuis l'activité récente ;
- notifications de fin et centre de tâches.

Deux écarts restent ouverts :

- tags de demandes, bloqués par l'absence de mutation dans l'API Issues ;
- couverture GPUI P0/P1 encore rouge dans l'inventaire de tests.

## Vision

L'IA est une capacité native des workflows ShellDeck, pas seulement un chat.
Elle combine :

1. un assistant général avec contexte borné ;
2. des suggestions au plus près des écrans ;
3. des actions typées, préparées ou exécutées selon une policy explicite.

Le workflow manuel doit toujours rester disponible lorsque le provider est
absent ou en erreur.

## Invariants produit

- L'IA est opt-in globalement et par surface.
- Le contexte provient de l'élément choisi, jamais d'une collecte implicite.
- Chaque résultat possède un type validable : texte, formulaire, diff,
  commande, triage, plan ou action.
- Accepter une suggestion n'équivaut pas à exécuter une action.
- Une cible, un rôle ou une permission est revérifié au dernier clic.
- Les tâches longues exposent progression, résultat et arrêt.
- Les erreurs provider restent visibles et récupérables.

## Autonomie

`AiAutonomyLevel` définit trois comportements exécutables :

| Niveau | Effet |
|---|---|
| `Preparation` | produit ou remplit un brouillon sans action finale |
| `Confirmation` | prépare un plan et exige la confirmation dédiée |
| `Automatic` | peut sauter la confirmation uniquement pour un risque faible ou modéré |

Les actions à risque élevé — notamment Terminal, Script et Fleet — restent
confirmées même si la policy demande `Automatic`.

Les policies persistées couvrent actuellement :

- envoi Support ;
- triage Support ;
- exécution Terminal ;
- exécution Script ;
- envoi Jean ;
- dispatch Fleet.

Le niveau automatique n'autorise jamais une collecte générale, une élévation
de privilèges ou un contournement des rôles.

## Pattern d'interaction

### Suggestion

1. L'utilisateur choisit une action IA.
2. ShellDeck construit un contexte borné pour la cible exacte.
3. Le résultat est affiché et peut être régénéré ou mis en attente.
4. Accepter remplit uniquement le composer, formulaire ou buffer concerné.

### Action exécutable

1. ShellDeck construit un `AiActionPlan` typé.
2. Le plan fixe capacité, cible, payload en mémoire, risque, autonomie,
   provider, modèle et délai.
3. L'interface présente le contenu complet et la cible.
4. La dernière action revérifie l'état courant avant mutation.
5. L'audit persiste les métadonnées, jamais le prompt, la commande ou le
   contenu sensible.

## Surfaces livrées

| Surface | Capacités principales |
|---|---|
| Support | réponse préparée, résumé, triage priorité/assignation, envoi confirmé |
| Demandes | brouillon structuré, réponse, résumé, triage, conversion Support, dispatch |
| Scripts | génération, explication, revue, correction avec diff, exécution confirmée |
| Terminal | commande préparée, explication, demande depuis contexte, diagnostic séquentiel |
| Jean/Fleet | prompt, envoi et dispatch contrôlés |
| Connexions/tunnels | nommage contextuel sans persistance automatique |
| Activité | analyse explicite de l'événement sélectionné |

Les tags de demandes ne sont pas livrés : l'API serveur doit d'abord exposer
lecture, validation, mutation et filtrage.

## Architecture actuelle

- `shelldeck_core::ai::AiClient` est l'unique porte d'entrée provider.
- `AiCapability` nomme les workflows et leurs contrats.
- Les parseurs core valident les sorties structurées.
- `AiTaskStore` persiste tâches et anciens brouillons compatibles.
- `AiCompanionController` possède l'assistant global utilisable sans
  `Workspace`.
- `Workspace` orchestre les workflows contextuels, confirmations, exécutions,
  audit et routage vers les surfaces.
- Les vues émettent des intentions ; elles ne possèdent ni provider ni secret.

## Sécurité et confidentialité

Les règles de [`.agents/ai.md`](../../.agents/ai.md) sont normatives :

- clés API dans le keychain OS uniquement ;
- contextes bornés et champs sensibles expurgés ;
- aucun rôle ou contrôle serveur contourné ;
- cible locale ou distante toujours visible ;
- commandes mutantes, interactives, élevées ou non bornées refusées par le
  plan de diagnostic ;
- annulation et timeout arrêtent le travail correspondant ;
- audit sans payload exécutable ni secret.

Le triage Support en mode automatique reste déclenché par un clic explicite.
Le polling Support ne lance aucun appel provider silencieux.

## Vérification

L'audit du 2026-07-24 confirme notamment :

- `AiTask`/`AiTaskStatus`/`AiTaskStore` et leur migration depuis les brouillons ;
- `AiActionPlan`, les dispositions d'autonomie et la validation
  capacité/payload ;
- les parseurs de nom, demande, triage, diff et diagnostic ;
- les marqueurs `CommandFinished` OSC 133 et la séquence Terminal ;
- le badge, les notifications et les actions du centre de tâches.

Les contrats core importants sont Green. Les scénarios de câblage GPUI encore
`to write` restent recensés dans
[`tests-ui-and-app.md`](../testing/tests-ui-and-app.md), notamment disponibilité
des affordances, protection des cibles périmées, confirmation finale, routage
du centre de tâches et séquence visuelle des diagnostics.

## Hors scope

- entraînement d'un modèle ShellDeck ;
- télémétrie silencieuse du contenu utilisateur ;
- autonomie générale sans policy par capacité ;
- exécution cachée sans statut, journal ou arrêt ;
- remplacement des permissions normales de ShellDeck ou des services distants.
