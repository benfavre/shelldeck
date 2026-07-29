# Monolith motion studies

Transparent lossless animated WebP exports from
`docs/design/monolith-animations.html`.

| Asset | Loop |
|---|---:|
| `monolith-breathe.webp` | 3.6 s |
| `monolith-blink.webp` | 5 s |
| `monolith-slow-blink.webp` | 3.2 s |
| `monolith-thinking.webp` | 6 s |
| `monolith-busy.webp` | 3.2 s |
| `monolith-scan.webp` | 1.2 s |
| `monolith-speaking.webp` | 0.9 s |
| `monolith-dots.webp` | 1.4 s |
| `monolith-progress-ring.webp` | 1.1 s |
| `monolith-progress-bar.webp` | 2.6 s |
| `monolith-terminal-typing.webp` | 1.4 s |
| `monolith-chevron-rain.webp` | 2.8 s |
| `monolith-success.webp` | 3 s |
| `monolith-alert.webp` | 4 s |
| `monolith-boot.webp` | 3.2 s |

Regenerate all studies:

```bash
node scripts/export-monolith-webp.mjs --studies
```

Regenerate one study:

```bash
node scripts/export-monolith-webp.mjs --studies --only=thinking
```
