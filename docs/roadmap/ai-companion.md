# ShellDeck AI Companion - cadrage produit et technique

> Statut: cadrage cible, document vivant.
>
> Objectif: faire de l'IA une capacite native de ShellDeck, integree dans
> chaque workflow utile, et pas seulement un chat lateral. Le companion peut
> proposer, preparer, appliquer et, lorsque l'utilisateur l'autorise,
> executer des actions.

## 1. Vision

ShellDeck AI Companion repose sur trois experiences complementaires:

1. **Assistant general**
   Une sheet disponible depuis la toolbar, la palette et un raccourci. Elle
   permet de discuter librement avec le contexte de l'ecran courant.
2. **Actions integrees aux ecrans**
   Des boutons IA apparaissent directement pres des controles concernes:
   composer Support, editeur de scripts, terminal, demandes, Jean, connexions,
   tunnels et activite recente.
3. **Agent operationnel**
   Certaines actions peuvent etre executees par ShellDeck: remplir un champ,
   modifier un script, lancer une commande, envoyer une reponse, creer une
   demande ou dispatcher un job. Le niveau d'autonomie est explicite et
   configurable.

La sheet generale n'est donc pas le produit principal. Elle est le point
d'entree transversal; les workflows integres sont l'experience quotidienne.

## 2. Principes produit

- L'IA est opt-in globalement et par surface.
- Une action integree n'apparait que si son backend est utilisable et si la
  surface correspondante est activee.
- Le contexte vient de l'element visible ou selectionne, pas d'une collecte
  globale implicite.
- Chaque resultat a un type: texte, patch, commande, formulaire, classement,
  plan ou action executable.
- Les propositions peuvent etre modifiees avant application.
- Les brouillons peuvent etre mis en attente et repris plus tard.
- Les actions automatiques sont autorisees, mais seulement dans un perimetre
  declare et avec une trace exploitable.
- Un echec du provider ne doit jamais bloquer le workflow manuel normal.

## 3. Niveaux d'autonomie

Chaque capacite declare son niveau maximal. L'utilisateur peut imposer un
niveau plus restrictif globalement ou par surface.

| Niveau | Nom | Comportement |
|---|---|---|
| 0 | Desactive | Aucun bouton ni appel IA sur la surface. |
| 1 | Suggestion | Produit un brouillon; aucune mutation. |
| 2 | Preparation | Peut remplir un composer, un formulaire ou un editeur, sans valider l'action finale. |
| 3 | Confirmation | Prepare une action executable et demande une confirmation explicite. |
| 4 | Automatique borne | Execute sans confirmation uniquement les actions autorisees par une policy precise. |

Exemples:

- Support en niveau 2: l'IA insere une reponse, l'agent humain clique Envoyer.
- Terminal en niveau 3: l'IA affiche la commande, les effets et la cible; le
  clic Executer lance la commande.
- Triage en niveau 4: l'IA peut appliquer automatiquement tags et priorite sur
  les tickets entrants, mais ne peut pas envoyer de message.

Le niveau 4 n'est jamais une permission generale. Il est accorde a une
capacite nommee, sur une surface nommee, avec des limites nommees.

## 4. Pattern d'interaction commun

### 4.1 Suggestion contextuelle

Utilise pour les reponses, resumes, tags, priorites et noms.

1. L'utilisateur clique le bouton IA sur l'ecran.
2. Une popup affiche le chargement puis la suggestion.
3. La suggestion est editable.
4. Actions standard:
   - **Accepter**: applique ou insere le resultat.
   - **Mettre en attente**: conserve le brouillon.
   - **Regenerer**: relance avec des instructions complementaires.
   - **Annuler**: ferme et supprime le brouillon courant.

Accepter n'est pas toujours Executer. Sur Support, cela remplit le composer;
sur un nom de connexion, cela remplit le champ; sur des tags, cela les applique.

### 4.2 Generation depuis instructions

Utilise pour les scripts, commandes, demandes et jobs Jean.

1. Un bouton **Generer avec l'IA** ouvre un dialogue d'instructions.
2. Le dialogue collecte uniquement les parametres utiles: objectif, langage,
   cible, contraintes, niveau de risque, environnement.
3. L'IA genere un resultat structure.
4. L'utilisateur peut modifier, regenerer, mettre en attente ou appliquer.
5. Si le resultat est executable, ShellDeck passe par le controle d'autonomie
   de la capacite avant execution.

### 4.3 Action directe

Utilise pour les automatismes autorises: triage, renommage, creation de
demande, lancement de diagnostic, envoi ou dispatch.

Avant execution, ShellDeck construit un `AiActionPlan` qui contient:

- la capacite demandee;
- la cible exacte;
- les mutations ou commandes prevues;
- le niveau de risque;
- le niveau d'autonomie requis;
- les permissions et limites applicables;
- les possibilites d'annulation ou de compensation.

## 5. Integrations par surface

### Support

Emplacement: pres du composer et dans la toolbar du ticket.

- **Proposer une reponse**: popup editable.
  - Accepter insere dans le composer.
  - Mettre en attente conserve le brouillon pour ce ticket.
  - En niveau 3/4, l'envoi peut etre propose ou automatise selon policy.
- **Resumer le ticket**: resume epingle ou temporaire.
- **Trier**: categorie, tags, priorite, assignation suggeree.
- **Prochaine action**: diagnostic ou question de clarification.
- **Convertir en demande**: pre-remplit la demande depuis le fil.

### Demandes / issues

Emplacement: creation, detail et barre staff.

- Ameliorer titre et description.
- Extraire reproduction, resultat attendu et environnement.
- Suggérer tags, priorite et assignation.
- Resumer les commentaires.
- Generer une reponse au demandeur.
- Preparer ou lancer un dispatch Fleet.
- Preparer un push GitHub; execution selon permissions staff et autonomie.

### Scripts

Emplacement: toolbar de l'editeur et menu contextuel.

- **Generer avec l'IA** depuis un dialogue d'instructions.
- Expliquer le script et ses prerequis.
- Auditer securite, portabilite et idempotence.
- Corriger avec apercu du diff.
- Convertir vers un autre `ScriptLanguage`.
- Completer une section selectionnee.
- Executer le script genere selon le niveau d'autonomie.

Un remplacement de script existant doit toujours presenter un diff. Une
execution automatique doit conserver le script exact et les parametres dans
le journal d'action.

### Terminal

Emplacement: toolbar, selection et etat de commande echouee.

- Generer une commande depuis une intention.
- Expliquer la sortie ou la selection.
- Proposer des diagnostics apres une erreur.
- Transformer une erreur en demande.
- Executer une commande proposee.
- Enchainer un plan de diagnostic borne, avec arret manuel permanent.

Les commandes destructives, les elevations de privilege et les acces a des
secrets exigent une policy specifique. Le terminal affiche toujours la cible,
la commande exacte et l'etat d'execution.

### Jean / Fleet

Emplacement: composer Jean, tickets et jobs Fleet.

- Transformer une intention en prompt Jean structure.
- Resumer un job ou expliquer un echec.
- Choisir une instance compatible.
- Dispatcher un job.
- Confirmer, rejeter, relancer ou annuler selon autonomie.

### Connexions, sites et tunnels

Emplacement: formulaires et menus contextuels.

- Suggérer un alias, un groupe ou des tags.
- Suggérer un nom de tunnel.
- Expliquer une configuration SSH.
- Preparer une connexion ou un port-forward.
- Lancer une verification de connectivite borne.

### Activite recente

Emplacement: toolbar de la vue Activite et Dashboard.

- Resumer la journee ou la semaine.
- Identifier les echecs recurrents.
- Proposer les prochaines actions.
- Creer une demande ou un script depuis un evenement.
- Relancer une action connue si la policy l'autorise.

## 6. Brouillons et taches en attente

Les suggestions mises en attente doivent survivre a la fermeture d'une popup
et au redemarrage de l'application.

Modele cible:

```rust
struct AiDraft {
    id: Uuid,
    capability: AiCapability,
    surface: AiSurface,
    target: AiTarget,
    provider: AiBackend,
    status: AiDraftStatus,
    instructions: String,
    result: AiArtifact,
    created_at: i64,
    updated_at: i64,
}
```

Statuts minimaux:

- `Generating`
- `Ready`
- `Pending`
- `Applied`
- `Executing`
- `Succeeded`
- `Failed`
- `Cancelled`

Une future vue **IA / En attente** regroupe les brouillons et actions. Les
ecrans affichent aussi leurs brouillons lies localement.

## 7. Contrats techniques

### Provider

Tous les appels passent par `shelldeck_core::ai::AiClient`. Les vues ne
lancent jamais directement Claude, Codex, Aider, OpenAI ou Anthropic.

### Capacites

Le provider ne doit pas recevoir des prompts ad hoc disperses dans les vues.
Chaque workflow est une capacite nommee:

```rust
enum AiCapability {
    SupportReply,
    SupportTriage,
    IssueDraft,
    ScriptGenerate,
    ScriptReview,
    TerminalCommand,
    TerminalDiagnose,
    JeanDispatch,
    Naming,
    ActivitySummary,
}
```

Chaque capacite possede:

- un constructeur de contexte;
- un schema d'instructions;
- un schema de sortie;
- un niveau de risque;
- les actions applicables;
- le niveau d'autonomie maximal;
- une fonction d'application ou d'execution.

### Sorties structurees

Pour les workflows integres, preferer une sortie JSON validee plutot qu'un
texte libre. Exemples:

- `SuggestedReply { body, confidence, missing_information }`
- `ScriptDraft { language, body, explanation, warnings }`
- `CommandPlan { command, cwd, explanation, risk, requires_sudo }`
- `TriageProposal { priority, tags, assignee, rationale }`

Le texte libre reste adapte a la sheet generale.

### Orchestration

`Workspace` reste proprietaire:

- de la configuration IA active;
- des jobs en cours;
- du registre de brouillons;
- des confirmations;
- du journal d'execution;
- du routage vers les surfaces.

Les vues emettent des intentions typees et recoivent des snapshots; elles ne
possedent ni client provider ni cle API.

## 8. Permissions, securite et audit

- Le contrat courant de [`.agents/ai.md`](../../.agents/ai.md) exige une action
  utilisateur explicite et interdit l'execution depuis un resultat IA. Il
  reste la regle d'implementation pour les phases 0 a 2. Les phases 3 et 4
  exigent d'abord une evolution relue de ce contrat vers le modele de policies
  decrit ici, avec ses tests de permissions et d'audit.
- Les cles API restent dans le keychain OS.
- Les contextes sont bornes et les champs sensibles sont redactes.
- Les roles ShellDeck continuent de controler les surfaces accessibles.
- Une action IA ne peut jamais contourner une permission applicative normale.
- Les actions terminal/script declarent leur cible locale ou distante.
- Un bouton **Arreter** est visible pendant toute execution longue.
- Les timeouts et annulations tuent le processus enfant quand necessaire.
- Toute action appliquee ou executee produit une entree d'audit:
  - acteur;
  - capacite;
  - cible;
  - provider/model;
  - autonomie utilisee;
  - resultat;
  - horodatage.
- Les secrets, tokens et mots de passe ne sont jamais journalises.

## 9. Configuration cible

Settings -> IA:

- activation globale;
- provider et modele;
- test de connexion;
- activation par surface;
- niveau d'autonomie global par defaut;
- override par surface;
- permissions fines par capacite;
- confirmation obligatoire pour commandes destructives;
- conservation des brouillons;
- historique/audit et purge.

Exemple de policy:

```toml
[ai.autonomy]
default = "suggestion"
support = "preparation"
scripts = "confirmation"
terminal = "confirmation"

[ai.permissions]
support_send = false
terminal_execute = true
terminal_sudo = false
script_execute = true
issue_dispatch = false
```

Les noms exacts du format TOML seront fixes lors de l'implementation; ce bloc
cadre le comportement, pas encore le contrat de serialisation.

## 10. Phasage

### Phase 0 - fondation existante

- `AiClient` provider-neutral.
- Detection CLI et keychain API.
- Test de connexion.
- Contextes bornes/redactes.
- Sheet generale.
- Toggles par surface.

### Phase 1 - suggestions integrees

- Composant commun de popup suggestion.
- Support: reponse, resume, triage.
- Scripts: dialogue de generation, explication, review.
- Terminal: expliquer erreur, generer commande.
- Brouillons en attente persistants.

Etat au 2026-07-16:

- Fait: composant commun avec chargement, acceptation, regeneration,
  annulation et mise en attente; les brouillons applicables sont editables,
  les analyses sont en lecture seule dans une zone scrollable.
- Fait: assistant general multi-tour avec conversations persistantes locales,
  historique actif/archive, suppression confirmee et limite aux 100 discussions
  les plus recentes.
- Fait: sheet conversationnelle a deux panneaux, fil et historique scrollables,
  composer fixe, actions d'archivage/restauration et suppression accessibles.
- Fait: Support, proposition de reponse puis insertion dans le composer sans
  envoi.
- Fait: Scripts, generation depuis instructions puis insertion dans le buffer
  d'edition sans sauvegarde ni execution.
- Fait: brouillons en attente persistants, restaures par cible et limites aux
  100 plus recents.
- Fait: Terminal, génération d'une commande avec insertion sans exécution,
  diagnostic depuis la sélection ou la sortie visible, et brouillons en attente.
- Fait: Support, résumé et triage depuis la toolbar du ticket; les analyses
  peuvent être ajustées par instruction/régénération, mises en attente et copiées.
- Fait: Scripts, explication et revue sécurité/portabilité/idempotence depuis la
  toolbar; les analyses sont scrollables, persistantes et copiées explicitement.

La phase 1 est terminée.

### Phase 2 - application dans les ecrans

- Fait partiellement: insertion dans le composer Support, l'editeur de script et
  le formulaire Nouveau/Modifier un script.
- Fait: la génération depuis le formulaire applique un résultat structuré et
  validé (nom, description, langage, catégorie, corps), avec une régénération
  corrective si le provider ne respecte pas le schéma. La cible et le host
  restent sous contrôle explicite de l'utilisateur.
- Fait: après la dernière exécution échouée du script sélectionné, la barre de
  sortie propose une correction IA basée sur le code de sortie et le log. La
  proposition ouvre le buffer d'édition non sauvegardé et n'est jamais exécutée
  automatiquement.
- Fait: les remplacements de scripts générés ou corrigés affichent un diff
  scrollable et borné contre le corps actuellement sauvegardé avant insertion
  dans le buffer non sauvegardé.
- Fait: Demandes, réponse, résumé et triage directement depuis le détail; la
  réponse remplit uniquement le composer et les analyses restent copiées
  explicitement. Les trois capacités restaurent leurs brouillons par demande.
- Fait: le formulaire Nouvelle demande propose une préparation IA structurée
  (titre, description avec contexte/reproduction/résultat attendu/environnement,
  priorité) dans un panneau replié par défaut. Le résultat validé remplit
  uniquement le brouillon local, avec une tentative corrective si le provider
  ne respecte pas le schéma; aucune demande n'est créée automatiquement et les
  hosts ne sont utilisés que comme contexte.
- Fait: nommage contextuel des scripts, sessions terminal, tunnels et nouvelles
  demandes. Chaque bouton ouvre le workflow IA commun, valide un nom JSON court
  sur une ligne et ne remplace le champ local qu'après acceptation explicite.
- Fait: le triage Demandes produit une proposition structurée, validée puis
  réparée une fois si nécessaire. Le staff prévisualise priorité et assignation
  en avant/après, la justification et les prochaines actions; un second clic
  applique uniquement les changements confirmés. La cible, le rôle et l'agent
  proposé sont revérifiés avant mutation.
- Restant: tags de demandes, bloqués tant que l'API Issues n'expose pas de
  mutation dédiée.
- Fait: convertir un ticket Support ouvre un brouillon Nouvelle demande
  prérempli et marqué `source=support`; aucune création n'a lieu avant le clic
  explicite sur Créer et le panneau IA peut encore ajuster le brouillon.
- Fait: la toolbar Terminal ouvre un brouillon de demande `source=shelldeck`
  depuis la sélection ou, à défaut, les 120 lignes visibles bornées, avec la
  session et le répertoire; aucune demande n'est créée automatiquement.

### Phase 3 - executions confirmees

- Fait: `AiActionPlan` type la capacité, l'action, le risque, la cible exacte,
  le provider/modèle, le délai et le payload en mémoire. Son détail d'audit est
  expurgé du contenu, des commandes et des prompts.
- Fait: un dialogue adabraka commun affiche cible, risque et contenu complet;
  l'action nécessite un bouton Exécuter/Envoyer distinct puis une seconde
  confirmation. La cible et les permissions sont revérifiées au dernier clic.
- Fait: les commandes Terminal confirmées sont soumises à la session exacte;
  les scripts générés/corrigés s'exécutent sans sauvegarder le brouillon et
  réutilisent la sortie et le bouton Arrêter existants.
- Fait: réponse Support, envoi à Jean et dispatch d'une demande vers Fleet
  passent par le même plan confirmé; les rôles, tickets, demandes et instances
  sont revérifiés avant l'appel réseau.
- Fait: les scripts IA sont suivis jusqu'au succès, échec, arrêt ou timeout de
  30 minutes. Les appels réseau gardent leurs timeouts clients. L'audit durable
  journalise l'acteur du compte ou la session locale, capacité, cible, provider/modèle,
  risque, délai et statut, jamais le payload. Une commande PTY reste suivie
  manuellement et interrompue avec Ctrl+C, sa fin n'étant pas observable de
  façon fiable par ShellDeck.

La phase 3 est terminée.

### Phase 4 - autonomie bornee

- Policies par capacite.
- Automatismes Support/triage.
- Plans de diagnostic terminal.
- File de taches IA et centre En attente.
- Notifications de progression et resultats.

## 11. Criteres d'acceptation transversaux

- Desactiver l'IA masque tous les affordances sans casser les workflows.
- Desactiver une surface ne masque que ses actions.
- Chaque bouton utilise le bon element selectionne comme cible.
- Une suggestion peut etre acceptee, modifiee, mise en attente ou annulee.
- Un brouillon en attente est restaurable apres redemarrage.
- Toute mutation affiche clairement ce qui sera change.
- Toute execution respecte les permissions et le niveau d'autonomie.
- Une action annulee ou obsolete ne peut pas ecraser un contexte plus recent.
- Les erreurs provider sont visibles et recuperables.
- Les actions longues affichent progression, spinner et bouton Arreter.
- Les sorties et controles restent utilisables avec le scroll et les themes
  clair/sombre.

## 12. Hors scope initial

- Entrainement d'un modele ShellDeck.
- Telemetrie silencieuse du contenu utilisateur.
- Autonomie generale sans policies par capacite.
- Contournement des roles, confirmations serveur ou controles SSH.
- Execution cachee sans statut, journal ou moyen d'arret.
