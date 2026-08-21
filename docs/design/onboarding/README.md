# Visuels d’onboarding par rôle

La production finale se trouve dans
`crates/shelldeck/assets/images/onboarding/role-aware/` :

- `user-01-welcome.webp` à `user-04-ai.webp` ;
- `support-01-welcome.webp` à `support-05-modes.webp` ;
- `dev-01-welcome.webp` à `dev-06-modes.webp`.

Les quinze images font exactement **1120 × 400 px**. Elles sont exportées en
**WebP lossless** afin de préserver les textes et les traits des captures UI.
Le lot complet pèse environ 800 Kio. La planche de contrôle est
`role-aware-contact-sheet.webp`.

## Reproduire les exports

La composition source est `../onboarding-role-visuals.html`. Elle conserve les
fragments d’interface tels quels et construit seulement les cadres, ombres,
pastilles, fils, cartes fantômes et autres ornements en HTML/CSS.

```bash
node scripts/export-onboarding-images.mjs
```

Chrome capture d’abord chaque scène dans un PNG temporaire, puis ImageMagick
l’encode en WebP lossless (`webp:method=6`). Les PNG temporaires sont supprimés
à la fin de l’export.

## Matière générative

L’outil ImageGen intégré a servi uniquement à créer la matière aquarelle de
fond. Les captures produit n’ont pas été passées au modèle : leur texte ne peut
donc pas être réécrit ou halluciné. La matière a ensuite été teintée par rôle et
compressée en WebP qualité 92 ; à l’opacité utilisée dans les scènes, cette
source décorative ne présente pas de perte visible.

Prompt final :

```text
Use case: stylized-concept
Asset type: subtle background material for a native desktop application's onboarding banners
Primary request: Create one refined, very pale watercolor wash on warm paper, intended to sit behind crisp UI screenshots. The color story should move gently from ShellDeck blue #146bff through a small restrained coral #f1644a accent into violet #6d46e7, with most of the canvas remaining warm white #fffdf9.
Scene/backdrop: abstract watercolor pigment bleeding softly into warm off-white cotton paper; no objects, no interface, no landscape, no botanical forms.
Style/medium: authentic hand-painted watercolor, fine paper grain, editorial SaaS brand art, airy and understated.
Composition/framing: very wide panoramic 2.8:1 composition; pigment concentrated near the far left and far right edges, large quiet clean center, generous negative space; safe for cropping to 1120x400.
Lighting/mood: bright, calm, welcoming, premium.
Color palette: only #fffdf9, #146bff, #f1644a, #6d46e7 and very pale tints of those colors.
Constraints: no text, no lettering, no logo, no icons, no UI, no frame, no hard-edged geometry, no people, no faces, no watermark. Keep the pigment very desaturated and background-like, never dominant.
Avoid: rainbow gradient, neon, glossy 3D, dark background, strong contrast, flowers, leaves, scenery.
```
