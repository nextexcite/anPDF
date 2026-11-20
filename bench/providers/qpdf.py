"""Reference qpdf command provider."""
import json
import os
import pathlib
import shutil
import subprocess
import sys


def main():
    binary = os.environ.get('ANPDF_QPDF') or shutil.which('qpdf')
    if not binary:
        raise RuntimeError('qpdf unavailable')
    if sys.argv[1] == 'capabilities':
        print(json.dumps({'name': 'qpdf', 'version': subprocess.check_output([binary, '--version'], text=True).splitlines()[0],
                          'tasks': ['pageCount', 'roundtrip'], 'artifacts': [str(pathlib.Path(binary).resolve())]}))
        return
    request = json.load(sys.stdin)
    args = [binary, '--show-npages', request['input']] if request['task'] == 'pageCount' else [binary, request['input'], request['output']]
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode < 0:
        os.kill(os.getpid(), -result.returncode)
    if result.returncode not in (0, 3):
        sys.exit(1)
    print(json.dumps({'pages': int(result.stdout.strip())} if request['task'] == 'pageCount' else {'saved': True}))


if __name__ == '__main__':
    main()
