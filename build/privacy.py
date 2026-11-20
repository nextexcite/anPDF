"""Generic public-bound content/history policy. Reports counts, never matched values."""
import hashlib,json,os,pathlib,re,subprocess,sys
run=lambda *a:subprocess.check_output(['git',*a])
commits=run('rev-list','--all').decode().splitlines();issues=0
baseline={}
baseline_path=pathlib.Path('build/privacy-baseline.json')
def unique_object(pairs):
    value=dict(pairs)
    if len(value)!=len(pairs):raise ValueError
    return value
if baseline_path.exists():
    try:
        document=json.loads(baseline_path.read_text(),object_pairs_hook=unique_object)
        if not isinstance(document,dict) or set(document)!={'identityExceptions'} or not isinstance(document['identityExceptions'],list):raise ValueError
        for entry in document['identityExceptions']:
            if not isinstance(entry,dict) or set(entry)!={'commit','identitySha256','reason'}:raise ValueError
            commit,digest,reason=entry['commit'],entry['identitySha256'],entry['reason']
            if not isinstance(commit,str) or not re.fullmatch('[0-9a-f]{40}',commit) or commit in baseline:raise ValueError
            if not isinstance(digest,str) or not re.fullmatch('[0-9a-f]{64}',digest):raise ValueError
            if not isinstance(reason,str) or not reason.strip():raise ValueError
            baseline[commit]=digest
    except (OSError,ValueError,TypeError,RecursionError):
        baseline={};issues+=1
for commit in commits:
    raw_identity=run('show','-s','--format=%an%n%ae%n%cn%n%ce',commit)
    identity=raw_identity.decode().splitlines()
    if identity!=['anPDF Contributors','contributors@anpdf.invalid']*2 and baseline.get(commit)!=hashlib.sha256(raw_identity).hexdigest():issues+=1
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
