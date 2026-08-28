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
