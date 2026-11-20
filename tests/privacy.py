"""Synthetic repositories for per-ref privacy notice qualification."""
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import unittest

CHECKER = pathlib.Path(__file__).resolve().parents[1] / 'build/privacy.py'


class PrivacyTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = pathlib.Path(self.directory.name)
        self.env = dict(os.environ, GIT_AUTHOR_NAME='anPDF Contributors',
                        GIT_AUTHOR_EMAIL='contributors@anpdf.invalid',
                        GIT_COMMITTER_NAME='anPDF Contributors',
                        GIT_COMMITTER_EMAIL='contributors@anpdf.invalid')
        self.git('init', '-b', 'main')
        (self.root / 'sample.txt').write_text('Synthetic fixture')
        self.commit()

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.root, env=self.env,
                                       stderr=subprocess.DEVNULL)

    def commit(self):
        self.git('add', 'sample.txt')
        for name in ['license-manifest.json', 'notice.txt']:
            if (self.root / name).exists():
                self.git('add', name)
        self.git('commit', '-m', 'Synthetic privacy fixture')

    def notice(self, content=None):
        content = content or b'Synthetic notice: ' + b'synthetic' + b'@example.invalid'
        (self.root / 'notice.txt').write_bytes(content)
        return hashlib.sha256(content).hexdigest()

    def manifest(self, digest=None):
        notices = [{'sha256': digest}] if digest else []
        (self.root / 'license-manifest.json').write_text(json.dumps({
            'components': [{'notices': notices}]
        }))

    def check(self, expected):
        result = subprocess.run([sys.executable, str(CHECKER)], cwd=self.root,
                                capture_output=True, text=True)
        self.assertEqual(result.stderr, '')
        self.assertEqual(result.returncode, int(expected != 0))
        match = re.fullmatch(r'Privacy policy: \d+ commits, \d+ unique blobs, (\d+) findings\n', result.stdout)
        self.assertIsNotNone(match, result.stdout)
        self.assertEqual(int(match[1]), expected)

    def test_branch_uses_its_own_manifest(self):
        self.git('switch', '-c', 'licensed')
        self.manifest(self.notice())
        self.commit()
        self.git('switch', 'main')
        self.check(0)

    def test_refs_are_scanned_when_head_is_unborn(self):
        self.git('symbolic-ref', 'HEAD', 'refs/heads/unborn')
        self.check(0)

    def test_missing_manifest_and_duplicate_reachability(self):
        self.notice()
        self.commit()
        self.git('branch', 'also-reaches-notice')
        self.check(1)

    def test_every_reaching_ref_must_sanction(self):
        self.manifest(self.notice())
        self.commit()
        self.git('switch', '-c', 'unsanctioned')
        self.manifest()
        self.commit()
        self.git('switch', 'main')
        self.check(1)

    def test_paths_are_never_sanctioned_and_blob_counts_once(self):
        self.manifest(self.notice(b'/' + b'home/' + b'synthetic ' + b'synthetic' + b'@example.invalid'))
        self.commit()
        self.git('switch', '-c', 'unsanctioned')
        self.manifest()
        self.commit()
        self.check(1)

    def test_malformed_manifests_fail_without_values(self):
        for document in [{}, {'components': None}, {'components': [None]},
                         {'components': [{'notices': None}]},
                         {'components': [{'notices': [{'sha256': 'invalid'}]}]}]:
            with self.subTest(document=document):
                (self.root / 'license-manifest.json').write_text(json.dumps(document))
                self.commit()
                self.check(2)


if __name__ == '__main__':
    unittest.main()
