# Signature des versions ShellDeck

## Identité éditrice

L'identité légale de référence est :

- **FAVRE BENJAMIN**, entrepreneur individuel ;
- nom commercial **WEB-DESIGN 29** / marque **Webdesign29** ;
- SIREN **531 889 962** ;
- site officiel : <https://webdesign29.net/>.

Le nom affiché par Windows SmartScreen ou macOS Gatekeeper vient du certificat
de signature. Il doit donc rester exactement celui validé par l'autorité de
certification ou Apple ; le workflow ne remplace jamais cette identité par une
chaîne déclarative.

## Politique

Une version taguée est bloquée si une seule des garanties suivantes manque :

- archive auto-update signée par un manifeste Ed25519 vérifié dans le client ;
- exécutable et installateur Windows signés Authenticode avec horodatage SHA-256 ;
- bundle et DMG macOS signés avec un certificat Developer ID Application,
  Hardened Runtime activé, puis soumis à `notarytool` ;
- AppImage Linux signée dans ses sections de signature avec GPG ;
- `SHA256SUMS.txt` accompagné d'une signature détachée
  `SHA256SUMS.txt.asc` et de la clé publique `ShellDeck-signing-key.asc`.

Les exécutions manuelles sans tag restent utilisables comme builds de
diagnostic non signés. Elles ne peuvent ni créer une GitHub Release, ni publier
un manifeste d'auto-update.

En l'absence de certificat Authenticode public, les releases Windows utilisent
le PFX auto-signé configuré dans
`WINDOWS_TEST_CERTIFICATE_PFX_BASE64` et
`WINDOWS_TEST_CERTIFICATE_PASSWORD`. Un build manuel doit activer
`test_windows_signing` pour tester cette même mécanique. Dès que les deux
secrets `WINDOWS_CERTIFICATE_*` sont présents, le workflow leur donne la
priorité. Une configuration de production partielle bloque la release.
Le certificat auto-signé valide la chaîne technique mais reste non approuvé
par défaut sur les PC des utilisateurs.

## Secrets et variable GitHub

Configurer ces valeurs dans les secrets Actions du dépôt :

| Nom | Contenu |
|---|---|
| `WINDOWS_CERTIFICATE_PFX_BASE64` | certificat Authenticode PFX encodé en base64 |
| `WINDOWS_CERTIFICATE_PASSWORD` | mot de passe du PFX |
| `APPLE_DEVELOPER_ID_P12_BASE64` | certificat Developer ID Application P12 encodé en base64 |
| `APPLE_DEVELOPER_ID_P12_PASSWORD` | mot de passe du P12 |
| `APPLE_SIGNING_IDENTITY` | identité complète retournée par `security find-identity -v -p codesigning` |
| `APPLE_NOTARY_KEY_P8_BASE64` | clé API App Store Connect P8 encodée en base64 |
| `APPLE_NOTARY_KEY_ID` | identifiant de la clé API |
| `APPLE_NOTARY_ISSUER_ID` | issuer App Store Connect |
| `LINUX_GPG_PRIVATE_KEY_BASE64` | sous-clé GPG de signature exportée en base64, sans passphrase interactive |
| `SHELLDECK_UPDATE_PRIVATE_KEY_PEM_BASE64` | clé privée Ed25519 PEM encodée en base64 |

Configurer également la variable Actions publique
`SHELLDECK_UPDATE_PUBLIC_KEY_BASE64`. Elle contient les 32 octets bruts de la
clé publique Ed25519 en base64 et est intégrée aux binaires pendant la
compilation.

Le workflow vérifie que la clé privée du secret correspond à cette clé
publique avant de publier le manifeste.

## Création de la clé d'auto-update

Cette opération doit être réalisée une seule fois sur une machine de
confiance. La clé privée doit être sauvegardée hors de GitHub dans le coffre de
l'éditeur ; sa perte empêcherait de signer les mises à jour pour les clients
qui connaissent déjà la clé publique.

```bash
mkdir -m 700 shelldeck-update-key
openssl genpkey -algorithm ED25519 \
  -out shelldeck-update-key/private.pem
openssl pkey -in shelldeck-update-key/private.pem \
  -pubout -outform DER \
  -out shelldeck-update-key/public.der

base64 -w0 shelldeck-update-key/private.pem |
  gh secret set SHELLDECK_UPDATE_PRIVATE_KEY_PEM_BASE64

tail -c 32 shelldeck-update-key/public.der |
  base64 -w0 |
  gh variable set SHELLDECK_UPDATE_PUBLIC_KEY_BASE64
```

Sur macOS, remplacer `base64 -w0` par `base64 | tr -d '\r\n'`.

## Contrôles après publication

Windows :

```powershell
signtool verify /pa /all /v ShellDeck-windows-x86_64-setup.exe
```

macOS :

```bash
codesign --verify --deep --strict --verbose=2 ShellDeck.app
spctl --assess --type execute --verbose=4 ShellDeck.app
xcrun stapler validate ShellDeck-macos-aarch64.dmg
```

Linux :

```bash
./ShellDeck-x86_64.AppImage --appimage-signature
gpg --verify SHA256SUMS.txt.asc SHA256SUMS.txt
sha256sum --check SHA256SUMS.txt
```
