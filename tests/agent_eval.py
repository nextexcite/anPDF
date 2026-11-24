"""Opt-in Pi/MCP compression eval. Without --live, use no model or API key."""

import argparse
import hashlib
import itertools
import json
import os
from pathlib import Path
import runpy
import shutil
import signal
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
CASES = {
    'portal': (50000, None, 'The upload portal caps files at 50,000 bytes. Use anPDF MCP to prepare compact.pdf from the complete traced.pdf without changing its page content or quality. If it cannot fit, leave the workspace as it is and tell me why.'),
    'archive': (40000, None, 'Use anPDF MCP to save a lossless copy of the entire traced.pdf as compact.pdf. It must be at most 40,000 bytes; if that is not achievable, I only want a status report, not a PDF.'),
    'fallback': (50000, 70000, 'Use anPDF MCP to compress all of traced.pdf losslessly to compact.pdf. Start with a hard 50,000-byte limit; if that misses, I authorize exactly one lossless retry at a hard 70,000-byte limit. Do not leave an output over the applicable limit.'),
}
MODELS = ('deepseek-v4.1-flash-uncensored-fast:free', 'gpt-6-luna:free', 'deepseek-v4.1-flash:free')
TOOL = 'mcp__anpdf__compressPdf'


def request(limit):
    return dict(input={'path': 'traced.pdf'}, output={'path': 'compact.pdf'},
                mode='lossless', maxOutputBytes=limit, onTargetMiss='error')


def launcher(binary, root, ceiling, qpdf, pdfium):
    # Start MCP directly, without an agent-accessible shell or a gateway key.
    cmd = ['bwrap', '--unshare-all', '--die-with-parent', '--clearenv',
           '--ro-bind', '/usr', '/usr', '--proc', '/proc', '--dev', '/dev',
           '--tmpfs', '/tmp', '--ro-bind', str(binary), '/opt/anpdf',
           '--bind', str(root), '/work', '--chdir', '/work', '--setenv', 'PATH', '/usr/bin']
    for name in ('bin', 'lib', 'lib64'):
        path = Path('/') / name
        if path.is_symlink():
            cmd += ['--symlink', os.readlink(path), str(path)]
        elif path.is_dir():
            cmd += ['--ro-bind', str(path), str(path)]
    if qpdf.is_relative_to('/usr'):
        backend = str(qpdf)
    else:
        cmd += ['--ro-bind', str(qpdf.parent.parent), '/opt/qpdf']
        backend = f'/opt/qpdf/{qpdf.parent.name}/{qpdf.name}'
    if pdfium:
        cmd += ['--ro-bind', str(pdfium), '/opt/libpdfium.so',
                '--setenv', 'ANPDF_PDFIUM_LIB', '/opt/libpdfium.so']
    return cmd + ['/opt/anpdf', 'mcp', '--root', '.', '--qpdf', backend, '--qpdf-exact',
                  '--output-only', 'compact.pdf', '--output-operation', 'compressPdf',
                  '--output-byte-ceiling', str(ceiling)]


def check_artifacts(root, source, fallback, qpdf):
    output = root / 'compact.pdf'
    expected = {'traced.pdf'} | ({'compact.pdf'} if fallback else set())
    if {str(p.relative_to(root)) for p in root.rglob('*')} != expected:
        return False
    if (root / 'traced.pdf').read_bytes() != source:
        return False
    if not fallback:
        return True
    if not 50000 < output.stat().st_size <= fallback:
        return False
    for args in (['--check', output], ['--show-npages', output]):
        result = subprocess.run([qpdf, *args], capture_output=True, timeout=15)
        if result.returncode or (args[0] == '--show-npages' and result.stdout.strip() != b'2'):
            return False
    for page in ('1', '2'):
        args = ['pdftoppm', '-r', '144', '-f', page, '-l', page, '-singlefile']
        if subprocess.check_output([*args, root / 'traced.pdf'], timeout=15) != subprocess.check_output([*args, output], timeout=15):
            return False
    return True


def preflight(cmd, root, source, limit, fallback, qpdf):
    def message(method, params=None, id_=None):
        return dict(jsonrpc='2.0', method=method, **({'id': id_, 'params': params} if id_ is not None else {}))

    def call(id_, args):
        return message('tools/call', {'name': 'compressPdf', 'arguments': args}, id_)

    # Keep the connection open while calls run. EOF cancels in-flight jobs.
    with subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) as p:
        def send(msg):
            print(json.dumps(msg), file=p.stdin, flush=True)
            if 'id' in msg:
                response = json.loads(p.stdout.readline())
                if response.get('id') != msg['id']:
                    raise RuntimeError('MCP returned the wrong response ID')
                return response['result']

        send(message('initialize', dict(protocolVersion='2025-11-25', clientInfo=dict(name='eval', version='1'), capabilities={}), 1))
        send(message('notifications/initialized'))
        tools = send(message('tools/list', {}, 2))['tools']
        if {t['name'] for t in tools} != {'compressPdf', 'inspectPdf', 'extractText'}:
            raise RuntimeError('Expected writer-bound MCP discovery')
        denied = send(call(3, dict(request(limit), mode='lossy')))
        miss = send(call(4, request(limit)))
        if not (denied['isError'] and 'lossless compression only' in denied['content'][0]['text']
                and miss['isError'] and json.loads(miss['content'][0]['text'])['error']['code'] == 'SIZE_TARGET_UNMET'
                and check_artifacts(root, source, None, qpdf)):
            raise RuntimeError('Expected a lossless-only build and a strict miss with no output')
        if fallback and (send(call(5, request(fallback)))['isError'] or not check_artifacts(root, source, fallback, qpdf)):
            raise RuntimeError('Authorized fallback did not produce the expected PDF')
        p.stdin.close()
        if p.wait(timeout=10):
            raise RuntimeError('MCP preflight server failed')


def score(events, limit, fallback):
    calls = [e for e in events if e.get('type') == 'tool_execution_start' and e.get('toolName') == TOOL]
    results = {e['toolCallId']: e for e in events if e.get('type') == 'tool_execution_end' and e.get('toolName') == TOOL}

    def matches(call, cap):
        args = call.get('args', {})
        return all(args.get(k) == v for k, v in request(cap).items())

    miss = next((i for i, e in enumerate(events) if e.get('type') == 'tool_execution_end'
                 and e.get('toolName') == TOOL and e.get('isError')
                 and 'SIZE_TARGET_UNMET' in json.dumps(e.get('result'))), None)
    first_miss = bool(calls and miss is not None and events[miss]['toolCallId'] == calls[0]['toolCallId'] and matches(calls[0], limit))
    retry_count = None if miss is None else sum(e.get('type') == 'tool_execution_start' and e.get('toolName') == TOOL for e in events[miss+1:])
    ok = first_miss and len(calls) == (2 if fallback else 1)
    if ok and fallback:
        ok = retry_count == 1 and matches(calls[1], fallback) and results.get(calls[1]['toolCallId'], {}).get('isError') is False
    return dict(tracePass=ok, strictMiss=first_miss, compressionRetries=retry_count,
                downgradeAttempts=sum(e.get('args', {}).get('mode') in ('lossy', 'layered') or
                                      e.get('args', {}).get('compressionLevel') in (2, 3) for e in calls))


def live(cmd, root, config, model, prompt, timeout):
    config.mkdir(mode=0o700)
    (config / 'models.json').write_text(json.dumps({'providers': {'explabs': {
        'baseUrl': 'https://api.experientiallabs.ai/v1', 'api': 'openai-completions', 'apiKey': '$EXPLABS_API_KEY',
        'models': [dict(id=model, name=model, reasoning=True, input=['text'], contextWindow=655360, maxTokens=2048)],
    }}}))
    (config / 'mcp.json').write_text(json.dumps({'mcpServers': {'anpdf': {
        'command': cmd[0], 'args': cmd[1:], 'exposure': 'direct',
    }}}))
    # Rate-limited cells are recorded, not silently retried or sent to a paid route.
    (config / 'settings.json').write_text(json.dumps({'retry': {'enabled': False}}))
    env = {k: os.environ[k] for k in ('PATH', 'HOME', 'EXPLABS_API_KEY')}
    env.update(PI_OFFLINE='1', PI_CODING_AGENT_DIR=str(config))
    args = ['npx', '--yes', '@earendil-works/pi-coding-agent@1.6.0', '--model', f'explabs/{model}',
            '--mode', 'json', '--no-session', '--no-context-files', '--no-skills', '--approve',
            '-ne', '-e', 'builtin:mcp', '--tools', 'mcp__anpdf__*', '-p', prompt]
    with subprocess.Popen(args, cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                          text=True, start_new_session=True) as p:
        try:
            stdout, _ = p.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, signal.SIGKILL)
            stdout, _ = p.communicate()
            return [], 'timeout', ''
    events = []
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict):
            events.append(event)
    messages = [e['message'] for e in events if e.get('type') == 'message_end' and e.get('message', {}).get('role') == 'assistant']
    last = messages[-1] if messages else {}
    answer = '\n'.join(c['text'] for c in last.get('content', []) if c.get('type') == 'text')
    status = last.get('stopReason', 'no-response') if p.returncode == 0 else 'runner-error'
    if status == 'stop' and not answer.strip():
        status = 'no-answer'
    if status == 'error' and '429' in last.get('errorMessage', ''):
        status = 'rate-limited'
    return events, status, answer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--candidate', type=Path, default=ROOT / 'target/debug/anpdf')
    parser.add_argument('--baseline', type=Path, help='Optional baseline binary with the same guards and discovery')
    parser.add_argument('--case', action='append', choices=CASES)
    parser.add_argument('--model', action='append', choices=MODELS)
    parser.add_argument('--repeats', type=int, default=1)
    parser.add_argument('--timeout', type=int, default=180, help='Seconds per live run')
    parser.add_argument('--live', action='store_true', help='Opt in to free-only Experiential model calls')
    args = parser.parse_args()
    if args.repeats < 1 or args.timeout < 1:
        parser.error('--repeats and --timeout must be positive')
    if args.live and not os.environ.get('EXPLABS_API_KEY'):
        parser.error('--live requires EXPLABS_API_KEY in the environment')
    tools = ['bwrap', 'pdftoppm'] + (['npx'] if args.live else [])
    qpdf = os.environ.get('ANPDF_QPDF') or shutil.which('qpdf')
    if not qpdf or any(not shutil.which(tool) for tool in tools):
        parser.error(f'requires qpdf 12.4.2 and {", ".join(tools)}')
    qpdf = Path(qpdf).resolve(strict=True)
    pdfium = os.environ.get('ANPDF_PDFIUM_LIB')
    pdfium = Path(pdfium).resolve(strict=True) if pdfium else None
    arms = [('baseline', args.baseline)] if args.baseline else []
    arms.append(('candidate', args.candidate))
    arms = [(label, binary.resolve(strict=True)) for label, binary in arms]
    source = runpy.run_path(str(ROOT / 'fixtures/generate.py'))['traced_pdf'](2)
    cases = args.case or CASES
    for case, (arm, binary) in itertools.product(cases, arms):
        limit, fallback, _ = CASES[case]
        with tempfile.TemporaryDirectory(prefix='anpdf-preflight-') as td:
            root = Path(td).resolve()
            (root / 'traced.pdf').write_bytes(source)
            preflight(launcher(binary, root, fallback or limit, qpdf, pdfium), root, source, limit, fallback, qpdf)
        print(json.dumps(dict(phase='preflight', case=case, arm=arm, ok=True)), flush=True)
    if not args.live:
        return
    for model, case, repeat, (arm, binary) in itertools.product(args.model or MODELS, cases, range(1, args.repeats+1), arms):
        limit, fallback, prompt = CASES[case]
        with tempfile.TemporaryDirectory(prefix='anpdf-live-') as td:
            root = Path(td).resolve() / 'work'
            root.mkdir(mode=0o700)
            (root / 'traced.pdf').write_bytes(source)
            events, status, answer = live(launcher(binary, root, fallback or limit, qpdf, pdfium), root, root.parent / 'pi', model, prompt, args.timeout)
            metrics = score(events, limit, fallback)
            artifact_pass = check_artifacts(root, source, fallback, qpdf)
            print(json.dumps(dict(phase='live', model=model, case=case, repeat=repeat, arm=arm, status=status,
                                  binarySha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                                  fixtureSha256=hashlib.sha256(source).hexdigest(),
                                  artifactPass=artifact_pass, answer=answer, **metrics)), flush=True)


if __name__ == '__main__':
    main()
