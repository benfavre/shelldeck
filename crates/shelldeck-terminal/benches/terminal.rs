//! CPU hot-path benchmarks for the terminal engine.
//!
//! These run headless (no GPU/window) and exercise the same code that burns
//! CPU when a full-screen TUI like htop repaints: VTE parsing + grid updates,
//! plus the per-frame helpers the UI calls (visible_rows, detect_urls, search).
//!
//! Run with: `cargo bench -p shelldeck-terminal`

use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use parking_lot::Mutex;
use shelldeck_terminal::grid::{Cell, TerminalGrid};
use shelldeck_terminal::parser::TerminalProcessor;
use shelldeck_terminal::url::detect_urls;

const ROWS: usize = 50;
const COLS: usize = 200;

/// Build one realistic full-screen TUI repaint frame (htop-like): home the
/// cursor, draw colored meter bars, then a colored process table, erasing each
/// line. This is the byte stream a TUI sends ~once per update.
fn htop_like_frame() -> Vec<u8> {
    let mut out = String::new();
    // Home cursor + hide cursor (htop uses the alt screen + DECTCEM).
    out.push_str("\x1b[H\x1b[?25l");

    // A couple of CPU meter rows with truecolor gradient segments.
    for meter in 0..4 {
        out.push_str(&format!("\x1b[{};1H", meter + 1));
        out.push_str("\x1b[K"); // erase line
        out.push_str("\x1b[1;37m["); // bold white
        for seg in 0..100 {
            // Vary color per segment to stress SGR parsing.
            let r = (seg * 2) as u8;
            let g = (255 - seg * 2) as u8;
            out.push_str(&format!("\x1b[38;2;{};{};0m|", r, g));
        }
        out.push_str("\x1b[0m]\r\n");
    }

    // Header row (inverse).
    out.push_str("\x1b[6;1H\x1b[K\x1b[7m  PID USER      PRI  NI  VIRT   RES   SHR S CPU% MEM%   TIME+  Command\x1b[0m\r\n");

    // Process rows: each with SGR color changes + a path-like command (so URL
    // detection has something to find), erased to EOL.
    for row in 7..=ROWS {
        out.push_str(&format!("\x1b[{};1H\x1b[K", row));
        let pid = 1000 + row;
        // Colored columns.
        out.push_str(&format!(
            "\x1b[36m{:>5}\x1b[0m user      20   0 \x1b[33m{:>5}M\x1b[0m {:>5}M  1234 S \x1b[32m{:>4.1}\x1b[0m \x1b[35m{:>4.1}\x1b[0m  0:0{}.12 ",
            pid,
            (row * 7) % 900,
            (row * 3) % 500,
            ((row * 13) % 100) as f32 / 10.0,
            ((row * 17) % 100) as f32 / 10.0,
            row % 10,
        ));
        out.push_str("/usr/lib/example/service --config /etc/app/conf.d/main.toml\r\n");
    }
    out.into_bytes()
}

/// Seed a grid by feeding it `n` frames (so visible_rows/search/detect benches
/// operate on realistic, fully-populated content).
fn seeded_grid() -> Arc<Mutex<TerminalGrid>> {
    let grid = Arc::new(Mutex::new(TerminalGrid::new(ROWS, COLS)));
    let mut processor = TerminalProcessor::new(grid.clone());
    let mut parser = vte::Parser::new();
    let frame = htop_like_frame();
    for _ in 0..5 {
        processor.process_bytes(&mut parser, &frame);
    }
    grid
}

fn bench_parse_frame(c: &mut Criterion) {
    let frame = htop_like_frame();
    let mut group = c.benchmark_group("parse");
    group.throughput(Throughput::Bytes(frame.len() as u64));
    group.bench_function("htop_frame", |b| {
        // Fresh grid+parser per iteration batch; process one frame each iter.
        let grid = Arc::new(Mutex::new(TerminalGrid::new(ROWS, COLS)));
        let mut processor = TerminalProcessor::new(grid.clone());
        let mut parser = vte::Parser::new();
        b.iter(|| {
            processor.process_bytes(&mut parser, black_box(&frame));
        });
    });
    group.finish();
}

fn bench_visible_rows(c: &mut Criterion) {
    let grid = seeded_grid();
    c.bench_function("visible_rows", |b| {
        b.iter(|| {
            let g = grid.lock();
            black_box(g.visible_rows());
        });
    });
}

fn bench_detect_urls(c: &mut Criterion) {
    let grid = seeded_grid();
    c.bench_function("detect_urls", |b| {
        b.iter(|| {
            let g = grid.lock();
            let rows = g.visible_rows();
            black_box(detect_urls(&rows));
        });
    });
}

fn bench_search(c: &mut Criterion) {
    let grid = seeded_grid();
    c.bench_function("search_plain", |b| {
        b.iter(|| {
            let g = grid.lock();
            black_box(g.search(black_box("service"), false, false));
        });
    });
}

fn bench_scroll(c: &mut Criterion) {
    c.bench_function("scroll_up_1", |b| {
        let grid = seeded_grid();
        b.iter(|| {
            grid.lock().scroll_up(black_box(1));
            grid.lock().scroll_down(black_box(1));
        });
    });
}

/// A small, frequently-allocated structure used per frame; sanity check its cost.
fn bench_cell_default(c: &mut Criterion) {
    c.bench_function("cell_row_alloc", |b| {
        b.iter(|| {
            let row: Vec<Cell> = (0..COLS).map(|_| Cell::default()).collect();
            black_box(row);
        });
    });
}

criterion_group!(
    benches,
    bench_parse_frame,
    bench_visible_rows,
    bench_detect_urls,
    bench_search,
    bench_scroll,
    bench_cell_default,
);
criterion_main!(benches);
