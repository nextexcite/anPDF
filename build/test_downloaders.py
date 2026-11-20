"""Cached downloader paths must be rediscovered without network access."""
import contextlib
import io
import pathlib
import runpy
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parent


class CachedDownloaderPaths(unittest.TestCase):
    def query(self, name, root, system='Linux', argument='--path-only'):
        script = ROOT / f'get-{name}.py'
        output = io.StringIO()
        with patch('sys.argv', [str(script), str(root), argument]), \
             patch('platform.system', return_value=system), \
             patch('platform.machine', return_value='arm64' if system == 'Darwin' else 'x86_64'), \
             patch('urllib.request.urlopen', side_effect=AssertionError('unexpected network access')), \
             contextlib.redirect_stdout(output):
            with self.assertRaises(SystemExit) as result:
                runpy.run_path(str(script), run_name='__main__')
            self.assertEqual(result.exception.code, 0)
        return output.getvalue().strip()

    def populate(self, root, system):
        (root / 'bin').mkdir(parents=True)
        (root / 'lib').mkdir()
        binary = root / 'bin/qpdf'
        binary.write_text('#!/bin/sh\nexit 0\n')
        binary.chmod(0o755)
        library = root / 'lib' / ('libpdfium.dylib' if system == 'Darwin' else 'libpdfium.so')
        library.write_bytes(b'synthetic library')
        (root / 'lib/libqpdf-version').write_bytes(b'synthetic qpdf library')
        (root / 'lib/libqpdf-link').symlink_to('libqpdf-version')
        return binary, library

    def test_canonical_paths_on_linux_and_macos(self):
        for system in ('Linux', 'Darwin'):
            with self.subTest(system=system), tempfile.TemporaryDirectory() as tmp:
                root = pathlib.Path(tmp) / 'real root'
                binary, library = self.populate(root, system)
                alias = pathlib.Path(tmp) / 'alias'
                alias.symlink_to(root, target_is_directory=True)
                for name, expected in [('qpdf', binary), ('pdfium', library)]:
                    printed = self.query(name, alias, system)
                    self.assertEqual(printed, str(expected.resolve()))
                    self.assertEqual(pathlib.Path(printed).resolve(), (alias / expected.relative_to(root)).resolve())

    def test_missing_cache_fails_without_network(self):
        for name in ('qpdf', 'pdfium'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as tmp:
                with self.assertRaisesRegex(AssertionError, 'missing'):
                    self.query(name, pathlib.Path(tmp))

    def test_nonexecutable_qpdf_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary, _ = self.populate(pathlib.Path(tmp), 'Linux')
            binary.chmod(0o644)
            with self.assertRaisesRegex(AssertionError, 'missing executable qpdf'):
                self.query('qpdf', pathlib.Path(tmp))

    def test_dangling_paths_fail(self):
        for name, relative in [('qpdf', 'bin/qpdf'), ('pdfium', 'lib/libpdfium.so')]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as tmp:
                root = pathlib.Path(tmp)
                self.populate(root, 'Linux')
                path = root / relative
                path.unlink()
                path.symlink_to('missing')
                with self.assertRaisesRegex(AssertionError, 'missing'):
                    self.query(name, root)

    def test_unknown_arguments_fail(self):
        for name in ('qpdf', 'pdfium'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as tmp:
                with self.assertRaisesRegex(AssertionError, 'unsupported downloader arguments'):
                    self.query(name, pathlib.Path(tmp), argument='--unknown')

    def test_tar_restore_preserves_paths_modes_and_links(self):
        tar = shutil.which('gtar') or shutil.which('tar')
        self.assertIsNotNone(tar)
        for system in ('Linux', 'Darwin'):
            with self.subTest(system=system), tempfile.TemporaryDirectory() as tmp:
                base = pathlib.Path(tmp)
                original = base / 'original'
                self.populate(original, system)
                archive = base / 'cache.tar'
                subprocess.run([tar, '-cf', str(archive), '-C', str(original), '.'], check=True)
                restored = base / 'different checkout'
                restored.mkdir()
                subprocess.run([tar, '-xf', str(archive), '-C', str(restored)], check=True)
                self.assertEqual((restored / 'bin/qpdf').stat().st_mode & 0o777, 0o755)
                self.assertEqual((restored / 'lib/libqpdf-link').readlink(), pathlib.Path('libqpdf-version'))
                for name, relative in [('qpdf', 'bin/qpdf'), ('pdfium', 'lib/libpdfium.dylib' if system == 'Darwin' else 'lib/libpdfium.so')]:
                    self.assertEqual(self.query(name, restored, system), str(restored / relative))


if __name__ == '__main__':
    unittest.main()
