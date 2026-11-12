#!/usr/bin/env python3
"""Mac icon/archive contracts; native consumers run on macOS, no app or profile."""
from pathlib import Path
import os
import plistlib
import shutil
import struct
import subprocess
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
ICON = ROOT / 'packaging/icons/shelldeck.icns'


class MacBranding(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='inklura mac packaging ')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for folder in ['packaging/macos', 'scripts', 'target/release']:
            (self.root / folder).mkdir(parents=True)
        for name in ['packaging/macos/build-dmg.sh', 'scripts/package-release.sh']:
            shutil.copyfile(ROOT / name, self.root / name)
        (self.root / 'Cargo.toml').write_text('[workspace.package]\nversion = "9.8.7"\n')
        self.binary = self.root / 'target/release/shelldeck'
        self.binary.write_bytes(b'not executed by the packaging fixture')
        self.binary.chmod(0o755)
        self.daemon = self.root / 'target/release/shelldeck-sessiond'
        self.daemon.write_bytes(b'not executed by the packaging fixture')
        self.daemon.chmod(0o755)

    # SDTEST-2885 / SDUC-566: the actual shipped icon is accepted by Apple's decoder.
    @unittest.skipUnless(shutil.which('iconutil'), 'native Apple iconutil is required; macOS CI runs this')
    def test_sdtest_2885_canonical_icon_decodes_with_native_iconutil(self):
        data = ICON.read_bytes()
        self.assertEqual(data[:4], b'icns')
        self.assertEqual(struct.unpack('>I', data[4:8])[0], len(data))
        output = self.root / 'native decoded.iconset'
        subprocess.run(['iconutil', '-c', 'iconset', str(ICON), '-o', str(output)], check=True)
        for filename, size in [('icon_256x256.png', 256), ('icon_512x512@2x.png', 1024)]:
            png = (output / filename).read_bytes()
            self.assertEqual(png[:8], b'\x89PNG\r\n\x1a\n')
            self.assertEqual(struct.unpack('>II', png[16:24]), (size, size))

    # SDTEST-2886 / SDUC-566: missing input fails before replacing an existing bundle.
    def test_sdtest_2886_dmg_build_refuses_missing_icon(self):
        bundle = self.root / 'dist/Inklura Desktop.app'
        bundle.mkdir(parents=True)
        sentinel = bundle / 'existing-bundle'
        sentinel.write_text('preserve on preflight failure')
        result = subprocess.run(['bash', str(self.root / 'packaging/macos/build-dmg.sh'), str(self.binary)],
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Required macOS icon is missing', result.stderr)
        self.assertEqual(sentinel.read_text(), 'preserve on preflight failure')

    # SDTEST-3065 (SDUC-574): a bundle is not built, and its archive is not
    # published, without the session daemon beside the application binary.
    def test_sdtest_3065_bundle_and_archive_require_the_session_daemon(self):
        bundle = self.root / 'dist/Inklura Desktop.app'
        bundle.mkdir(parents=True)
        sentinel = bundle / 'existing-bundle'
        sentinel.write_text('preserve on preflight failure')
        self.daemon.unlink()
        result = subprocess.run(['bash', str(self.root / 'packaging/macos/build-dmg.sh'), str(self.binary)],
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Session daemon not found', result.stderr)
        self.assertEqual(sentinel.read_text(), 'preserve on preflight failure')
        shutil.rmtree(bundle)

        app = self.prepare_bundle(daemon=False)
        shutil.copyfile(ICON, app / 'Contents/Resources/shelldeck.icns')
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('macOS bundle has no session daemon', result.stderr)
        self.assertFalse((self.root / 'dist/shelldeck-macos-aarch64.zip').exists())

    def prepare_bundle(self, daemon=True):
        app = self.root / 'dist/Inklura Desktop.app'
        contents = app / 'Contents'
        (contents / 'Resources').mkdir(parents=True)
        (contents / 'MacOS').mkdir()
        shutil.copyfile(self.binary, contents / 'MacOS/shelldeck')
        if daemon:
            shutil.copy2(self.daemon, contents / 'MacOS/shelldeck-sessiond')
        browser = contents / 'Frameworks/browser/ShellDeck Browser.app/Contents'
        (browser / 'MacOS').mkdir(parents=True)
        helper = browser / 'MacOS/shelldeck-browser'
        helper.write_bytes(b'not executed')
        helper.chmod(0o755)
        framework = browser / 'Frameworks/Chromium Embedded Framework.framework'
        framework.mkdir(parents=True)
        (framework / 'Chromium Embedded Framework').write_bytes(b'browser fixture')
        (contents / 'Info.plist').write_bytes(plistlib.dumps({'CFBundleName': 'Inklura Desktop',
                                                            'CFBundleIconFile': 'shelldeck'}))
        return app

    def package(self):
        return subprocess.run(['bash', str(self.root / 'scripts/package-release.sh')],
                              env=dict(os.environ, SHELLDECK_RELEASE_PLATFORM='macos-aarch64'),
                              capture_output=True, text=True)

    # SDTEST-2887 / SDUC-566: the final archive is independently guarded, too.
    def test_sdtest_2887_update_archive_refuses_missing_bundle_icon(self):
        self.prepare_bundle()
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('macOS bundle icon is missing', result.stderr)
        self.assertFalse((self.root / 'dist/shelldeck-macos-aarch64.zip').exists())

    # SDTEST-2888 / SDUC-566: exercise real macOS ZIP packaging, including spaces.
    @unittest.skipUnless(shutil.which('ditto'), 'native Apple ditto is required; macOS CI runs this')
    def test_sdtest_2888_update_archive_preserves_the_canonical_icon(self):
        app = self.prepare_bundle()
        shutil.copyfile(ICON, app / 'Contents/Resources/shelldeck.icns')
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        with zipfile.ZipFile(self.root / 'dist/shelldeck-macos-aarch64.zip') as archive:
            self.assertEqual(archive.read('Inklura Desktop.app/Contents/Resources/shelldeck.icns'), ICON.read_bytes())
            self.assertIn('Inklura Desktop.app/Contents/MacOS/shelldeck-sessiond', archive.namelist())


if __name__ == '__main__':
    unittest.main()
