# Pièces jointes image dans les demandes et tickets ShellDeck

## Objectif

Permettre à un utilisateur d'ajouter des images et des captures d'écran :

- lors de la création d'une demande ;
- dans un commentaire ultérieur ;
- dans une réponse ou une note interne d'un ticket Support ;
- depuis un fichier, le presse-papiers ou un glisser-déposer.

Le stockage d'images existant de [Inklura Share](https://share.inklura.fr/) doit
être réutilisé, avec un chemin d'intégration spécifique aux demandes ShellDeck.

L'utilisateur doit pouvoir fournir une image de quatre façons équivalentes :

1. coller une URL d'image ;
2. coller les octets d'une image avec `Ctrl+V` ou `Cmd+V` ;
3. glisser-déposer un fichier ;
4. lancer depuis ShellDeck une sélection de zone sur l'écran.

## Clarification sur ShareX

ShareX n'est pas une dépendance envisagée pour ShellDeck et l'utilisateur
n'aura pas besoin de l'installer.

Le nom apparaît dans cette étude uniquement parce que l'endpoint existant
`POST https://share.inklura.fr/api/upload` a été conçu pour le client ShareX :

- il accepte une clé `isk_...` présentée en Bearer ;
- son champ multipart s'appelle `file` ;
- sa réponse JSON contient les champs attendus par ce type de client (`url`,
  `viewer`, `thumb`, `deletion_url`).

ShellDeck ne doit pas demander ni stocker cette clé ShareX. Il doit utiliser
la session Manage déjà active et obtenir une autorisation d'upload courte et
limitée à la demande concernée.

## État actuel

### Demandes et tickets ShellDeck

Le contrat transporte maintenant des pièces jointes structurées sur `Issue`,
`IssueComment` et `SupportMessage`. Les octets ne transitent jamais dans ces
objets : ils sont envoyés directement à Share, puis Manage et support-prism ne
conservent que l'identifiant, les métadonnées et les URLs-capacités.

### Inklura Share

Inklura Share fournit déjà :

- un endpoint multipart ;
- un stockage binaire Bext ;
- des métadonnées et des index en KV ;
- des pages de visualisation ;
- une limite de 9 Mo par fichier (marge multipart sous le plafond Bext de 10 Mio) ;
- la suppression et le renommage des fichiers d'une galerie utilisateur.

L'endpoint live répond correctement et annonce :

```json
{
  "service": "Inklura Share upload endpoint",
  "method": "POST",
  "field": "file",
  "auth": "Bearer isk_... (clé API ShareX)"
}
```

### Capacités GPUI disponibles

Le fork GPUI utilisé par ShellDeck sait déjà :

- lire les images du presse-papiers sur Linux, macOS et Windows ;
- ouvrir un sélecteur natif de fichiers ;
- recevoir des fichiers externes par glisser-déposer ;
- décoder et afficher des images locales ou distantes ;
- exposer une API optionnelle de capture d'écran.

L'API de capture GPUI n'est actuellement pas activée dans ShellDeck. Elle peut
capturer un écran sous macOS, Windows et X11, mais son backend Wayland est
explicitement incomplet. Elle ne fournit pas non plus à elle seule le sélecteur
de rectangle attendu. La capture de zone doit donc passer par une abstraction
ShellDeck avec une implémentation native par plateforme.

## Pourquoi l'endpoint Share actuel ne doit pas être utilisé tel quel

Les uploads actuels sont conçus pour le partage volontaire par lien :

```text
https://share.inklura.fr/u/<identifiant>.<extension>
```

Ce fichier est servi statiquement sans vérification de la session Manage. Ce
modèle est pratique pour partager une image, mais une demande support peut
contenir des informations plus sensibles : URLs d'administration, noms de
clients, adresses IP, données personnelles ou informations d'infrastructure.

Les points à corriger pour l'usage ShellDeck sont :

- les identifiants actuels ne contiennent que 48 bits aléatoires ;
- l'upload fait confiance au type MIME déclaré ;
- SVG est accepté alors qu'il peut contenir du contenu actif ;
- aucun quota utilisateur ou tenant n'apparaît dans l'implémentation actuelle ;
- aucune politique de rétention ou de nettoyage des fichiers orphelins n'est
  définie ;
- le cookie `share_session` devrait être `HttpOnly` avant de continuer à
  servir du contenu utilisateur sur le même domaine.

Le bridge V8 de PRISM ne peut pas restituer de façon fiable un corps binaire.
Le stockage utilise donc actuellement une écriture Bext dédiée, puis un chemin
statique pour la lecture. Une protection stricte des téléchargements devra être
faite dans la couche native Bext ou dans le proxy, et non en faisant transiter
l'image dans une route TypeScript classique.

## Architecture recommandée

```text
ShellDeck
   │ token Manage existant
   ▼
Manage /issues ou /support ──► ticket d'upload opaque, court et à usage unique
   │
   ▼
Share /api/attachments/upload
   │ stockage Bext
   ▼
reçu opaque ──► Manage ──► rattachement à la demande/commentaire/ticket
```

### Flux d'une nouvelle demande

1. L'utilisateur sélectionne ou colle les images. Elles restent localement
   dans une file d'attente avec leurs miniatures.
2. ShellDeck crée la demande texte avec le token Manage existant.
3. Pour chaque image, ShellDeck demande à Manage un ticket d'upload lié à
   l'utilisateur, au tenant et à l'identifiant de la demande.
4. ShellDeck envoie directement l'image à Inklura Share avec ce ticket.
5. Share renvoie un reçu-capacité opaque associé aux métadonnées du fichier.
6. ShellDeck transmet le reçu à Manage, qui le valide puis rattache la pièce
   jointe à la demande.
7. Si un upload échoue, la demande reste créée et l'interface permet de
   réessayer.

### Flux d'un commentaire

Le même mécanisme s'applique aux commentaires. Le serveur doit accepter :

- texte uniquement ;
- texte et images ;
- images uniquement.

### Flux d'un ticket Support

Le même ticket d'upload porte `target_kind: "support"` et l'identifiant du
thread. Après validation du reçu, l'image est conservée comme donnée structurée
sur le message. La réponse envoyée au canal d'origine reçoit également le lien
de visualisation Share : cela fonctionne uniformément pour email, livechat,
Manage et SMS sans exiger que chaque bridge transporte des octets binaires.
Une note interne conserve les images dans le thread mais ne route rien au client.

### Propriétés du ticket d'upload

Le ticket devrait :

- expirer après une ou deux minutes ;
- contenir 256 bits aléatoires et être stocké côté Manage ;
- contenir le tenant, l'utilisateur, le type de cible et son identifiant ;
- limiter le nombre d'octets et le type MIME ;
- être utilisable une seule fois ;
- ne jamais contenir le token Manage lui-même.

Le reçu renvoyé par Share contient également 256 bits aléatoires. Manage le
résout directement auprès de Share et les deux consommations utilisent un CAS,
ce qui garantit l'usage unique même sous deux requêtes concurrentes.

## Niveau de confidentialité

### Première version pragmatique

Pour rester compatible avec le chemin statique binaire actuel :

- générer au moins 128 bits aléatoires pour chaque URL-capacité ;
- ne pas indexer les pièces jointes ShellDeck dans la galerie publique ;
- marquer les fichiers avec `purpose: "issue" | "support"`, `tenant_id` et `target_id` ;
- ne permettre leur suppression qu'à travers le cycle de vie de la demande ;
- conserver la possibilité de révoquer le lien en supprimant le blob.

Toute personne possédant l'URL pourra encore lire le fichier. L'URL doit donc
être considérée comme un secret.

### Protection stricte ultérieure

La cible préférable pour les données sensibles est une route média native qui :

1. vérifie une URL signée à durée courte ou une session autorisée ;
2. vérifie l'accès au tenant et à la demande ;
3. délègue ensuite le fichier au serveur statique, par exemple avec un mécanisme
   équivalent à `X-Accel-Redirect`.

Cette étape nécessite un changement dans Bext ou dans sa configuration de
proxy, car PRISM ne doit pas retransmettre lui-même les octets binaires.

## Modèle de données proposé

```rust
pub struct IssueAttachment {
    pub id: String,
    pub share_id: String,
    pub filename: String,
    pub content_type: String,
    pub bytes: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub created_by: String,
    pub created_at: f64,
}
```

Puis :

```rust
pub struct Issue {
    #[serde(default)]
    pub attachments: Vec<IssueAttachment>,
    // ...
}

pub struct IssueComment {
    #[serde(default)]
    pub attachments: Vec<IssueAttachment>,
    // ...
}
```

Le modèle TypeScript Manage devra avoir les mêmes champs optionnels. Les
valeurs par défaut sont indispensables pour préserver les demandes déjà
stockées.

Les URLs signées temporaires ne doivent pas être persistées dans la demande.
Seuls l'identifiant Share et les métadonnées stables doivent être stockés.

## Sources d'image

Toutes les méthodes produisent le même objet local `AttachmentDraft`. Après
l'import, leur comportement est identique : miniature, validation, retrait,
progression et upload vers Share uniquement lors de l'envoi.

### 1. URL d'image

Le composeur propose une action « Depuis une URL » avec un petit champ dédié.
ShellDeck :

1. accepte uniquement `https://` et éventuellement `http://` avec un
   avertissement ;
2. télécharge l'image sur le poste de l'utilisateur ;
3. limite les redirections, la durée et le nombre d'octets téléchargés ;
4. vérifie le type réel de l'image à partir de ses octets ;
5. crée une miniature locale ;
6. ré-uploade une copie vers Inklura Share lors de l'envoi.

Inklura Share ne doit pas télécharger lui-même l'URL fournie. Un fetch côté
serveur ouvrirait un risque SSRF vers les services internes de l'infrastructure.
Le téléchargement local évite également le hotlink : la pièce jointe reste
disponible même si l'image distante change ou disparaît.

Une URL nécessitant les cookies du navigateur ne pourra pas être importée
automatiquement. Dans ce cas, l'utilisateur pourra enregistrer l'image, la
glisser-déposer ou faire une capture de zone.

### 2. Collage avec Ctrl/Cmd+V

Lorsque le presse-papiers contient une image, ShellDeck récupère directement
son format et ses octets via `ClipboardEntry::Image` et l'ajoute à la file.

Lorsque le presse-papiers contient du texte :

- une URL d'image seule peut déclencher la proposition d'import par URL ;
- tout autre texte conserve le comportement normal du champ.

Les composants `Input` et `Editor` actuels ne traitent que le texte dans leur
action `Paste`. Il faudra leur ajouter un callback générique de collage d'image
dans le patch adabraka-ui, utilisé de façon identique par les composeurs User et
Support.

### 3. Glisser-déposer

La zone du composeur accepte les `ExternalPaths` GPUI. Pour chaque chemin :

- vérifier qu'il s'agit d'un fichier régulier ;
- lire au maximum la limite autorisée ;
- vérifier le type réel ;
- ajouter une miniature ou afficher une erreur locale.

Le glisser-déposer d'une image directement depuis un navigateur n'est pas
uniforme selon les OS : il peut produire un fichier temporaire ou seulement une
URL. Les chemins de fichiers sont pris en charge directement ; une URL seule
repasse par le flux « Depuis une URL ».

### 4. Sélection d'une zone de l'écran

Le bouton « Capturer une zone » se trouve à côté du trombone. Son flux est :

1. suspendre ou masquer temporairement la fenêtre ShellDeck pour qu'elle ne
   couvre pas la zone voulue ;
2. lancer le sélecteur de zone adapté à l'OS ;
3. laisser l'utilisateur tracer son rectangle sur n'importe quel écran ;
4. récupérer le PNG produit ;
5. restaurer ShellDeck ;
6. ajouter immédiatement la capture à la file locale ;
7. ne rien envoyer tant que la demande ou le commentaire n'est pas validé.

La sélection doit être annulable avec `Échap` sans afficher de toast d'erreur.

#### macOS

Utiliser l'outil système `screencapture` en mode interactif avec une destination
temporaire créée de façon sûre. macOS gère le rectangle, les écrans multiples et
la permission « Enregistrement de l'écran ». Le fichier temporaire est lu puis
supprimé dès que le `AttachmentDraft` est créé.

#### Windows

Le nouveau protocole officiel de l'Outil Capture d'écran prend en charge le
rectangle et un callback vers l'application, mais ce callback exige une
application empaquetée MSIX. ShellDeck est actuellement distribué par NSIS et
ZIP ; il ne doit donc pas dépendre de ce callback.

La solution fiable avec le packaging actuel est :

1. activer la feature `screen-capture` du fork GPUI ;
2. masquer ShellDeck ;
3. capturer une image figée des écrans via le backend `scap` déjà présent ;
4. ouvrir une fenêtre GPUI plein écran de type overlay ;
5. laisser l'utilisateur tracer le rectangle ;
6. convertir les coordonnées logiques en pixels physiques ;
7. rogner le buffer et l'encoder en PNG ;
8. fermer l'overlay et restaurer ShellDeck.

Cela ne dépend pas du presse-papiers et permet un résultat direct même avec
l'installateur NSIS actuel. Le lancement de l'Outil Capture pourra devenir une
alternative si ShellDeck adopte plus tard un package MSIX avec protocole de
callback enregistré.

#### Linux

Utiliser en priorité le portail desktop de capture d'écran en mode interactif.
Il fournit l'interface utilisateur adaptée au bureau et fonctionne dans les
environnements Wayland où la capture directe GPUI n'est pas implémentée.

Le portail Screenshot version 3 expose explicitement la cible `Area`, ce qui
correspond exactement au besoin de sélection de rectangle. Sous X11, le même
portail reste le premier choix. Un fallback vers la capture GPUI et l'overlay
ShellDeck ne doit être utilisé que si le portail est réellement indisponible.
L'absence d'un utilitaire optionnel ne doit jamais faire échouer le démarrage de
ShellDeck.

### Interface interne proposée

```rust
pub trait RegionCaptureProvider {
    fn is_available(&self) -> bool;
    fn capture_region(&self) -> BoxFuture<'static, Result<Option<CapturedImage>>>;
}

pub struct CapturedImage {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub filename: String,
}
```

`Ok(None)` représente une annulation utilisateur. Les implémentations sont
séparées avec `#[cfg(target_os = "linux")]`, `macos` et `windows`, mais les trois
doivent être fonctionnelles avant livraison conformément aux règles
cross-platform du projet.

### Overlay de sélection ShellDeck

Le fallback GPUI, requis sous Windows avec le packaging actuel, est une vue
réutilisable et non trois implémentations de dessin différentes :

- capture figée de chaque écran ;
- une fenêtre overlay sans décoration par écran ;
- voile sombre semi-transparent ;
- curseur en croix ;
- rectangle clair pendant le drag ;
- dimensions en pixels à côté du rectangle ;
- `Échap` pour annuler, relâchement de la souris pour valider ;
- correction du facteur d'échelle HiDPI avant le crop.

Le fork GPUI expose aujourd'hui les frames sous une forme dépendante de la
plateforme. Il faudra probablement lui ajouter un helper stable qui convertit
une frame en buffer RGBA/BGRA afin que le code de crop reste commun et testable.

## Expérience utilisateur proposée

Les composeurs « Nouvelle demande » et « Ajouter un commentaire » partagent le
même composant de pièces jointes :

- bouton trombone « Ajouter des images » ;
- action « Depuis une URL » ;
- collage avec `Ctrl+V` ou `Cmd+V` lorsqu'une image est présente ;
- glisser-déposer ;
- bouton « Capturer une zone » ;
- bande de miniatures ;
- nom et taille de chaque image ;
- bouton de retrait avant envoi ;
- progression, erreur et bouton « Réessayer ».

Limites proposées pour la première version :

- PNG, JPEG et WebP uniquement ;
- 5 images maximum ;
- 9 Mo maximum par image ;
- 25 Mo maximum par demande ou commentaire ;
- 2 uploads simultanés au maximum.

SVG, PDF, vidéos et archives restent hors périmètre de cette première version.

## Affichage dans ShellDeck

Dans le détail d'une demande :

- afficher les pièces jointes sous la description ou le commentaire associé ;
- utiliser une grille compacte de miniatures ;
- ouvrir l'image en grand dans une sheet ou dans le navigateur ;
- afficher un état de remplacement si l'image a expiré ou a été supprimée ;
- mettre en cache les miniatures pour éviter un téléchargement à chaque rendu.

L'application n'installe actuellement pas de client HTTP GPUI. Il faudra soit
installer le client prévu par GPUI au démarrage, soit télécharger et mettre en
cache les octets avec le client réseau ShellDeck.

Le support multipart de `reqwest` devra également être activé dans le workspace.

## GitHub et agents Jean

### GitHub

Lorsqu'une demande est poussée vers GitHub :

- une image publique par URL-capacité peut être ajoutée au Markdown ;
- une image strictement privée doit rester un lien vers une page nécessitant
  une authentification, car GitHub ne pourra pas l'intégrer directement ;
- une synchronisation GitHub ultérieure ne doit pas effacer les métadonnées
  structurées des pièces jointes ShellDeck.

### Jean et la fleet

Ajouter seulement une URL au prompt ne garantit pas que l'agent puisse lire
l'image. Pour une véritable analyse visuelle, le runtime devrait :

1. télécharger les pièces jointes autorisées dans un répertoire temporaire du
   job ;
2. ajouter leurs chemins locaux au contexte ;
3. nettoyer ces fichiers après le job.

Cette intégration peut être livrée après l'upload et l'affichage de base.

## Suppression et rétention

Une demande est supprimée aujourd'hui par soft-delete. Ses images ne devraient
donc pas disparaître immédiatement, sinon l'historique d'audit devient
incomplet.

Politique proposée :

- image rattachée : conservée pendant la durée de rétention de la demande ;
- image uploadée mais jamais réclamée par Manage : purge opportuniste après 1 heure ;
- demande soft-deleted : purge différée selon une durée à définir ;
- suppression immédiate réservée à une action explicite et auditée ;
- quotas par utilisateur et tenant pour empêcher l'accumulation incontrôlée.

## Découpage d'implémentation

### 1. Inklura Share

- ajouter le mode d'upload `issue` ;
- ajouter les tickets et reçus opaques à usage unique ;
- passer à des identifiants d'au moins 128 bits ;
- vérifier la signature réelle des formats acceptés ;
- refuser SVG dans ce mode ;
- ajouter quotas, rate limiting et nettoyage des orphelins ;
- durcir le cookie de session avec `HttpOnly`.

### 2. Manage/Bext

- ajouter `IssueAttachment` aux demandes et commentaires ;
- émettre les tickets d'upload après contrôle du scope ;
- valider les reçus Share ;
- ajouter les actions de rattachement et de retrait ;
- maintenir la compatibilité avec les anciens enregistrements ;
- définir le comportement GitHub et la rétention.

### 3. ShellDeck Core

- ajouter les nouveaux contrats serde ;
- ajouter le client d'upload multipart ;
- gérer les limites, erreurs et retries ;
- ajouter des tests de contrat avec le `TcpListener` mock existant.

### 4. ShellDeck UI

- créer un composant partagé de pièces jointes ;
- intégrer URL, fichier, collage et glisser-déposer ;
- ajouter le fournisseur cross-platform « Capturer une zone » ;
- afficher les miniatures et la progression ;
- rendre les images dans les vues User et Support ;
- ajouter les libellés français et anglais.

### 5. GitHub et Jean

- propager les liens selon le niveau de confidentialité retenu ;
- matérialiser les images localement pour les jobs qui doivent les analyser.

## Tests indispensables

- parsing d'une ancienne demande sans `attachments` ;
- création et commentaire avec plusieurs images ;
- commentaire composé uniquement d'une image ;
- ticket expiré, falsifié ou réutilisé ;
- tentative de rattachement entre deux tenants ;
- type MIME mensonger ou format interdit ;
- limites de taille, nombre et quota ;
- échec d'un upload au milieu d'un lot puis retry ;
- purge d'un upload orphelin ;
- conservation après soft-delete ;
- collage et sélection manuelle sur Linux, macOS et Windows.

## Recommandation finale

Réutiliser Inklura Share pour le stockage est pertinent. Il ne faut cependant
pas intégrer directement l'API historiquement prévue pour ShareX ni persister
une clé `isk_...` dans ShellDeck.

La bonne frontière est : session Manage pour l'autorisation, ticket court pour
l'upload direct vers Share, reçu signé pour le rattachement à la demande, et
métadonnées structurées dans le modèle `Issue`.

## Références plateforme

- [Portail Screenshot XDG](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Screenshot.html)
  — capture interactive et cible `Area` sous Linux/Wayland.
- [Protocole Windows Snipping Tool](https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-snipping-tool)
  — rectangle et callback, avec contrainte de packaging MSIX.
- `man screencapture` sous macOS — option interactive `-i` pour sélectionner
  une zone ou une fenêtre.
