"""Tests run without external PDF executables; optional CJK uses the pinned font."""
import hashlib
import json
import os
import pathlib
import sys
import tempfile
import unittest
from unittest.mock import patch

from PIL import Image
from pypdf import PdfReader, PdfWriter
from bench.corpus import generate
from bench.harness import compare, deployment_size, invoke, preservation, run, score, text_score
from bench.oracle import inspect_pdf, signature_evidence


class CorpusTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.work = tempfile.TemporaryDirectory()
        cls.root = pathlib.Path(cls.work.name)
        cls.manifest = generate(cls.root / 'a')
        cls.other = generate(cls.root / 'b')

    @classmethod
    def tearDownClass(cls):
        cls.work.cleanup()

    def test_deterministic(self):
        for a, b in zip(self.manifest['cases'], self.other['cases']):
            if not a.get('nondeterministic'):
                self.assertEqual(a, b)
                self.assertEqual((self.root / 'a' / a['file']).read_bytes(), (self.root / 'b' / b['file']).read_bytes())

    def test_manifest_and_recipe_truth(self):
        cases = {c['id']: c for c in self.manifest['cases']}
        for case in cases.values():
            path = self.root / 'a' / case['file']
            raw = path.read_bytes()
            self.assertEqual(hashlib.sha256(raw).hexdigest(), case['sha256'])
            self.assertEqual(len(raw), case['bytes'])
            self.assertEqual('Apache-2.0', case['license'])
            if case['category'] == 'malformed':
                self.assertTrue(case['expected']['expectError'])
                continue
            actual = inspect_pdf(path)
            if case.get('ocrOnly'):
                self.assertEqual([''], actual['text'])
                actual['text'] = case['expected']['text']
            self.assertEqual(case['expected'], actual)
        self.assertEqual(3, cases['text-mixed']['expected']['pages'])
        self.assertEqual(['Synthetic page 1\n', 'Synthetic page 2\n', 'Synthetic page 3\n'], cases['text-mixed']['expected']['text'])
        self.assertEqual([0, 90, 0], cases['text-mixed']['expected']['rotations'])
        self.assertEqual('Section', cases['bookmarks']['expected']['outlines'][0]['children'][0]['title'])
        self.assertEqual([{'page': 1, 'uri': 'https://example.invalid/'}, {'page': 1, 'destPage': 2}], cases['links']['expected']['links'])
        self.assertEqual(['FreeText', 'Highlight', 'Ink', 'Square', 'Text'], cases['annotations']['expected']['annotations'][0])
        self.assertEqual(5, len(cases['forms']['expected']['formFields']))
        reader = PdfReader(self.root / 'a' / 'forms.pdf')
        self.assertTrue(all('/AP' in a.get_object() for a in reader.pages[0]['/Annots']))
        self.assertEqual(12, cases['images']['expected']['imageCount'])
        if 'cjk' in cases:
            self.assertIn('中文页面测试 日本語テスト', cases['cjk']['expected']['text'][0])

    def test_signature_integrity(self):
        evidence = signature_evidence(self.root / 'a' / 'signed.pdf')
        self.assertTrue(evidence[0]['valid'])
        self.assertEqual([{'name': 'UnsignedSignature', 'signed': False}], inspect_pdf(self.root / 'a' / 'unsigned.pdf')['signatureFields'])
        self.assertEqual(1, preservation(self.root / 'a' / 'signed.pdf', self.root / 'a' / 'signed.pdf')['score'])

    def test_no_font_skip(self):
        manifest = generate(self.root / 'no-font', font=self.root / 'missing.ttf', real_signed=False)
        self.assertEqual({'cjk', 'signed'}, {s['id'] for s in manifest['skipped']})

    def test_preservation_loss(self):
        output = self.root / 'empty.pdf'
        writer = PdfWriter(); writer.add_blank_page(360, 480)
        with output.open('wb') as stream:
            writer.write(stream)
        retained = preservation(self.root / 'a' / 'bookmarks.pdf', output)
        self.assertLess(retained['score'], 1)
        self.assertEqual(0, retained['fields']['outlines'])


class ScoringTests(unittest.TestCase):
    def test_native_provider_signal_is_crash(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            for name in ['qpdf', 'pdfinfo', 'pdftotext', 'pdftoppm']:
                binary = root / name
                binary.write_text(f'#!{sys.executable}\nimport os,signal\nos.kill(os.getpid(), signal.SIGTERM)\n')
                binary.chmod(0o755)
            with patch.dict(os.environ, {'ANPDF_QPDF': str(root / 'qpdf'), 'PATH': str(root)}):
                for provider, tasks in [('qpdf', ['pageCount', 'roundtrip']), ('poppler', ['pageCount', 'extractText', 'render'])]:
                    for task in tasks:
                        with self.subTest(provider=provider, task=task):
                            result = invoke([sys.executable, f'bench/providers/{provider}.py'], 'run',
                                            {'task': task, 'input': 'synthetic.pdf', 'output': str(root / 'out.png'), 'page': 1, 'dpi': 72}, 5)
                            self.assertEqual('crash', result['status'])
                            self.assertEqual(0, score(task, {'expected': {'expectError': True}}, result, None, None))

    def test_text(self):
        self.assertEqual(1, text_score('', ''))
        self.assertEqual(1, text_score('中 文\n日本語', '中文 日本語'))
        self.assertEqual(0.5, text_score('中文', '中日'))
        self.assertEqual(0, text_score('abc', ''))

    def test_fields(self):
        self.assertEqual(1, compare([], [])['score'])
        self.assertEqual(0, compare(['link'], [])['recall'])
        result = compare(['a'], ['a', 'b'])
        self.assertEqual(0.5, result['precision'])
        self.assertEqual(1, result['recall'])

    def test_malformed_roundtrip_crash_in_summary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / 'bad.pdf').write_bytes(b'garbage')
            (root / 'manifest.json').write_text(json.dumps({'cases': [{'id': 'bad', 'category': 'malformed', 'file': 'bad.pdf', 'expected': {'expectError': True}}], 'skipped': []}))
            command = [sys.executable, '-c', 'import sys,json,os,signal; print(json.dumps({"name":"fake","version":"1","tasks":["roundtrip"],"artifacts":[]})) if sys.argv[1]=="capabilities" else os.kill(os.getpid(),signal.SIGTERM)']
            run(root, {'fake': command}, root / 'out')
            self.assertIn('| fake | malformed | 0.000 | n/a |', (root / 'out' / 'report.md').read_text())

    def test_malformed(self):
        for status, expected in [('ok', 1), ('error', 1), ('crash', 0), ('timeout', 0), ('protocolError', 0)]:
            self.assertEqual(expected, score('pageCount', {'expected': {'expectError': True}}, {'status': status, 'result': {'pages': 1}}, None, None))
        self.assertEqual(0, score('pageCount', {'expected': {'expectError': True}}, {'status': 'ok', 'result': {}}, None, None))

    def test_malformed_partial_inspect(self):
        result = {'status': 'ok', 'result': {'metadata': {'Title': 'Repaired'},
                  'unsupportedFields': ['outlines', 'links', 'annotations', 'formFields', 'signatureFields', 'imageCount']}}
        self.assertEqual(1, score('inspect', {'expected': {'expectError': True}}, result, None, None))
        del result['result']['metadata']
        self.assertEqual(0, score('inspect', {'expected': {'expectError': True}}, result, None, None))

    def test_render(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'render.png'
            case = {'expected': {'pageSizes': [[10, 20]], 'rotations': [90]}}
            image = Image.new('RGB', (20, 10), 'white'); image.putpixel((0, 0), (0, 0, 0)); image.save(path)
            self.assertEqual(1, score('render', case, {'status': 'ok', 'result': {}}, path, None))
            Image.new('RGB', (20, 10), 'white').save(path)
            self.assertEqual(0, score('render', case, {'status': 'ok', 'result': {}}, path, None))

    def test_subprocess_outcomes(self):
        for program, expected in [('import json; print("{}")', 'ok'), ('raise SystemExit(2)', 'error'),
                                  ('print("not json")', 'protocolError'),
                                  ('import os,signal; os.kill(os.getpid(),signal.SIGTERM)', 'crash'),
                                  ('import time; time.sleep(20)', 'timeout')]:
            result = invoke([sys.executable, '-c', program], 'run', {}, 0.2)
            self.assertEqual(expected, result['status'])
            self.assertGreater(result['peakRssBytes'], 0)

    def test_end_to_end(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            # The fake provider has no third-party dependencies or PDF parsing.
            command = [sys.executable, '-c', 'import sys,json; print(json.dumps({"name":"fake","version":"1","tasks":["pageCount"],"artifacts":[]} if sys.argv[1]=="capabilities" else {"pages":1}))']
            corpus = root / 'corpus'; corpus.mkdir()
            (corpus / 'one.pdf').write_bytes(b'synthetic placeholder')
            (corpus / 'manifest.json').write_text(json.dumps({'cases': [{'id': 'one', 'category': 'text', 'file': 'one.pdf', 'expected': {'pages': 1}}], 'skipped': []}))
            results = run(corpus, {'fake': command}, root / 'out', repeat=2)
            self.assertEqual(1, results['runs'][0]['score'])
            self.assertEqual('unsupported', results['runs'][1]['status'])
            self.assertTrue((root / 'out' / 'report.md').is_file())
            self.assertEqual(0, deployment_size([]))
            self.assertEqual(21, deployment_size([corpus / 'one.pdf', corpus / 'one.pdf']))


FAKE_ANPDF = '''#!/usr/bin/env python3
import json, os, pathlib, signal, sys
command = sys.argv[1]
if command == '--version':
    print('anpdf synthetic')
elif command == 'operations':
    print(json.dumps({'operations': [
        {'operationId':'inspectPdf', 'engines':['qpdf']},
        {'operationId':'extractText', 'engines':['pdfium']},
        {'operationId':'renderPage', 'engines':['pdfium']}],
        'artifacts':[os.environ['FAKE_LIBRARY']]}))
else:
    root = pathlib.Path(sys.argv[sys.argv.index('--root') + 1])
    request = json.loads(sys.argv[sys.argv.index('--request-json') + 1])
    assert request['input'] == {'path':'input.pdf'}
    assert (root / 'input.pdf').is_file()
    if command == 'inspect':
        assert 'engine' not in request
        result = {'pageCount':int(os.environ.get('FAKE_PAGES','1')), 'metadata':{'title':'Synthetic'}}
    else:
        assert request['engine'] == 'pdfium'
        mode = os.environ.get('FAKE_MODE')
        if mode == 'crash':
            os.kill(os.getpid(), signal.SIGTERM)
        if mode in ('BACKEND_UNAVAILABLE', 'INVALID_PDF'):
            print(json.dumps({'error':{'code':mode}}), file=sys.stderr)
            sys.exit(5)
        if command == 'extract-text':
            assert 1 <= len(request['pages']) <= 100
            result = {'engine':'pdfium','pages':[{'page':p,'text':'Synthetic','spans':[]} for p in request['pages']]}
        else:
            from PIL import Image
            assert request['output'] == {'path':'render.png'}
            image = Image.new('RGB',(360,480),'white')
            image.putpixel((0,0),(0,0,0))
            image.save(root / 'render.png')
            result = {'path':'render.png','engine':'pdfium'}
    print(json.dumps({'result':result,'warnings':[],'contractVersion':'1.0.0'}))
'''


class AnpdfAdapterTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.work.name)
        binary = self.root / 'fake-anpdf'
        binary.write_text(FAKE_ANPDF)
        binary.chmod(0o755)
        library = self.root / 'synthetic-provider.so'; library.write_bytes(b'synthetic artifact')
        self.environment = patch.dict(os.environ, {'ANPDF_BIN': str(binary), 'ANPDF_ENGINE': 'pdfium', 'FAKE_LIBRARY': str(library)})
        self.environment.start()
        self.command = [str(pathlib.Path('bench/providers/anpdf').resolve())]
        self.source = self.root / 'source.pdf'; self.source.write_bytes(b'synthetic input')
        self.request = {'input': str(self.source), 'output': str(self.root / 'render.png'), 'page': 1, 'dpi': 72}

    def tearDown(self):
        self.environment.stop()
        self.work.cleanup()

    def test_capabilities_and_mapping(self):
        result = invoke(self.command, 'capabilities', {}, 5)
        self.assertEqual('ok', result['status'])
        self.assertEqual(['pageCount', 'inspect', 'extractText', 'render'], result['result']['tasks'])
        self.assertEqual(2, len(result['result']['artifacts']))
        for task, field, expected in [('pageCount', 'pages', 1), ('extractText', 'text', ['Synthetic']), ('inspect', 'metadata', {'Title': 'Synthetic'}), ('render', 'saved', True)]:
            with self.subTest(task=task):
                response = invoke(self.command, 'run', dict(self.request, task=task), 5)
                self.assertEqual('ok', response['status'])
                self.assertEqual(expected, response['result'][field])
        self.assertEqual(b'synthetic input', self.source.read_bytes())
        with Image.open(self.root / 'render.png') as image:
            self.assertEqual((360, 480), image.size)
        response = invoke(self.command + ['--engine', 'pdfoxide'], 'capabilities', {}, 5)
        self.assertEqual(['pageCount', 'inspect'], response['result']['tasks'])

    def test_errors_and_unsupported(self):
        for mode, status in [('BACKEND_UNAVAILABLE', 'unsupported'), ('INVALID_PDF', 'error'), ('crash', 'crash')]:
            with self.subTest(mode=mode), patch.dict(os.environ, {'FAKE_MODE': mode}):
                response = invoke(self.command, 'run', dict(self.request, task='extractText'), 5)
                self.assertEqual(status, response['status'])
                if status == 'unsupported':
                    self.assertIsNone(score('extractText', {'expected': {}}, response, None, None))

    def test_text_request_chunking(self):
        with patch.dict(os.environ, {'FAKE_PAGES': '201'}):
            response = invoke(self.command, 'run', dict(self.request, task='extractText'), 5)
        self.assertEqual(201, len(response['result']['text']))


if __name__ == '__main__':
    unittest.main()
