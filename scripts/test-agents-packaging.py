#!/usr/bin/env python3
"""Offline launcher/package contracts; no user profile, app, network or Rust build."""
from pathlib import Path
import configparser
import json
import os
import shutil
import subprocess
import tempfile
import unittest

REPOSITORY = Path(__file__).resolve().parent.parent
VERSION = "9.8.7"
RESOURCES = ("libcef.so", "icudtl.dat", "resources.pak", "chrome_100_percent.pak",
             "chrome_200_percent.pak", "v8_context_snapshot.bin", "locales/en-US.pak", "runtime.json")
NSS_MODULES = ("libsoftokn3.so", "libfreebl3.so", "libfreeblpriv3.so", "libnssckbi.so", "libnssdbm3.so")


class AgentsPackaging(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="shelldeck agents packaging ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for folder in ("packaging/linux", "packaging/icons", ".build-tools", "target/release"):
            (self.root / folder).mkdir(parents=True)
        for name in ("build-appimage.sh", "shelldeck.desktop"):
            shutil.copy2(REPOSITORY / "packaging/linux" / name, self.root / "packaging/linux" / name)
        shutil.copy2(REPOSITORY / "packaging/icons/shelldeck-256.png",
                     self.root / "packaging/icons/shelldeck-256.png")
        (self.root / "Cargo.toml").write_text(f'[workspace.package]\nversion = "{VERSION}"\n')
        self.browser = self.root / "target/release/browser" / VERSION
        self.browser.mkdir(parents=True, mode=0o700)
        for resource in RESOURCES:
            target = self.browser / resource
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("browser fixture: " + resource)
        self.executable(self.browser / "shelldeck-browser", "#!/bin/sh\nexit 0\n")
        (self.browser / "library-alias.so").symlink_to("libcef.so")
        stale = self.browser.parent / "0.0.1"
        stale.mkdir()
        (stale / "must-not-ship.txt").write_text("stale runtime")
        self.executable(self.root / "target/release/shelldeck", "#!/bin/sh\nprintf '%s\\n' \"$@\"\n")
        self.executable(self.root / ".build-tools/linuxdeploy-x86_64.AppImage",
                        """#!/bin/sh
printf '%s\\n' "$@" > "$SHELLDECK_TEST_DEPLOY_LOG"
appdir="$2"
mkdir -p "$appdir/usr/lib"
while [ "$#" -gt 0 ]; do
    if [ "$1" = "--library" ]; then
        shift
        if [ "${SHELLDECK_TEST_SKIP_NSS:-0}" != "1" ]; then
            cp "$1" "$appdir/usr/lib/"
        fi
    fi
    shift
done
exit "${SHELLDECK_TEST_DEPLOY_EXIT:-0}"
""")
        self.executable(self.root / ".build-tools/appimagetool-x86_64.AppImage",
                        "#!/bin/sh\nfor output do :; done\nprintf 'offline AppImage fixture\\n' > \"$output\"\n")
        (self.root / "fixture-tools").mkdir()
        self.executable(self.root / "fixture-tools/ldd", """#!/bin/sh
case "$1" in
    */libcef.so) printf 'libnss3.so => %s/libnss3.so (0x123)\\n' "$SHELLDECK_TEST_NSS_DIRECTORY"; exit 0 ;;
esac
if [ "${SHELLDECK_TEST_LOADER_MISSING:-0}" = "1" ]; then
    printf 'libcef.so => not found\\n'
elif [ "${SHELLDECK_TEST_LOADER_WRONG:-0}" = "1" ]; then
    printf 'libcef.so => /external/libcef.so (0x123)\\n'
else
    case "$1" in
        */shelldeck-browser) printf 'libcef.so => %s/libcef.so (0x123)\\n' "$(dirname "$1")" ;;
        *) printf 'libc.so.6 => /lib/libc.so.6 (0x123)\\n' ;;
    esac
fi
""")
        self.nss = self.root / "host NSS distribution"
        (self.nss / "nss").mkdir(parents=True)
        (self.nss / "libnss3.so").write_text("matching host NSS")
        for module in NSS_MODULES:
            (self.nss / "nss" / module).write_text("matching NSS module: " + module)
        self.environment = dict(os.environ, SHELLDECK_APPIMAGE_SIGN="0",
                                SHELLDECK_APPIMAGE_VERIFY_RUNTIME="0",
                                PATH=str(self.root / "fixture-tools") + os.pathsep + os.environ["PATH"],
                                SHELLDECK_TEST_NSS_DIRECTORY=str(self.nss),
                                SHELLDECK_TEST_DEPLOY_LOG=str(self.root / "deploy-arguments.txt"))

    def executable(self, path, content):
        path.write_text(content)
        path.chmod(0o755)

    def package(self):
        return subprocess.run(["bash", "packaging/linux/build-appimage.sh"], cwd=self.root,
                              env=self.environment, text=True, capture_output=True, timeout=15)

    def desktop(self, path):
        parser = configparser.ConfigParser(interpolation=None)
        parser.optionxform = str
        parser.read(path)
        return parser

    # SDTEST-2630 — SDUC-558, SDUC-557: the production AppDir keeps its versioned
    # browser and Agents action; AppRun forwards --agents and spaced args intact.
    def test_sdtest_2630_appimage_launches_agents_with_matching_browser_payload(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        appdir = self.root / "dist/ShellDeck.AppDir"
        desktop = self.desktop(appdir / "shelldeck.desktop")
        self.assertEqual(desktop["Desktop Entry"]["Exec"], "shelldeck %u")
        self.assertEqual(desktop["Desktop Entry"]["MimeType"], "x-scheme-handler/shelldeck;")
        self.assertIn("Agents", desktop["Desktop Entry"]["Actions"].split(";"))
        self.assertEqual(desktop["Desktop Action Agents"]["Exec"], "shelldeck --agents")
        launch = subprocess.run([str(appdir / "AppRun"), "--agents", "a spaced argument"],
                                text=True, capture_output=True, timeout=5)
        self.assertEqual(launch.returncode, 0, launch.stderr)
        self.assertEqual(launch.stdout.splitlines(), ["--agents", "a spaced argument"])
        packaged = appdir / "usr/bin/browser" / VERSION
        self.assertEqual((packaged / "locales/en-US.pak").read_text(), "browser fixture: locales/en-US.pak")
        self.assertTrue((packaged / "library-alias.so").is_symlink())
        self.assertEqual(packaged.stat().st_mode & 0o005, 0o005)
        self.assertEqual((packaged / "runtime.json").stat().st_mode & 0o004, 0o004)
        self.assertFalse((packaged.parent / "0.0.1").exists())
        deployed_args = (self.root / "deploy-arguments.txt").read_text().splitlines()
        self.assertIn(str(packaged), deployed_args)
        self.assertIn(str(packaged / "shelldeck-browser"), deployed_args)
        self.assertIn("--exclude-library", deployed_args)
        self.assertIn("libcef.so", deployed_args)
        self.assertTrue((self.root / "dist/ShellDeck-x86_64.AppImage").is_file())

    # SDTEST-2631 — SDUC-558, SDUC-557: no successful release artifact can be
    # reported when the native browser is incomplete or dependency bundling fails.
    def test_sdtest_2631_appimage_refuses_missing_browser_and_failed_dependencies(self):
        resource = self.browser / "locales/en-US.pak"
        resource.unlink()
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Missing browser resource", result.stderr)
        self.assertFalse((self.root / "deploy-arguments.txt").exists())
        resource.write_text("restored fixture")
        self.environment["SHELLDECK_TEST_DEPLOY_EXIT"] = "1"
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Could not bundle", result.stderr)
        self.assertFalse((self.root / "dist/ShellDeck-x86_64.AppImage").exists())
        self.environment["SHELLDECK_TEST_DEPLOY_EXIT"] = "0"
        for mode, diagnostic in [("MISSING", "Unresolved packaged loader dependency"),
                                 ("WRONG", "did not resolve its matching versioned libcef")]:
            with self.subTest(loader=mode):
                self.environment["SHELLDECK_TEST_LOADER_" + mode] = "1"
                result = self.package()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(diagnostic, result.stderr)
                self.assertFalse((self.root / "dist/ShellDeck-x86_64.AppImage").exists())
                self.environment.pop("SHELLDECK_TEST_LOADER_" + mode)

    # SDTEST-2644 — SDUC-557, SDUC-558: NSS modules are dlopened by Chromium,
    # absent from ldd's dependency graph. Bundle the matching distribution and
    # reject either unavailable modules or a tool silently omitting them.
    def test_sdtest_2644_appimage_includes_matching_dynamic_nss_modules(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        appdir = self.root / "dist/ShellDeck.AppDir"
        deployed = (self.root / "deploy-arguments.txt").read_text().splitlines()
        for module in NSS_MODULES:
            self.assertIn(str(self.nss / "nss" / module), deployed)
            self.assertEqual((appdir / "usr/lib" / module).read_text(), "matching NSS module: " + module)
        # A successful previous artifact cannot satisfy a failed new build.
        (self.root / "dist/ShellDeck-x86_64.AppImage").unlink()
        missing = self.nss / "nss/libsoftokn3.so"
        missing.unlink()
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Missing Chromium NSS runtime module: libsoftokn3.so", result.stderr)
        self.assertFalse((self.root / "dist/ShellDeck-x86_64.AppImage").exists())
        missing.write_text("restored matching NSS module")
        self.environment["SHELLDECK_TEST_SKIP_NSS"] = "1"
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("NSS runtime module was not deployed", result.stderr)
        self.assertFalse((self.root / "dist/ShellDeck-x86_64.AppImage").exists())

    # SDTEST-2632 — SDUC-558: actual worker rendering and desktop-file generation
    # keep normal/protocol launch separate from Agents; NSIS compiles with a
    # fixture payload. Native Windows shortcut activation remains a release check.
    def test_sdtest_2632_installers_keep_production_agents_arguments_separate(self):
        node = shutil.which("node")
        makensis = shutil.which("makensis")
        if not node or not makensis:
            self.skipTest("requires Node with TypeScript stripping and NSIS; no native app or network is used")
        version = subprocess.run([node, "-p", "process.versions.node"], text=True,
                                 capture_output=True, timeout=5)
        self.assertEqual(version.returncode, 0, version.stderr)
        node_version = tuple(int(part) for part in version.stdout.strip().split(".")[:2])
        if node_version < (22, 6):
            self.skipTest("requires Node 22.6+ TypeScript stripping; CI explicitly installs Node 24")
        module = (REPOSITORY / "cloudflare/update-worker/src/index.ts").as_uri()
        javascript = f'''import worker from {json.dumps(module)};
const env = {{SHELLDECK_KV: {{get: async () => null}}}};
const output = {{}};
for (const suffix of ["sh", "ps1"]) {{
    const response = await worker.fetch(new Request("https://example.invalid/install." + suffix), env);
    output[suffix] = await response.text();
}}
process.stdout.write(JSON.stringify(output));'''
        generated = subprocess.run([node, "--experimental-strip-types", "--input-type=module", "-e", javascript],
                                   text=True, capture_output=True, timeout=10)
        self.assertEqual(generated.returncode, 0, generated.stderr)
        scripts = json.loads(generated.stdout)
        syntax = subprocess.run(["bash", "-n"], input=scripts["sh"], text=True,
                                capture_output=True, timeout=5)
        self.assertEqual(syntax.returncode, 0, syntax.stderr)
        start = scripts["sh"].index('  cat > "$APPS_DIR/shelldeck.desktop" <<DESKTOP')
        end = scripts["sh"].index("\nDESKTOP", start) + len("\nDESKTOP")
        applications = self.root / "installed applications"
        applications.mkdir()
        install = self.root / "installed binary path"
        environment = dict(os.environ, APPS_DIR=str(applications), INSTALL_DIR=str(install))
        registration = subprocess.run(["bash"], input=scripts["sh"][start:end] + "\n",
                                      text=True, capture_output=True, env=environment, timeout=5)
        self.assertEqual(registration.returncode, 0, registration.stderr)
        desktop = self.desktop(applications / "shelldeck.desktop")
        self.assertEqual(desktop["Desktop Entry"]["Exec"], f'"{install}/shelldeck" %u')
        self.assertEqual(desktop["Desktop Action Agents"]["Exec"], f'"{install}/shelldeck" --agents')
        self.assertIn('$Shortcut.TargetPath = $InstalledExe', scripts["ps1"])
        self.assertIn('Name="ShellDeck Agents"; Arguments="--agents"', scripts["ps1"])
        self.assertIn('$Shortcut.Arguments = $Launch.Arguments', scripts["ps1"])
        self.assertIn('$Shortcut.Save()', scripts["ps1"])

        (self.root / "packaging/windows").mkdir()
        shutil.copy2(REPOSITORY / "packaging/windows/shelldeck.nsi", self.root / "packaging/windows/shelldeck.nsi")
        shutil.copy2(REPOSITORY / "LICENSE", self.root / "LICENSE")
        (self.root / "target/release/shelldeck.exe").write_bytes(b"offline installer binary fixture")
        (self.root / "dist").mkdir()
        result = subprocess.run([makensis, "-V2", "-NOCD", f"-DVERSION={VERSION}", "packaging/windows/shelldeck.nsi"],
                                cwd=self.root, text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue((self.root / "dist/ShellDeck-windows-x86_64-setup.exe").is_file())


if __name__ == "__main__":
    unittest.main()
