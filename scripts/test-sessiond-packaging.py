#!/usr/bin/env python3
"""Session daemon packaging contract tests.

Offline: a fixture payload goes through the real scripts/package-release.sh
and the install script rendered by the real update worker source. Nothing is
built, downloaded, published or launched, and every path the installer
writes is under a temporary HOME.
"""
from pathlib import Path
import hashlib
import json
import os
import platform
import shutil
import subprocess
import tarfile
import tempfile
import unittest

REPOSITORY = Path(__file__).resolve().parent.parent
VERSION = "9.8.7"
RESOURCES = (
    "icudtl.dat", "resources.pak", "chrome_100_percent.pak", "chrome_200_percent.pak",
    "v8_context_snapshot.bin", "locales/en-US.pak", "runtime.json",
)
DAEMON = "shelldeck-sessiond"
REMOTE_X86 = "shelldeck-sessiond-x86_64-unknown-linux-musl"
REMOTE_ARM = "shelldeck-sessiond-aarch64-unknown-linux-musl"
ELF = b"\x7fELF\x02\x01\x01"
# What the generated installer may run. Desktop integration tools are left
# out on purpose, so the test never talks to the session of whoever runs it.
INSTALLER_TOOLS = (
    "bash", "sh", "uname", "mktemp", "rm", "curl", "sha256sum", "cut", "mkdir", "tar",
    "cp", "chmod", "mv", "cat", "grep", "basename", "dirname", "head", "printf", "ln",
    "gzip", "ldd",
)


class SessiondPackaging(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="shelldeck sessiond packaging ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "scripts").mkdir()
        shutil.copy2(REPOSITORY / "scripts/package-release.sh", self.root / "scripts/package-release.sh")
        (self.root / "Cargo.toml").write_text(
            f'[workspace.package]\nversion = "{VERSION}"\n[workspace.dependencies]\n')
        self.release = self.root / "target/release"
        self.browser = self.release / "browser" / VERSION
        self.browser.mkdir(parents=True)
        for resource in RESOURCES:
            path = self.browser / resource
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(f"browser fixture {resource}".encode())
        self.put("shelldeck", b"application fixture")
        self.put(f"browser/{VERSION}/shelldeck-browser", b"helper fixture")
        self.put(f"browser/{VERSION}/libcef.so", b"cef fixture")

    def put(self, path, content):
        result = self.release / path
        result.write_bytes(content)
        result.chmod(0o755)
        return result

    def package(self):
        environment = dict(os.environ, SHELLDECK_RELEASE_PLATFORM="linux-x86_64")
        return subprocess.run(["bash", "scripts/package-release.sh"], cwd=self.root,
                              env=environment, text=True, capture_output=True)

    @property
    def archive(self):
        return self.root / "dist/shelldeck-linux-x86_64.tar.gz"

    # SDTEST-3034 (SDUC-574): the Linux archive carries the session daemon
    # beside the application binary, and the static builds for SSH servers
    # when they were built. An archive without the daemon is not produced.
    def test_sdtest_3034_linux_archive_carries_the_session_daemon(self):
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Missing session daemon", result.stderr)
        self.assertFalse(self.archive.exists())

        self.put(DAEMON, ELF + b" daemon fixture")
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        with tarfile.open(self.archive) as archive:
            names = archive.getnames()
            self.assertIn("shelldeck", names)
            self.assertIn(DAEMON, names)
            # Beside the binary, not in a subdirectory.
            self.assertTrue(archive.getmember(DAEMON).isfile())
            self.assertTrue(archive.getmember(DAEMON).mode & 0o100)
            self.assertEqual(archive.extractfile(DAEMON).read(), ELF + b" daemon fixture")
            self.assertFalse(any("musl" in name for name in names))

        # The static builds ship when present; nothing else with that prefix.
        self.put(REMOTE_X86, ELF + b" x86 fixture")
        self.put(REMOTE_ARM, ELF + b" arm fixture")
        self.put("shelldeck-sessiond-riscv64-unknown-linux-musl", ELF + b" not shipped")
        self.put("shelldeck-sessiond.d", b"build metadata, not shipped")
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        with tarfile.open(self.archive) as archive:
            shipped = sorted(name for name in archive.getnames() if name.startswith(DAEMON))
            self.assertEqual(shipped, [DAEMON, REMOTE_ARM, REMOTE_X86])
            self.assertEqual(archive.extractfile(REMOTE_ARM).read(), ELF + b" arm fixture")

        # A file with the right name and the wrong content, or a symlink,
        # stops the packaging.
        self.put(REMOTE_ARM, b"#!/bin/sh\n")
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("is not an ELF file", result.stderr)
        (self.release / REMOTE_ARM).unlink()
        (self.release / REMOTE_ARM).symlink_to(REMOTE_X86)
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("is not an ELF file", result.stderr)

    def render_installer(self, node, archive):
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        manifest = {"version": VERSION, "pub_date": "2026-01-01T00:00:00Z", "platforms": {
            "linux-x86_64": {"url": archive.as_uri(), "sha256": digest,
                             "size": archive.stat().st_size, "signature": ""}}}
        module = (REPOSITORY / "cloudflare/update-worker/src/index.ts").as_uri()
        javascript = f'''import worker from {json.dumps(module)};
const env = {{SHELLDECK_KV: {{get: async () => {json.dumps(json.dumps(manifest))}}}}};
const response = await worker.fetch(new Request("https://example.invalid/install.sh"), env);
process.stdout.write(await response.text());'''
        generated = subprocess.run(
            [node, "--experimental-strip-types", "--input-type=module", "-e", javascript],
            text=True, capture_output=True, timeout=20)
        self.assertEqual(generated.returncode, 0, generated.stderr)
        return generated.stdout

    def run_installer(self, script, home, tools):
        environment = {"HOME": str(home), "PATH": str(tools), "SHELL": "/bin/sh",
                       "XDG_CONFIG_HOME": str(home / ".config"),
                       "XDG_DATA_HOME": str(home / ".local/share"), "LC_ALL": "C"}
        return subprocess.run([str(tools / "bash")], input=script, env=environment, cwd=home,
                              text=True, capture_output=True, timeout=60)

    # SDTEST-3035 (SDUC-574): the generated Linux installer puts the daemon
    # and the static builds beside the installed application, replaces an
    # installed daemon by rename (a running one keeps its file), and still
    # installs a release that predates the daemon.
    def test_sdtest_3035_installer_places_the_session_daemon_beside_the_application(self):
        node = shutil.which("node")
        if platform.system() != "Linux" or platform.machine() not in ("x86_64", "amd64"):
            self.skipTest("the generated installer picks the Linux x86_64 archive")
        if not node or not shutil.which("curl"):
            self.skipTest("requires Node (TypeScript stripping) and curl; no network is used")
        version = subprocess.run([node, "-p", "process.versions.node"], text=True,
                                 capture_output=True, timeout=5)
        self.assertEqual(version.returncode, 0, version.stderr)
        if tuple(int(part) for part in version.stdout.strip().split(".")[:2]) < (22, 6):
            self.skipTest("requires Node 22.6+ TypeScript stripping")

        tools = self.root / "tools"
        tools.mkdir()
        for tool in INSTALLER_TOOLS:
            found = shutil.which(tool)
            if found:
                (tools / tool).symlink_to(found)
        home = self.root / "home"
        home.mkdir()
        (home / ".profile").write_text("")
        install = home / ".shelldeck/bin"

        self.put(DAEMON, ELF + b" daemon one")
        self.put(REMOTE_X86, ELF + b" x86 one")
        self.assertEqual(self.package().returncode, 0)
        first = self.root / "first.tar.gz"
        shutil.copy2(self.archive, first)
        result = self.run_installer(self.render_installer(node, first), home, tools)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((install / "shelldeck").read_bytes(), b"application fixture")
        self.assertEqual((install / DAEMON).read_bytes(), ELF + b" daemon one")
        self.assertEqual((install / REMOTE_X86).read_bytes(), ELF + b" x86 one")
        self.assertFalse((install / REMOTE_ARM).exists())
        for name in (DAEMON, REMOTE_X86):
            self.assertEqual((install / name).stat().st_mode & 0o777, 0o755)
        # The daemon is where the application looks for it: beside its binary.
        self.assertEqual((install / DAEMON).parent, (install / "shelldeck").parent)

        # A second release over the first, while "a daemon runs" (its file
        # is held open): the name gets a new file, the open one is intact.
        self.put(DAEMON, ELF + b" daemon two")
        self.put(REMOTE_ARM, ELF + b" arm two")
        self.assertEqual(self.package().returncode, 0)
        second = self.root / "second.tar.gz"
        shutil.copy2(self.archive, second)
        with open(install / DAEMON, "rb") as running:
            inode = os.fstat(running.fileno()).st_ino
            # The browser directory of this version exists already and is kept.
            result = self.run_installer(self.render_installer(node, second), home, tools)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(running.read(), ELF + b" daemon one")
        self.assertEqual((install / DAEMON).read_bytes(), ELF + b" daemon two")
        self.assertNotEqual((install / DAEMON).stat().st_ino, inode)
        self.assertEqual((install / REMOTE_ARM).read_bytes(), ELF + b" arm two")
        leftovers = [path.name for path in install.iterdir() if path.name.endswith(".new")]
        self.assertEqual(leftovers, [])

        # A release from before the daemon shipped: installs, and leaves
        # the daemon that is there. The builds for SSH servers are those of
        # another release, which this one never uploads: they are deleted.
        legacy = self.root / "legacy.tar.gz"
        with tarfile.open(legacy, "w:gz") as archive:
            archive.add(self.release / "shelldeck", "shelldeck")
        result = self.run_installer(self.render_installer(node, legacy), home, tools)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((install / DAEMON).read_bytes(), ELF + b" daemon two")
        self.assertFalse((install / REMOTE_X86).exists())
        self.assertFalse((install / REMOTE_ARM).exists())
        self.assertEqual((install / "shelldeck").read_bytes(), b"application fixture")

        # Nothing was written outside the temporary HOME's own tree.
        self.assertTrue(str(install).startswith(str(self.root)))

    # SDTEST-3036 (SDUC-574): the remote build script refuses to build for a
    # target that is not one of the two shipped ones, or not installed,
    # before it compiles anything.
    def test_sdtest_3036_remote_build_script_checks_targets_before_building(self):
        script = REPOSITORY / "scripts/build-sessiond-remote.sh"
        self.assertEqual(subprocess.run(["bash", "-n", str(script)]).returncode, 0)
        fake = self.root / "fake-bin"
        fake.mkdir()
        (fake / "rustup").write_text("#!/bin/sh\necho x86_64-unknown-linux-gnu\n")
        (fake / "rustup").chmod(0o755)
        (fake / "cargo").write_text("#!/bin/sh\necho cargo-was-run >&2\nexit 97\n")
        (fake / "cargo").chmod(0o755)
        environment = dict(os.environ, PATH=f"{fake}:/usr/bin:/bin")
        for arguments, message in (
            (["riscv64gc-unknown-linux-musl"], "Unsupported session daemon target"),
            ([], "Rust target x86_64-unknown-linux-musl is not installed"),
            (["aarch64-unknown-linux-musl"], "Rust target aarch64-unknown-linux-musl is not installed"),
        ):
            result = subprocess.run(["bash", str(script), *arguments], env=environment,
                                    text=True, capture_output=True, timeout=20)
            self.assertNotEqual(result.returncode, 0, arguments)
            self.assertIn(message, result.stderr)
            self.assertNotIn("cargo-was-run", result.stderr)


if __name__ == "__main__":
    unittest.main()
