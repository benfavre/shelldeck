# Permissions macOS du Companion

ShellDeck ne demande aucune permission macOS supplémentaire pour afficher la
fenêtre principale, le Dock IA, le menu de barre système ou pour suivre un deep
link `shelldeck://`.

## Raccourcis globaux

Le backend macOS actuel de GPUI installe un moniteur global
`NSEvent.addGlobalMonitorForEvents(matching: .keyDown)` et un moniteur local
équivalent. D'après la
[documentation Apple de `NSEvent`](https://developer.apple.com/documentation/appkit/nsevent/addglobalmonitorforevents%28matching%3Ahandler%3A%29),
les événements clavier globaux ne sont transmis que lorsque l'accessibilité est
activée ou que l'application est approuvée pour l'accès d'assistance.

Conséquences observables :

- sans approbation, la combinaison peut encore fonctionner lorsque ShellDeck
  est l'application active grâce au moniteur local ;
- depuis une autre application, le Dock et la palette ne reçoivent pas la
  frappe tant que ShellDeck n'est pas autorisé dans les réglages de
  confidentialité macOS ;
- macOS peut présenter ce consentement sous **Accessibilité** ou
  **Surveillance de l'entrée** selon sa version et sa politique TCC ;
- ShellDeck ne capture pas le contenu tapé : le callback compare uniquement la
  touche et les modificateurs aux deux combinaisons configurées, puis laisse
  l'événement continuer vers son application cible.

Cette implémentation ne requiert pas les permissions **Enregistrement de
l'écran**, **Microphone**, **Caméra**, **Fichiers et dossiers** ou
**Automatisation**. Elle n'ajoute pas non plus d'entitlement privilégié ni de
clé de description d'usage dans `Info.plist`.

## Diagnostic

Si Settings affiche le raccourci comme actif mais que rien ne se passe depuis
une autre application :

1. ouvrir les réglages de confidentialité et sécurité de macOS ;
2. autoriser ShellDeck dans **Accessibilité** et, si macOS la présente,
   **Surveillance de l'entrée** ;
3. relancer ShellDeck si macOS ne réactive pas immédiatement le moniteur ;
4. vérifier qu'aucune autre application ne consomme déjà la combinaison.

Le menu de barre système reste le fallback sans permission.
