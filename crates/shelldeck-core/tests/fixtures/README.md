# Platform v2 review conformance fixture

`platform-v2-review-v2.json` is copied byte-for-byte from
`rust/crates/automonique-protocol/fixtures/platform-v2-review-v2.json` at
Automonique commit `facee850931aabf23c9d57a17aff7097dc3a5b36` (merged PR #196).

- Upstream: <https://github.com/bext-stack/automonique/blob/facee850931aabf23c9d57a17aff7097dc3a5b36/rust/crates/automonique-protocol/fixtures/platform-v2-review-v2.json>
- SHA-256: `dcbc5e2477a2d999c94e29ef8189d0a431146e2026f47ec86b12ba95049b733b`
- License: Elastic License 2.0 (`SPDX-License-Identifier: Elastic-2.0`), matching the upstream protocol source and repository license.

Keep this file exact. Scenario variants belong in test memory so the shared
fixture remains a reliable cross-client conformance input.

## Render corpus

`platform-v2-render-conformance-v1.json` is copied byte-for-byte from
`rust/crates/automonique-protocol/fixtures/platform-v2-render-conformance-v1.json`
at Automonique commit `1dd5c13f7a53aac9e28d3cd004bf73789ca66bb7`.

- Upstream: <https://github.com/bext-stack/automonique/blob/1dd5c13f7a53aac9e28d3cd004bf73789ca66bb7/rust/crates/automonique-protocol/fixtures/platform-v2-render-conformance-v1.json>
- SHA-256: `5971163444d0ed7527eee15601b1c6bded07d1e03c34c6702c9047a13675e5f8`
- License: Elastic License 2.0 (`SPDX-License-Identifier: Elastic-2.0`), matching the upstream protocol fixture and repository license.

Keep this corpus exact as well. It intentionally carries revisions above the
JavaScript safe-integer ceiling so every client proves lossless source
revision custody while rendering the same semantic keys.

## Attention succession corpus

`platform-v2-attention-conformance-v1.json` is copied byte-for-byte from
`rust/crates/automonique-protocol/fixtures/platform-v2-attention-conformance-v1.json`
at Automonique commit `4a4cab534b35dc29a37e2c6424d23396049b1003` (merged PR #214).

- Upstream: <https://github.com/bext-stack/automonique/blob/4a4cab534b35dc29a37e2c6424d23396049b1003/rust/crates/automonique-protocol/fixtures/platform-v2-attention-conformance-v1.json>
- SHA-256: `67cdb58ed7cbf6cd0a2f471c722b1dc1616d7200736c44ed702bd8b07ef21f8f`
- License: Elastic License 2.0 (`SPDX-License-Identifier: Elastic-2.0`), matching the upstream protocol fixture and repository license.

`automonique.platform/attention/v1` is `atomic_replace`, so no single snapshot
says what a client must conclude after a sequence of reads. This corpus fixes
that sequence, including the cases where the honest answer is that the source
is hidden rather than empty: a retention gap, a refusal, and an inventoried
source nobody has read yet. Keep it exact — a scenario variant belongs in test
memory so the shared file stays a reliable cross-client input.

