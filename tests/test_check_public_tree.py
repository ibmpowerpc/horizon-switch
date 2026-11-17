"""Exercise the staged public-content gate in an isolated Git index."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


GATE = Path(__file__).resolve().parents[1] / 'tools/check_public_tree.py'


class PublicTreeTests(unittest.TestCase):
    def test_staged_gate_checks_index_content_without_unchanged_blob_reads(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            env = dict(os.environ, GIT_NO_LAZY_FETCH='1')
            def git(*args):
                return subprocess.run(['git', *args], cwd=root, env=env,
                                      check=True, capture_output=True, text=True)
            def check():
                return subprocess.run([sys.executable, str(GATE), '--staged'],
                                      cwd=root, env=env, capture_output=True, text=True)
            git('init', '-q')
            # A partial clone may have an unchanged index entry whose blob is
            # unavailable locally. The pre-commit gate need not fetch it.
            old = root / 'old.rs'
            old.write_text('unchanged baseline\n')
            git('add', 'old.rs')
            git('-c', 'user.name=Test', '-c', 'user.email=test@example.invalid',
                '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'baseline')
            oid = git('rev-parse', 'HEAD:old.rs').stdout.strip()
            (root / '.git/objects' / oid[:2] / oid[2:]).unlink()

            added = root / 'new.rs'
            added.write_text('original code\n')
            git('add', 'new.rs')
            self.assertEqual(check().returncode, 0)
            added.write_text('\n'.join('minecraft:stone.' + str(i) for i in range(50)))
            git('add', 'new.rs')
            self.assertNotEqual(check().returncode, 0)
            added.write_text('original code\n')
            git('add', 'new.rs')
            (root / 'client.jar').write_bytes(b'not distributable')
            git('add', 'client.jar')
            result = check()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('client.jar', result.stderr)


if __name__ == '__main__':
    unittest.main()
