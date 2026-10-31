# Agents frame performance

The target is enough CPU headroom for 120 Hz (8.33 ms per frame), with native
input dispatch reaching GPU submission promptly. Delivered presentation FPS is
limited by the display and compositor: the current Linux/Xwayland display is
59.96 Hz. A CPU budget result is not evidence of 120 displayed FPS, and
input-to-submit excludes compositor queueing and scanout.

## Use the production fixture

Do not measure native FPS with `cargo run -p shelldeck-ui --example
agent_console_preview`. That package's dev dependency enables GPUI
`test-support`, whose eager effect flushing redraws outside the production frame
scheduler. The older preview also resolved the operating-system font catalogue
on every root render. The previously recorded approximately 20-FPS native
figures therefore do not establish production ShellDeck FPS.

The dedicated binary uses the application's dependency graph, resolves its UI
font once like Workspace, and rejects profiling when GPUI test-support is active:

```sh
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig \
  cargo build --release --locked -p shelldeck --features agent-preview \
  --bin agent-console-preview
python3 scripts/test-agent-frame-benchmark.py
```

`agent-preview` is opt-in; normal application builds and `cargo run` retain the
`shelldeck` entry point. CI checks the fixture on Linux, macOS and Windows and
pins its feature graph separately from the UI test harness.

Create all three XDG homes under a fresh absolute temporary directory. Do not
change `HOME` or use the desktop profile. Launch the binary with
`SHELLDECK_PREVIEW_PERF=1 SHELLDECK_PREVIEW_PROFILE=1 ZED_MEASUREMENTS=1`, then
scroll inside the conversation. The fixture has no provider executor; optional
streaming uses synthetic events only after Send. Existing native walkthroughs
also mask the real home and disable networking.

Fixture controls:

- `SHELLDECK_PREVIEW_PERF=1`: 80 retained exchanges.
- `SHELLDECK_PREVIEW_BODY_PARAGRAPHS=512`: long individual message stress case;
  default 12, bounded to 1–512.
- `SHELLDECK_PREVIEW_STREAM=1`: Send admits a fake response with 180 updates at
  16 ms intervals, then completes. No real provider is started.
- `SHELLDECK_PREVIEW_PROFILE=1`: warm up for two seconds, then count 179 native
  frame intervals while requesting redraws. Omit for ordinary visual QA.
- `ZED_MEASUREMENTS=1`: opt-in CPU draw, GPU presentation, arena cleanup and
  first-pending-input-to-submit durations. No message or credential data is logged.
- Existing `SHELLDECK_PREVIEW_LIGHT`, `FR`, `WIDTH` and `SECONDS` flags select
  appearance, language, initial width and bounded fixture lifetime.

Summarize a captured stderr log:

```sh
python3 scripts/summarize-agent-frames.py /absolute/path/to/preview.log
```

Compare identical workloads and build flags, with compilers stopped before
sampling. Keep short, long, large-body and active-stream cases distinct. Report
p50/p95/max, sample counts, cold startup separately, and native cadence separately
from CPU throughput. The current optimized comparison disables release LTO for
build time; optimization level remains 3 in both variants.

## Retained row contract

Every nonempty conversation uses the measured list. Only viewport/overdraw rows are rendered. Up to 64 retained row views reuse
measured layout and paint when unchanged. The cache stores descriptors and weak
parent handles, not copies of retained message bodies. Live/pending operation
rows and changing footer/history controls remain uncached. Content, expansion,
source identity, typography, locale and width changes trigger fresh layout.
Canonical copy/reveal and draft behavior remain authoritative.

SDTEST-2875 checks actual cached hitboxes and copied content, streaming
replacement, resize/font/theme changes and source/draft switching. Existing large
timeline, paging, disclosure, focus, motion and pruning scenarios remain required.

The unmaximized Workspace resize cursor no longer forces `Window::refresh` for
ordinary pointer motion. It notifies the Workspace only when the resize edge
changes, allowing retained transcript paint to survive. Windows' DWM timing
fallback now converts full rational periods instead of truncating the frequency
to ticks per microsecond (which could produce a zero divisor). SDTEST-2878 covers
60/120/144/240 Hz, fractional rates, invalid values and QPC conversion; Windows
CI compiles the native call site. Native high-refresh presentation on Windows
still requires a suitable display.

## Prose and repaint scopes

Long agent replies (at least 4 KiB and over 16 top-level Markdown blocks) render
as ranges of eight complete blocks. Parsing the full document retains reference
links, fenced code, lists and tables. Range rendering keeps global block spacing.
Source text remains authoritative for copy/quote; encoded payloads keep their
original fold/reveal path. Lightweight block counts persist with the timeline,
while at most four parsed documents remain cached. Single complex blocks and
long native-history entries remain atomic; they need separate stress coverage.

The navigator uses a fixed-size retained view. A separate timeline view owns
wheel invalidation; follow-state changes still update the parent. Existing
selected reply TextDelta/TextSnapshot batches can invalidate the timeline only
when search is empty and status/attention/unread state is unchanged. First output,
background updates, active search and non-text events notify the parent, keeping
navigation and controls current. Other parent notifications invalidate the
navigator through its ordinary GPUI observer.

SDTEST-2880–2882 check long-message layout counts, exact full-source copy,
same-length snapshot anchors, complete-document references and structures,
bounded AST retention, actual wheel/cache hitboxes and live transcript search.
