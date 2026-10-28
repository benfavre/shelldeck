#!/usr/bin/env python3
"""Exercise the production CI credential scripts without GitHub or real secrets."""
import json
import os
from pathlib import Path
import shlex
import stat
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
ACTION = ROOT / '.github/actions/automonique-read-access'
DUMMY_KEY = 'TEST_ONLY_DEPENDENCY_KEY_NOT_A_CREDENTIAL'


class DependencyAccessTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='dependency access test ')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.log = self.root / 'calls.jsonl'
        self.environment = self.root / 'job.env'
        self.environment.touch()
        self.env = os.environ.copy()
        self.env.update(PATH=str(self.bin) + os.pathsep + self.env['PATH'],
                        RUNNER_TEMP=str(self.root), RUNNER_OS='Linux',
                        GITHUB_ACTION_PATH=str(ACTION), GITHUB_ENV=str(self.environment),
                        AUTOMONIQUE_DEPENDENCY_KEY=DUMMY_KEY, TEST_CALLS=str(self.log))
        self.stub('git', '''import json, os, sys
with open(os.environ['TEST_CALLS'], 'a') as log:
    log.write(json.dumps(sys.argv[1:]) + '\\n')
if sys.argv[1] == 'ls-remote' and os.environ.get('TEST_FAIL_READ'):
    sys.exit(1)
''')

    def stub(self, name, source):
        path = self.bin / name
        path.write_text('#!/usr/bin/env python3\n' + source)
        path.chmod(0o755)

    def run_script(self, name, env=None):
        return subprocess.run(['bash', str(ACTION / name)], env=env or self.env,
                              capture_output=True, text=True)

    # SDTEST-2707 / SDUC-560
    def test_sdtest_2707_key_scope_permissions_cleanup_and_windows_acl(self):
        for platform in ('Linux', 'Windows'):
            with self.subTest(platform=platform):
                self.env['RUNNER_OS'] = platform
                if platform == 'Windows':
                    self.env['USERNAME'] = 'fixture-runner'
                    self.stub('cygpath', 'import sys\nprint(sys.argv[2])\n')
                    self.stub('icacls', '''import json, os, sys
assert os.environ['MSYS2_ARG_CONV_EXCL'] == '*'
with open(os.environ['TEST_CALLS'], 'a') as log:
    log.write(json.dumps(['icacls'] + sys.argv[1:]) + '\\n')
''')
                result = self.run_script('setup.sh')
                self.assertEqual(result.returncode, 0, result.stderr)
                exported = dict(line.split('=', 1) for line in self.environment.read_text().splitlines())
                self.assertNotIn(DUMMY_KEY, result.stdout + result.stderr + self.environment.read_text())
                key = Path(exported['AUTOMONIQUE_DEPENDENCY_KEY_PATH'])
                hosts = Path(exported['AUTOMONIQUE_DEPENDENCY_HOSTS_PATH'])
                self.assertEqual(stat.S_IMODE(key.stat().st_mode), 0o600)
                self.assertEqual(key.read_text().strip(), DUMMY_KEY)
                self.assertEqual(hosts.read_text(), (ACTION / 'github_known_hosts').read_text())
                command = shlex.split(exported['GIT_SSH_COMMAND'])
                self.assertEqual(command[command.index('-F') + 1], 'none')
                self.assertEqual(command[command.index('-i') + 1], str(key))
                self.assertIn('IdentitiesOnly=yes', command)
                self.assertIn('StrictHostKeyChecking=yes', command)
                calls = [json.loads(line) for line in self.log.read_text().splitlines()]
                self.assertIn(['config', '--global', 'url.ssh://git@github.com/bext-stack/automonique.insteadOf',
                               'https://github.com/bext-stack/automonique'], calls)
                if platform == 'Windows':
                    self.assertIn(['icacls', str(key), '/inheritance:r', '/grant:r', 'fixture-runner:R'], calls)
                cleanup_env = self.env | exported
                self.assertEqual(self.run_script('cleanup.sh', cleanup_env).returncode, 0)
                self.assertFalse(key.exists())
                self.assertFalse(hosts.exists())

    # SDTEST-2708 / SDUC-560
    def test_sdtest_2708_failed_probe_leaves_no_key_and_cleanup_rejects_foreign_files(self):
        self.env['TEST_FAIL_READ'] = '1'
        result = self.run_script('setup.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(list(self.root.glob('shelldeck-automonique-*')), [])
        self.assertEqual(self.environment.read_text(), '')
        self.assertNotIn(DUMMY_KEY, result.stdout + result.stderr)
        foreign = self.root / 'keep-me'
        foreign.write_text('keep')
        result = self.run_script('cleanup.sh', self.env | {'AUTOMONIQUE_DEPENDENCY_KEY_PATH': str(foreign)})
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(foreign.exists())
        traversal = str(self.root / 'shelldeck-automonique-directory' / '..' / 'keep-me')
        result = self.run_script('cleanup.sh', self.env | {'AUTOMONIQUE_DEPENDENCY_KEY_PATH': traversal})
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(foreign.exists())

    # SDTEST-2709 / SDUC-560
    def test_sdtest_2709_public_workflows_never_cache_git_or_grant_fork_credentials(self):
        for name in ('ci.yml', 'release.yml'):
            source = (ROOT / '.github/workflows' / name).read_text()
            self.assertNotIn('~/.cargo/git', source)
            self.assertNotIn('pull_request_target:', source)
            count = source.count('uses: ./.github/actions/automonique-read-access')
            self.assertGreater(count, 0)
            self.assertEqual(count, source.count("if: github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository"))
            self.assertEqual(count, source.count('name: Remove temporary dependency credential\n        if: always()'))
        security = (ROOT / '.github/workflows/security.yml').read_text()
        self.assertIn("head.repo.full_name == github.repository) && secrets.AUTOMONIQUE_READ_DEPLOY_KEY || ''", security)
        self.assertNotIn('pull_request_target:', security)


if __name__ == '__main__':
    unittest.main()
