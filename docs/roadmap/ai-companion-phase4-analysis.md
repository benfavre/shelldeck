# ShellDeck AI Companion - analyse de pertinence de la phase 4

> Statut: recommandation produit et technique.
>
> Objectif: evaluer les fonctions restant a developper pour terminer le
> companion IA et definir un ordre d'implementation coherent avec les usages
> de ShellDeck.

## 1. Synthese

Toutes les fonctions restantes n'ont pas la meme pertinence et certaines
doivent etre simplifiees pour eviter une configuration trop complexe.

| Fonction | Pertinence | Recommandation |
|---|---:|---|
| Policies par capacite | Tres forte | Fait pour les capacites executables |
| Automatismes Support et triage | Forte, mais ciblee | Fait apres action explicite |
| Plans de diagnostic Terminal | Tres forte | Fait avec completion OSC 133 |
| Centre En attente | Tres forte | Fait |
| Notifications et progression | Indispensable | Fait |
| Tags de demandes | Moyenne a forte | Attendre la mutation dans l'API Issues |
| Reactions IA aux evenements | Moyenne | Fait depuis les lignes Activite |

## 2. Policies d'autonomie par capacite

Les policies sont pertinentes parce que ShellDeck peut maintenant executer
des commandes, des scripts, des reponses Support et des dispatchs. Sans elles,
chaque action demande toujours une confirmation et l'autonomie reste
incomplete.

Le modele initial est cependant trop complexe: niveau global, niveau par
surface, permissions par capacite et exceptions de risque rendraient les
Settings difficiles a comprendre.

La configuration devrait proposer une seule regle par capacite:

- `Desactive`;
- `Preparer`;
- `Confirmer`;
- `Automatique`.

Certaines protections doivent rester non desactivables:

- `sudo`, commandes destructives et acces aux secrets: confirmation
  obligatoire;
- envoi de messages Support: confirmation par defaut;
- script ou terminal automatique: uniquement sur des cibles explicitement
  autorisees;
- aucune policy ne peut contourner les roles User, Support et Dev.

**Decision:** indispensable, avec une configuration volontairement reduite.

## 3. Automatismes Support et triage

L'automatisation est pertinente pour les taches repetitives:

- resumer un nouveau ticket;
- detecter sa priorite et sa categorie;
- proposer ou appliquer une assignation;
- identifier les informations manquantes;
- reperer les doublons ou incidents similaires.

Les actions suivantes ne doivent pas etre automatiques par defaut:

- envoyer une reponse au client;
- resoudre un ticket;
- dispatcher vers Fleet sans limites explicites.

ShellDeck etant une application desktop, ces automatismes ne fonctionnent que
lorsque l'application est ouverte. Le perimetre livre part d'un clic explicite
sur Trier; le polling Support ne declenche donc aucun appel provider et ne peut
pas retraiter un ticket en boucle.

**Decision:** tres utile pour le triage de metadonnees; l'envoi automatique de
reponses reste hors du comportement par defaut. La proposition JSON validee,
l'annuaire des agents, les mutations priorite/assignation et la policy dediee
sont livres. `Automatique` applique le triage apres le clic explicite; le
declenchement silencieux en arriere-plan reste volontairement exclu.

## 4. Plans de diagnostic Terminal

Cette fonction est particulierement coherente avec le role de ShellDeck.

Un plan de diagnostic doit suivre ce flux:

1. L'IA analyse une erreur ou une sortie selectionnee.
2. Elle prepare un plan court de diagnostics.
3. ShellDeck affiche chaque commande, sa cible et son objectif.
4. Les commandes autorisees s'executent successivement.
5. La sortie d'une etape alimente la suivante.
6. L'utilisateur peut arreter le plan a tout moment.

**Livre (2026-07-21):** le produit genere un JSON strict de 1
a 5 etapes, valide une allowlist de commandes en lecture seule, puis affiche
chaque commande avec son objectif. Chaque etape est une action Terminal a
risque eleve, revalidee et confirmee separement; Ctrl+C reste le mecanisme
d'arret. Le plan complet instrumente les shells sans integration native,
transforme `OSC 133;D[;exitcode]` en evenement de session deduplique et capture
une sortie bornee. L'etape suivante n'est preparee qu'apres cet evenement,
conserve sa confirmation et un code non nul arrete la sequence.

Le modele ne doit pas pouvoir declarer seul qu'une commande est sans danger.
L'execution automatique doit reposer sur des operations reconnues et bornees.
Toute commande inconnue, destructive, interactive ou avec elevation repasse
en confirmation.

Chaque plan doit avoir une limite d'etapes, de duree et de volume de sortie
pour eviter les boucles et les consommations non bornees.

**Decision:** livre en etendant le protocole `OSC 133` existant jusqu'au
Workspace; aucune temporisation n'est utilisee pour supposer une completion.

## 5. File de taches et centre En attente

Le centre de taches devient necessaire maintenant que les brouillons, scripts,
commandes et actions longues existent. Le stockage actuel des brouillons devra
evoluer pour representer le cycle de vie complet d'une tache.

Le centre doit regrouper:

- brouillons prets ou mis en attente;
- generations en cours;
- actions attendant une confirmation;
- executions actives;
- resultats reussis, echoues ou annules;
- actions pour reprendre, ouvrir la cible, relancer, arreter et supprimer.

Il n'est pas necessaire d'ajouter un nouvel ecran permanent a la navigation.
Un onglet `Taches` dans la sheet IA, accompagne d'un badge sur son bouton
global, limite l'encombrement. Les ecrans conservent uniquement leurs taches
contextuelles.

**Decision:** fondation necessaire avant toute autonomie reelle.

## 6. Notifications de progression et resultats

La progression et les notifications font partie du systeme de taches et ne
doivent pas former un deuxieme mecanisme parallele.

Comportement recommande:

- spinner local pendant une generation courte;
- badge global lorsqu'une tache continue ailleurs;
- toast uniquement pour un succes, un echec ou une confirmation requise;
- notification systeme seulement lorsque ShellDeck n'est pas au premier plan;
- aucune notification a chaque etape d'un diagnostic.

Chaque notification doit ouvrir directement la tache ou sa cible.

**Decision:** indispensable, a developper avec le centre de taches.

## 7. Tags de demandes

Les tags sont utiles pour categoriser les incidents, filtrer les demandes,
identifier des problemes recurrents, preparer une synchronisation GitHub et
ameliorer le triage automatique.

Ils ne doivent pas etre implementes uniquement en local. Sans mutation cote
API, ShellDeck creerait un etat incoherent avec le serveur. L'API Issues doit
d'abord fournir la lecture, la mutation et la validation des tags, idealement
avec leur filtrage.

**Decision:** pertinent mais non bloquant pour terminer le companion IA.

## 8. Ordre d'implementation recommande

1. Modele unifie de taches IA et centre En attente. **Fait.**
2. Progression, arret, resultats et notifications. **Fait.**
3. Policies simples par capacite. **Fait pour les capacites executables.**
4. Plans de diagnostic Terminal. **Fait avec orchestration OSC 133.**
5. Triage Support automatique borne. **Fait apres clic explicite; aucun polling IA silencieux.**
6. Tags apres evolution de l'API Issues.
7. Actions IA depuis l'activite recente. **Fait avec declenchement explicite.**

La phase 4 reste donc pertinente avec deux restrictions importantes:
l'automatisation generale des reponses Support est ecartee et la configuration
d'autonomie doit rester simple.
