#!/usr/bin/env python3
"""Offline archive contract tests. No app, CEF, provider, or installed profile is launched."""
from pathlib import Path
import os
import shutil
import subprocess
import tarfile
import tempfile
import unittest
import zipfile

REPOSITORY = Path(__file__).resolve().parent.parent
VERSION = "9.8.7"
RESOURCES = (
    "icudtl.dat", "resources.pak", "chrome_100_percent.pak", "chrome_200_percent.pak",
    "v8_context_snapshot.bin", "locales/en-US.pak", "runtime.json",
)


class BrowserPackaging(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="shelldeck packaging fixture ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "scripts").mkdir()
        shutil.copy2(REPOSITORY / "scripts/package-release.sh", self.root / "scripts/package-release.sh")
        (self.root / "Cargo.toml").write_text(f'[workspace.package]\nversion = "{VERSION}"\n[workspace.dependencies]\n')
        self.release = self.root / "target/release"
        self.browser = self.release / "browser" / VERSION
        self.browser.mkdir(parents=True)
        for resource in RESOURCES:
            path = self.browser / resource
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(f"browser fixture {resource}".encode())
        stale = self.release / "browser/0.0.1"
        stale.mkdir()
        (stale / "must-not-ship.txt").write_text("old loaded runtime")

    def put(self, path):
        result = self.release / path
        result.write_bytes(f"fixture {path}".encode())
        result.chmod(0o755)
        return result

    def package(self, platform):
        environment = dict(os.environ, SHELLDECK_RELEASE_PLATFORM=platform)
        return subprocess.run(["bash", "scripts/package-release.sh"], cwd=self.root,
                              env=environment, text=True, capture_output=True)

    # SDTEST-2595 — SDUC-557: Linux update archives carry the matching executable,
    # CEF resources and symlinks, excluding other loaded browser versions.
    def test_sdtest_2595_linux_archive_preserves_versioned_browser(self):
        self.put("shelldeck")
        self.put(f"browser/{VERSION}/shelldeck-browser")
        self.put(f"browser/{VERSION}/libcef.so")
        (self.browser / "library-alias.so").symlink_to("libcef.so")
        result = self.package("linux-x86_64")
        self.assertEqual(result.returncode, 0, result.stderr)
        with tarfile.open(self.root / "dist/shelldeck-linux-x86_64.tar.gz") as archive:
            names = archive.getnames()
            self.assertIn(f"browser/{VERSION}/locales/en-US.pak", names)
            self.assertFalse(any("0.0.1" in name for name in names))
            helper = archive.getmember(f"browser/{VERSION}/shelldeck-browser")
            self.assertTrue(helper.mode & 0o100)
            self.assertEqual(archive.getmember(f"browser/{VERSION}/library-alias.so").linkname, "libcef.so")
            self.assertEqual(archive.extractfile(f"browser/{VERSION}/locales/en-US.pak").read(),
                             b"browser fixture locales/en-US.pak")

    # SDTEST-2596 — SDUC-557: Windows update ZIPs retain the versioned bootstrap,
    # client DLL and nested resources instead of flattening or including stale DLLs.
    def test_sdtest_2596_windows_archive_preserves_versioned_browser(self):
        for path in ("shelldeck.exe", f"browser/{VERSION}/shelldeck-browser.exe",
                     f"browser/{VERSION}/shelldeck-browser.dll", f"browser/{VERSION}/libcef.dll"):
            self.put(path)
        result = self.package("windows-x86_64")
        self.assertEqual(result.returncode, 0, result.stderr)
        with zipfile.ZipFile(self.root / "dist/shelldeck-windows-x86_64.zip") as archive:
            self.assertIn("shelldeck.exe", archive.namelist())
            self.assertIn(f"browser/{VERSION}/shelldeck-browser.dll", archive.namelist())
            self.assertFalse(any("0.0.1" in name for name in archive.namelist()))
            self.assertEqual(archive.read(f"browser/{VERSION}/locales/en-US.pak"),
                             b"browser fixture locales/en-US.pak")

    # SDTEST-2597 — SDUC-557: an incomplete browser runtime cannot produce a
    # successful release archive on Linux, Windows, or macOS.
    def test_sdtest_2597_incomplete_browser_refuses_packaging(self):
        for path in ("shelldeck", "shelldeck.exe", f"browser/{VERSION}/shelldeck-browser",
                     f"browser/{VERSION}/shelldeck-browser.exe", f"browser/{VERSION}/shelldeck-browser.dll",
                     f"browser/{VERSION}/libcef.so", f"browser/{VERSION}/libcef.dll"):
            self.put(path)
        (self.browser / "locales/en-US.pak").unlink()
        (self.root / "dist/ShellDeck.app").mkdir(parents=True)
        for platform in ("linux-x86_64", "windows-x86_64", "macos-aarch64"):
            with self.subTest(platform=platform):
                result = self.package(platform)
                self.assertNotEqual(result.returncode, 0)
        self.assertEqual(list((self.root / "dist").glob("shelldeck-*")), [])


if __name__ == "__main__":
    unittest.main()
