"""Generic public-bound content/history policy. Reports counts, never matched values."""
import hashlib,json,os,pathlib,re,subprocess,sys
run=lambda *a:subprocess.check_output(['git',*a])
commits=run('rev-list','--all').decode().splitlines();issues=0
for commit in commits:
    identity=run('show','-s','--format=%an%n%ae%n%cn%n%ce',commit).decode().splitlines()
    if identity!=['anPDF Contributors','contributors@anpdf.invalid']*2:issues+=1
objects=run('rev-list','--objects','--all').splitlines();blobs=set()
for row in objects:
    oid=row.split(b' ',1)[0].decode()
    if run('cat-file','-t',oid).strip()==b'blob':blobs.add(oid)
# Construct path patterns to avoid matching this checker itself.
patterns=[re.compile(re.escape('/'+'workspace/')),re.compile(re.escape('/'+'home/')+r'(?!runner/|build/)'),re.compile(re.escape('/'+'Users/'))]
email=re.compile(rb'[A-Za-z0-9_.+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}')
sanctioned=set()
manifest=pathlib.Path('license-manifest.json')
if manifest.exists():sanctioned={n['sha256'] for c in json.loads(manifest.read_text())['components'] for n in c.get('notices',[])}
for oid in blobs:
    raw=run('cat-file','blob',oid);text=raw.decode('utf8','replace')
    if any(p.search(text) for p in patterns):issues+=1
    if hashlib.sha256(raw).hexdigest() not in sanctioned and any(m.group()!=b'contributors@anpdf.invalid' for m in email.finditer(raw)):issues+=1
remotes=run('remote').decode().splitlines()
if '--ci' in sys.argv and os.environ.get('GITHUB_ACTIONS')=='true':
    expected='https://github.com/'+os.environ.get('GITHUB_REPOSITORY','')
    if remotes!=['origin'] or run('remote','get-url','origin').decode().strip().removesuffix('.git')!=expected:issues+=1
elif remotes:issues+=1
print(f'Privacy policy: {len(commits)} commits, {len(blobs)} unique blobs, {issues} findings')
sys.exit(bool(issues))
