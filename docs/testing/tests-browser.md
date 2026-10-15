# Browser runtime and computer-use contracts

All entries map to SDUC-557. Offline tests use fake child pipes and private loopback
fixtures; they never start a provider, use a real browser profile or call a remote
website. Runtime smoke tests use the packaged CEF helper and a local HTTP page.

| ID | Location | Status | Contract |
|---|---|---|---|
| SDTEST-2530 | `shelldeck-browser/src/protocol.rs` | Green | Binary framing, bounded headers/payloads, truncated frames. |
| SDTEST-2531 | `protocol.rs` | Green | Navigation refuses privileged schemes and embedded credentials. |
| SDTEST-2532 | `mcp.rs` | Green | Read-only grants refuse typed browser mutations. |
| SDTEST-2533 | `mcp.rs` | Green | MCP request, capability and message bounds. |
| SDTEST-2534 | `client.rs` | Green | Latest-frame cache, foreign tab frames and bounded queues. |
| SDTEST-2535 | `mcp.rs` | Green | Tab ownership, private capability configuration and revocation. |
| SDTEST-2536 | `client.rs` | Green | CDP replies match both request and tab; timeouts release pending slots. |
| SDTEST-2537 | `mcp.rs` | Green | Screenshot contains actual OSR pixels plus popup and maps HiDPI coordinates. |
| SDTEST-2538 | `mcp.rs` | Green | Explicit grants admit new same-helper tabs while excluding prior ungranted/foreign tabs. |
| SDTEST-2550 | `runtime/output.rs` | Green | Backpressure coalesces pending paint and preserves control events. |
| SDTEST-2551 | `runtime/mod.rs` | Green | HiDPI and rectangle/frame bounds reject overflow. |
| SDTEST-2552 | `scripts/test-browser-runtime.py` | Green (Linux live) | Native input/Unicode/select, page dialog, upload/download bytes, related popup/opener messages, exact HiDPI frames, cookie/storage retention after helper restart. |
| SDTEST-2553 | CI browser runtime matrix | Green | Native CEF helper builds, staged launcher probes and actual MCP workflows passed on Linux/macOS/Windows. |
| SDTEST-2595 | `scripts/test-browser-packaging.py` | Green | Linux archive retains current-version CEF resources, executable mode and links, excluding old versions. |
| SDTEST-2596 | `scripts/test-browser-packaging.py` | Green | Windows ZIP retains nested bootstrap, client DLL and runtime resources, excluding old versions. |
| SDTEST-2597 | `scripts/test-browser-packaging.py` | Green | Incomplete engine resources refuse archives on all three platforms. |
| SDTEST-2598 | `scripts/test-browser-helper-launch.py` | Green (Linux/macOS/Windows) | Packaged native launcher reaches Rust MCP entry point instead of failing in the OS loader. |
| SDTEST-2539 | `client.rs` | Green | Native popup adoption verifies owned parent, helper ID domain and tab cap. |
| SDTEST-2540 | `client.rs` | Green | Background reaper allows graceful profile flush, then terminates a stuck helper within a deadline. |
| SDTEST-2541 | `mcp.rs` | Green | High popup IDs cross MCP as lossless decimal strings; lossy numeric IDs refuse. |
| SDTEST-2542 | `mcp.rs` | Green | New-tab inheritance excludes descendants of private/ungranted parents. |
| SDTEST-2554 | `runtime/output.rs` | Green | Page metadata coalesces and remains bounded at UTF-8 boundaries. |
| SDTEST-2555 | `examples/computer_use_smoke.rs` | Green (Linux/macOS/Windows live) | Actual helper + client + MCP stdio: native PNG/AX, read-only/control, Unicode input, Enter form submit, exactly-once human raw-key/IME typing, popup control, foreign-tab refusal and revocation. |
| SDTEST-2556 | `runtime/keys.rs` | Green | Portable logical keys become native Cocoa key locations and characters while preserving CHAR events and explicit Unicode input. |
