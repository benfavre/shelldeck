#!/usr/bin/env node

import { spawn } from "node:child_process";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const FPS = 30;
const SIZE = 256;
const CONCURRENCY = 10;
const MODE_EXPORTS = [
  { slug: "user", durationMs: 2_500, group: "modes" },
  { slug: "support", durationMs: 2_500, group: "modes" },
  { slug: "dev", durationMs: 2_500, group: "modes" },
];
const STUDY_EXPORTS = [
  { slug: "breathe", durationMs: 3_600, group: "studies" },
  { slug: "blink", durationMs: 5_000, group: "studies" },
  { slug: "slow-blink", durationMs: 3_200, group: "studies" },
  { slug: "thinking", durationMs: 6_000, group: "studies" },
  { slug: "busy", durationMs: 3_200, group: "studies" },
  { slug: "scan", durationMs: 1_200, group: "studies" },
  { slug: "speaking", durationMs: 900, group: "studies" },
  { slug: "dots", durationMs: 1_400, group: "studies" },
  { slug: "progress-ring", durationMs: 1_100, group: "studies" },
  { slug: "progress-bar", durationMs: 2_600, group: "studies" },
  { slug: "terminal-typing", durationMs: 1_400, group: "studies" },
  { slug: "chevron-rain", durationMs: 2_800, group: "studies" },
  { slug: "success", durationMs: 3_000, group: "studies" },
  { slug: "alert", durationMs: 4_000, group: "studies" },
  { slug: "boot", durationMs: 3_200, group: "studies" },
];

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const source = path.join(repoRoot, "docs/design/monolith-animations.html");
const assetsDir = path.join(repoRoot, "crates/shelldeck/assets/images/brand/webp");

function run(command, args, stdio = "ignore") {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio });
    child.once("error", reject);
    child.once("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${command} exited with code ${code}`));
    });
  });
}

async function parallel(items, limit, worker) {
  let cursor = 0;
  async function consume() {
    while (cursor < items.length) {
      const index = cursor;
      cursor += 1;
      await worker(items[index], index);
    }
  }
  await Promise.all(Array.from({ length: limit }, consume));
}

const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "shelldeck-monolith-"));
let exportsToRun = process.argv.includes("--studies")
  ? STUDY_EXPORTS
  : process.argv.includes("--modes")
    ? MODE_EXPORTS
    : [...MODE_EXPORTS, ...STUDY_EXPORTS];
const fromArg = process.argv.find((arg) => arg.startsWith("--from="));
if (fromArg) {
  const fromSlug = fromArg.slice("--from=".length);
  const start = exportsToRun.findIndex((item) => item.slug === fromSlug);
  if (start < 0) throw new Error(`Unknown export slug: ${fromSlug}`);
  exportsToRun = exportsToRun.slice(start);
}
const onlyArg = process.argv.find((arg) => arg.startsWith("--only="));
if (onlyArg) {
  const onlySlug = onlyArg.slice("--only=".length);
  const item = exportsToRun.find((candidate) => candidate.slug === onlySlug);
  if (!item) throw new Error(`Unknown export slug: ${onlySlug}`);
  exportsToRun = [item];
}

try {
  for (const item of exportsToRun) {
    const frameCount = Math.round((FPS * item.durationMs) / 1_000);
    const framesDir = path.join(tempRoot, item.slug);
    const outputDir = path.join(assetsDir, item.group);
    await fs.mkdir(framesDir);
    await fs.mkdir(outputDir, { recursive: true });
    process.stdout.write(
      `capturing ${item.slug} (${frameCount} frames, ${item.durationMs}ms)\n`,
    );

    await parallel(
      Array.from({ length: frameCount }, (_, frame) => frame),
      CONCURRENCY,
      async (frame) => {
        const elapsed = (frame * 1_000) / FPS;
        const url = new URL(pathToFileURL(source));
        url.searchParams.set("export", item.slug);
        url.searchParams.set("time", String(elapsed));
        const destination = path.join(
          framesDir,
          `frame-${String(frame).padStart(3, "0")}.png`,
        );

        await run(process.env.CHROME_BIN || "google-chrome", [
          "--headless=new",
          "--no-sandbox",
          "--disable-gpu",
          "--hide-scrollbars",
          "--allow-file-access-from-files",
          "--run-all-compositor-stages-before-draw",
          "--default-background-color=00000000",
          `--window-size=${SIZE},${SIZE}`,
          `--screenshot=${destination}`,
          url.href,
        ]);
      },
    );

    await run(
      "ffmpeg",
      [
        "-hide_banner",
        "-loglevel",
        "warning",
        "-y",
        "-framerate",
        String(FPS),
        "-i",
        path.join(framesDir, "frame-%03d.png"),
        "-loop",
        "0",
        "-c:v",
        "libwebp_anim",
        "-lossless",
        "1",
        "-compression_level",
        "6",
        path.join(outputDir, `monolith-${item.slug}.webp`),
      ],
      "inherit",
    );
    process.stdout.write(`exported monolith-${item.slug}.webp\n`);
  }
} finally {
  await fs.rm(tempRoot, { recursive: true, force: true });
}
