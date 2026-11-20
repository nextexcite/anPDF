"""Reference Poppler command provider."""
import json
import pathlib
import re
import shutil
import subprocess
import sys


def execute(args):
    return subprocess.run(args, capture_output=True, check=True).stdout.decode('utf-8')


def main():
    binaries = [shutil.which(name) for name in ['pdfinfo', 'pdftotext', 'pdftoppm']]
    if not all(binaries):
        raise RuntimeError('Poppler tools unavailable')
    if sys.argv[1] == 'capabilities':
        version = subprocess.run([binaries[0], '-v'], capture_output=True, text=True).stderr.splitlines()[0]
        print(json.dumps({'name': 'poppler', 'version': version, 'tasks': ['pageCount', 'extractText', 'render'], 'artifacts': binaries}))
        return
    request = json.load(sys.stdin)
    source, task = request['input'], request['task']
    if task == 'pageCount':
        result = {'pages': int(re.search(r'^Pages:\s+(\d+)', execute([binaries[0], source]), re.M)[1])}
    elif task == 'extractText':
        text = execute([binaries[1], '-enc', 'UTF-8', source, '-'])
        pages = text.split('\f')
        if pages[-1] == '':
            pages.pop()
        result = {'text': pages}
    else:
        output = pathlib.Path(request['output'])
        execute([binaries[2], '-f', str(request['page']), '-l', str(request['page']), '-r', str(request['dpi']),
                 '-singlefile', '-png', source, str(output.with_suffix(''))])
        result = {'saved': True}
    print(json.dumps(result))


if __name__ == '__main__':
    main()
