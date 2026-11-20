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

    def identity_hash(self, identity):
        keys = ['GIT_AUTHOR_NAME', 'GIT_AUTHOR_EMAIL',
                'GIT_COMMITTER_NAME', 'GIT_COMMITTER_EMAIL']
        return hashlib.sha256(('\n'.join(identity[k] for k in keys) + '\n').encode()).hexdigest()

    def baseline(self, hashes, exceptions=None):
        (self.root / 'build').mkdir(exist_ok=True)
        (self.root / 'build/privacy-baseline.json').write_text(json.dumps({
            'identityExceptions': exceptions or [], 'webMergeIdentitySha256': hashes
        }))

    def synthetic_commit(self, identity, parents):
        args = ['git', 'commit-tree', self.git('rev-parse', 'HEAD^{tree}').decode().strip()]
        for parent in parents:
            args.extend(['-p', parent])
        commit = subprocess.check_output(args, cwd=self.root, env=identity,
                                         input=b'Synthetic identity fixture\n').decode().strip()
        self.git('update-ref', 'refs/heads/candidate', commit)
        return commit

    def approved_identity(self):
        return dict(self.env, GIT_AUTHOR_NAME='Synthetic merge author')

    def test_approved_merge_identity_passes(self):
        identity = self.approved_identity()
        self.baseline([self.identity_hash(identity)])
        parent = self.git('rev-parse', 'HEAD').decode().strip()
        other = self.synthetic_commit(self.env, [parent])
        self.synthetic_commit(identity, [parent, other])
        self.check(0)

    def test_approved_identity_rejects_single_parent_and_root(self):
        identity = self.approved_identity()
        self.baseline([self.identity_hash(identity)])
        parent = self.git('rev-parse', 'HEAD').decode().strip()
        for parents in [[parent], []]:
            with self.subTest(parents=parents):
                self.synthetic_commit(identity, parents)
                self.check(1)

    def test_merge_requires_every_identity_field_to_match(self):
        identity = self.approved_identity()
        self.baseline([self.identity_hash(identity)])
        parent = self.git('rev-parse', 'HEAD').decode().strip()
        other = self.synthetic_commit(self.env, [parent])
        variants = {
            'GIT_AUTHOR_NAME': 'Different synthetic author',
            'GIT_AUTHOR_EMAIL': 'different' + '@example.invalid',
            'GIT_COMMITTER_NAME': 'Different synthetic committer',
            'GIT_COMMITTER_EMAIL': 'different' + '@example.invalid',
        }
        for key, value in variants.items():
            with self.subTest(field=key):
                self.synthetic_commit(dict(identity, **{key: value}), [parent, other])
                self.check(1)

    def test_malformed_web_merge_baselines_fail(self):
        for hashes in [None, {}, 'a' * 64, ['a' * 63], ['g' * 64],
                       [42], ['a' * 64, 'a' * 64]]:
            with self.subTest(hashes=hashes):
                self.baseline(hashes)
                self.check(1)
        path = self.root / 'build/privacy-baseline.json'
        for document in [
            {'identityExceptions': []},
            {'identityExceptions': [], 'webMergeIdentitySha256': [], 'unknown': []},
        ]:
            path.write_text(json.dumps(document))
            self.check(1)
        path.write_text('{"identityExceptions":[],"webMergeIdentitySha256":[],"webMergeIdentitySha256":[]}')
        self.check(1)

    def test_per_commit_exception_still_requires_exact_hash(self):
        identity = self.approved_identity()
        parent = self.git('rev-parse', 'HEAD').decode().strip()
        commit = self.synthetic_commit(identity, [parent])
        exception = {'commit': commit, 'identitySha256': self.identity_hash(identity),
                     'reason': 'Synthetic reviewed exception.'}
        self.baseline([], [exception])
        self.check(0)
        exception['identitySha256'] = '0' * 64
        self.baseline([], [exception])
        self.check(1)


if __name__ == '__main__':
    unittest.main()
