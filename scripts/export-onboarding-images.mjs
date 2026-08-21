#!/usr/bin/env node

import { spawn } from "node:child_process";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const slugs = [
  "user-01-welcome",
  "user-02-request",
  "user-03-follow",
  "user-04-ai",
  "support-01-welcome",
  "support-02-prioritize",
  "support-03-context",
  "support-04-ai",
  "support-05-modes",
  "dev-01-welcome",
  "dev-02-terminal",
  "dev-03-scripts",
  "dev-04-tunnels",
  "dev-05-ai",
  "dev-06-modes",
];
const onlyArgs = process.argv
  .filter((arg) => arg.startsWith("--only="))
  .flatMap((arg) => arg.slice("--only=".length).split(","))
  .filter(Boolean);
const exportsToRun = onlyArgs.length > 0 ? onlyArgs : slugs;
for (const slug of exportsToRun) {
  if (!slugs.includes(slug)) throw new Error(`Unknown export slug: ${slug}`);
}

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const source = path.join(repoRoot, "docs/design/onboarding-role-visuals.html");
const outputDir = path.join(
  repoRoot,
  "crates/shelldeck/assets/images/onboarding/role-aware",
);

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: "inherit" });
    child.once("error", reject);
    child.once("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${command} exited with code ${code}`));
    });
  });
}

await fs.mkdir(outputDir, { recursive: true });
const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), "shelldeck-onboarding-"));
try {
  for (const slug of exportsToRun) {
    const url = new URL(pathToFileURL(source));
    url.searchParams.set("export", slug);
    const screenshot = path.join(tempDir, `${slug}.png`);
    const destination = path.join(outputDir, `${slug}.webp`);
    await run(process.env.CHROME_BIN || "google-chrome", [
      "--headless=new",
      "--no-sandbox",
      "--disable-gpu",
      "--hide-scrollbars",
      "--allow-file-access-from-files",
      "--run-all-compositor-stages-before-draw",
      "--force-device-scale-factor=1",
      "--window-size=1120,400",
      `--screenshot=${screenshot}`,
      url.href,
    ]);
    await run(process.env.IMAGEMAGICK_BIN || "convert", [
      screenshot,
      "-strip",
      "-define",
      "webp:lossless=true",
      "-define",
      "webp:method=6",
      destination,
    ]);
    process.stdout.write(`exported ${slug}.webp (lossless)\n`);
  }
} finally {
  await fs.rm(tempDir, { recursive: true, force: true });
}
