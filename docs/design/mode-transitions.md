# Mode transitions

Switching between Client (User), Support, and Dev uses a brief neutral crossfade:

1. A veil fades over the current surface for 60 ms.
2. The authorized destination is committed and its background loading starts.
3. The veil clears over 100 ms. There is no first-visit or repeat loading hold.

Reduced motion commits immediately. The transition no longer decodes or animates
a full-window mascot. Network loading belongs inside each destination; retained
views and active terminal/agent sessions survive the switch. Repeated requests
during the 160 ms transition are ignored. Logout cancels it, and permissions are
checked again at the commit boundary.

The previous 3,000 ms first-entry and 880 ms repeat sequences were artificial
navigation waits. These values describe scheduled UI timing, not a guarantee
that remote Support or Client data has arrived within 160 ms.
