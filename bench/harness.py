"""External-command engine comparison; see docs/BENCHMARK.md."""
import argparse
from collections import Counter
import json
import math
import os
import pathlib
import shlex
import signal
import statistics
import subprocess
import sys
import tempfile
import time

if __package__ in (None, ''):
    sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from bench.oracle import inspect_pdf, signature_evidence
from PIL import Image, ImageStat

TASKS = ['pageCount', 'extractText', 'render', 'inspect', 'roundtrip']
FEATURES = ['outlines', 'links', 'annotations', 'formFields', 'signatureFields', 'metadata', 'imageCount']


def text_score(expected, actual):
    a, b = ''.join(expected.split()), ''.join(actual.split())
    if not a and not b:
        return 1.0
    if len(a) < len(b):
        a, b = b, a
    row = list(range(len(b) + 1))
    for i, char in enumerate(a, 1):
        new = [i]
        for j, other in enumerate(b, 1):
            new.append(min(new[-1] + 1, row[j] + 1, row[j-1] + (char != other)))
        row = new
    return 1 - row[-1] / max(len(a), len(b))


def canonical(value):
    if isinstance(value, dict):
        return {key: canonical(item) for key, item in value.items()}
    if isinstance(value, list):
        return sorted((canonical(item) for item in value), key=lambda item: json.dumps(item, sort_keys=True))
    return value


def atoms(value, prefix='', unordered=False):
    if isinstance(value, dict):
        return [atom for key, item in sorted(value.items()) for atom in atoms(item, prefix + '/' + key, unordered)]
    if isinstance(value, list):
        if unordered:
            return [(prefix, json.dumps(canonical(item), sort_keys=True)) for item in value]
        return [atom for i, item in enumerate(value) for atom in atoms(item, prefix + '/' + str(i))]
    return [(prefix, json.dumps(value, sort_keys=True))]


def compare(expected, actual, unordered=False):
    a, b = Counter(atoms(expected, unordered=unordered)), Counter(atoms(actual, unordered=unordered))
    common = sum((a & b).values())
    precision = common / sum(b.values()) if b else float(not a)
    recall = common / sum(a.values()) if a else float(not b)
    return {'precision': precision, 'recall': recall,
            'score': 2 * precision * recall / (precision + recall) if precision + recall else 0.0}


def invoke(command, mode, request, timeout):
    """Use regular files to avoid pipe deadlocks; reap exactly once with wait4."""
    with tempfile.TemporaryFile() as stdin, tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        stdin.write(json.dumps(request).encode()); stdin.seek(0)
        started = time.monotonic()
        process = subprocess.Popen(command + [mode], stdin=stdin, stdout=stdout, stderr=stderr, start_new_session=True)
        timed_out = False
        while True:
            pid, status, usage = os.wait4(process.pid, os.WNOHANG)
            if pid:
                break
            if time.monotonic() - started >= timeout:
                timed_out = True
                os.killpg(process.pid, signal.SIGKILL)
                _, status, usage = os.wait4(process.pid, 0)
                break
            time.sleep(0.005)
        process.returncode = os.waitstatus_to_exitcode(status)
        result = {'status': 'timeout' if timed_out else 'crash' if process.returncode < 0 else 'error' if process.returncode else 'ok',
                  'exitCode': process.returncode, 'seconds': time.monotonic() - started,
                  'peakRssBytes': int(usage.ru_maxrss * (1 if sys.platform == 'darwin' else 1024))}
        stdout.seek(0)
        try:
            result['result'] = json.loads(stdout.read(8 * 1024 * 1024))
            if not isinstance(result['result'], dict):
                raise ValueError('Expected a JSON object')
            if result['status'] == 'ok' and result['result'].get('status') == 'unsupported':
                result['status'] = 'unsupported'
        except (ValueError, UnicodeDecodeError):
            if result['status'] == 'ok':
                result['status'] = 'protocolError'
        return result


def preservation(source, output):
    before, after = inspect_pdf(source), inspect_pdf(output)
    scores = {field: compare(before[field], after.get(field), unordered=field != 'annotations')['recall'] for field in FEATURES
              if before[field] and (field != 'annotations' or any(before[field]))}
    scores['pages'] = float(before['pages'] == after['pages'])
    scores['text'] = text_score('\n'.join(before['text']), '\n'.join(after['text']))
    if any(f['signed'] for f in before['signatureFields']):
        original, saved = signature_evidence(source), signature_evidence(output)
        scores['signatureIntegrity'] = float(bool(original) and original == saved and all(s['valid'] for s in saved))
    return {'score': statistics.mean(scores.values()), 'fields': scores}


def score(task, case, result, output, source, dpi=72, page=1):
    expected = case['expected']
    if result['status'] == 'unsupported':
        return None
    if task == 'extractText' and case.get('ocrOnly'):
        return 0.0 if result['status'] in ('crash', 'timeout') else None
    if expected.get('expectError'):
        if result['status'] == 'error':
            return 1.0
        if result['status'] != 'ok':
            return 0.0
        value = result.get('result', {})
        if task == 'pageCount':
            return float(type(value.get('pages')) is int and value['pages'] >= 0)
        if task == 'extractText':
            return float(isinstance(value.get('text'), list) and all(isinstance(t, str) for t in value['text']))
        if task == 'inspect':
            return float(all(field in value or field in value.get('unsupportedFields', []) for field in FEATURES))
        if task == 'render':
            with Image.open(output) as image:
                image.verify()
            return 1.0
        with open(output, 'rb') as stream:
            return float(stream.read(5) == b'%PDF-')
    if result['status'] != 'ok':
        return 0.0
    value = result['result']
    if task == 'pageCount':
        return float(value.get('pages') == expected['pages'])
    if task == 'extractText':
        text = value.get('text', [])
        return statistics.mean(text_score(e, text[i] if i < len(text) else '') for i, e in enumerate(expected['text'])) if len(text) <= expected['pages'] else 0.0
    if task == 'inspect':
        return {key: {'status': 'unsupported'} if key in value.get('unsupportedFields', []) else compare(expected[key], value.get(key)) for key in FEATURES}
    if task == 'roundtrip':
        return preservation(source, output)
    if task == 'render':
        width, height = expected['pageSizes'][page-1]
        if expected['rotations'][page-1] in (90, 270):
            width, height = height, width
        with Image.open(output) as image:
            dimensions = (math.ceil(width * dpi / 72), math.ceil(height * dpi / 72))
            return float(image.size == dimensions and max(ImageStat.Stat(image.convert('RGB')).stddev) > 0.1)


def numeric_score(value):
    if value is None:
        return None
    if isinstance(value, dict):
        return value['score'] if 'score' in value else statistics.mean(scores) if (scores := [v['score'] for v in value.values() if 'score' in v]) else None
    return value


def deployment_size(paths):
    files = set()
    for item in paths:
        path = pathlib.Path(item)
        files.update(p.resolve() for p in path.rglob('*') if p.is_file()) if path.is_dir() else files.add(path.resolve())
    return sum(path.stat().st_size for path in files)


def run(corpus, providers, out, repeat=1, timeout=10):
    corpus, out = pathlib.Path(corpus).resolve(), pathlib.Path(out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    manifest = json.loads((corpus / 'manifest.json').read_text())
    results = {'schemaVersion': 1, 'repeat': repeat, 'timeoutSeconds': timeout, 'providers': {}, 'runs': [], 'skipped': manifest['skipped']}
    for name, command in providers.items():
        capability = invoke(command, 'capabilities', {}, timeout)
        if capability['status'] != 'ok':
            raise ValueError('Provider capability discovery failed: ' + name)
        caps = capability['result']
        results['providers'][name] = {'name': caps['name'], 'version': caps['version'], 'tasks': caps['tasks'],
                                      'deploymentBytes': deployment_size(caps.get('artifacts', []))}
        for case in manifest['cases']:
            source = corpus / case['file']
            for task in TASKS:
                pages = range(1, case['expected'].get('pages', 1) + 1) if task == 'render' else [None]
                for page in pages:
                    row = {'provider': name, 'case': case['id'], 'category': case['category'], 'task': task, 'page': page}
                    if task not in caps['tasks']:
                        row.update(status='unsupported', score=None)
                    else:
                        samples = []
                        for index in range(repeat):
                            output = out / f'{name}-{case["id"]}-{task}-{page}-{index}{".png" if task == "render" else ".pdf"}'
                            output.unlink(missing_ok=True)
                            request = {'task': task, 'input': str(source), 'output': str(output), 'page': page or 1, 'dpi': 72}
                            observation = invoke(command, 'run', request, timeout)
                            try:
                                observation['score'] = score(task, case, observation, output, source, page=page or 1)
                            except Exception:
                                observation['score'] = 0.0
                                observation['oracleError'] = True
                            samples.append(observation)
                        row.update(status=next((s['status'] for s in samples if s['status'] != 'ok'), 'ok'),
                                   score=statistics.mean(values) if (values := [value for s in samples if (value := numeric_score(s['score'])) is not None]) else None,
                                   seconds=statistics.median(s['seconds'] for s in samples),
                                   peakRssBytes=max(s['peakRssBytes'] for s in samples), samples=samples)
                    results['runs'].append(row)
    (out / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
    report = ['# Synthetic PDF engine comparison', '',
              'Scores are corpus-specific, not product qualification. Unsupported tasks are excluded, not failures.', '',
              '| Provider | Category | Correctness | Preservation | Median seconds | Peak RSS bytes | Supported runs |',
              '| --- | --- | ---: | ---: | ---: | ---: | ---: |']
    def mean(values):
        return f'{statistics.mean(values):.3f}' if values else 'n/a'
    for name in providers:
        for category in sorted({r['category'] for r in results['runs']} | {'ALL'}):
            rows = [r for r in results['runs'] if r['provider'] == name and (category == 'ALL' or r['category'] == category) and r['status'] != 'unsupported']
            correctness = [r['score'] for r in rows if (r['task'] != 'roundtrip' or r['category'] == 'malformed') and r['score'] is not None]
            retained = [r['score'] for r in rows if r['task'] == 'roundtrip' and r['category'] != 'malformed' and r['score'] is not None]
            report.append(f'| {name} | {category} | {mean(correctness)} | {mean(retained)} | {statistics.median([r["seconds"] for r in rows]):.4f} | {max(r["peakRssBytes"] for r in rows)} | {len(rows)} |' if rows else f'| {name} | {category} | n/a | n/a | n/a | n/a | 0 |')
    report += ['', '## Deployment artifacts', '', '| Provider | Version | Bytes |', '| --- | --- | ---: |']
    for name, caps in results['providers'].items():
        report.append(f'| {name} | {caps["version"]} | {caps["deploymentBytes"]} |')
    report += ['', 'Size covers declared artifacts only, not a complete dependency closure. RSS includes the command wrapper; see protocol limitations.', '', '## Skips', '', json.dumps(manifest['skipped'], ensure_ascii=False), '']
    (out / 'report.md').write_text('\n'.join(report))
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--corpus', required=True); parser.add_argument('--provider', action='append', required=True)
    parser.add_argument('--out', required=True); parser.add_argument('--repeat', type=int, default=1)
    parser.add_argument('--timeout', type=float, default=10)
    args = parser.parse_args()
    if args.repeat < 1 or args.timeout <= 0:
        parser.error('repeat and timeout must be positive')
    providers = {}
    for item in args.provider:
        name, command = item.split('=', 1)
        if not name or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-' for c in name):
            parser.error('provider names must be filename-safe ASCII identifiers')
        providers[name] = shlex.split(command)
    run(args.corpus, providers, args.out, args.repeat, args.timeout)
    print('Wrote results.json and report.md')
