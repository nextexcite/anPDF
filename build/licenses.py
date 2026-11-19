"""Inventory exact native Rust license alternatives; copy upstream legal text only.
This does not replace artifact/license review for a release or include OCI OS packages.
"""
import argparse,hashlib,json,pathlib,re,subprocess,tomllib,urllib.request
ROOT=pathlib.Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser();p.add_argument('--target',default='x86_64-unknown-linux-gnu');a=p.parse_args()
m=json.loads(subprocess.check_output(['cargo','metadata','--locked','--format-version','1','--filter-platform',a.target],cwd=ROOT))
nodes={n['id']:n for n in m['resolve']['nodes']};reachable=set();stack=[m['resolve']['root']]
while stack:
 i=stack.pop()
 if i in reachable:continue
 reachable.add(i);stack.extend(d['pkg'] for d in nodes[i]['deps'])
lock=tomllib.loads((ROOT/'Cargo.lock').read_text());checks={(p['name'],p['version']):p.get('checksum') for p in lock['package']}
out=ROOT/'THIRD_PARTY_LICENSES';out.mkdir(exist_ok=True);components=[]
def add(data,original):
 digest=hashlib.sha256(data).hexdigest();name=f'THIRD_PARTY_LICENSES/{digest}.txt';(ROOT/name).write_bytes(data)
 return {'file':name,'originalName':original,'sha256':digest}
for c in sorted(m['packages'],key=lambda c:(c['name'],c['version'])):
 if c['id'] not in reachable or c['name']=='anpdf':continue
 root=pathlib.Path(c['manifest_path']).parent
 files=[f for f in root.iterdir() if f.is_file() and f.name.lower().startswith(('license','copying','notice','copyright','unlicense'))]
 notices=[add(f.read_bytes(),f.name) for f in sorted(files)]
 if not notices and c['name'] in ['uuid-simd','vsimd']:
  vcs=json.loads((root/'.cargo_vcs_info.json').read_text())['git']['sha1'];url=f'https://raw.githubusercontent.com/Nugine/simd/{vcs}/LICENSE';data=urllib.request.urlopen(url,timeout=30).read()
  assert hashlib.sha256(data).hexdigest()=='71674605ec4c087fe9eb534e3e4f9e26eb2e4aabcd76a29fd156c6a844d44b3d';notices=[add(data,'LICENSE')]
 expression=c['license'];selected=('Apache-2.0 AND Unicode-3.0' if 'AND Unicode-3.0' in expression else expression) if ' AND ' in expression else ('MIT' if re.search(r'(?<![A-Z-])MIT(?![A-Z-])',expression) else 'Apache-2.0' if 'Apache-2.0' in expression else expression)
 assert notices,'missing legal text: '+c['name']
 assert 'LGPL' not in selected and 'GPL' not in selected,'review required'
 components.append({'component':c['name'],'version':c['version'],'artifactSha256':checks[(c['name'],c['version'])],'source':f'https://crates.io/crates/{c["name"]}/{c["version"]}','spdxExpression':expression,'selectedAlternative':selected,'scope':'native Rust dependency graph; build and runtime','modifications':'none','notices':notices,'decision':'permissive alternative inventoried; release artifact review pending'})
(ROOT/'license-manifest.json').write_text(json.dumps({'target':a.target,'scope':'Rust native dependency inventory; OCI OS and external backend artifacts require separate SBOM review','components':components},indent=2)+'\n')
print(json.dumps({'nativeComponents':len(components),'uniqueNoticeFiles':len(list(out.glob('*.txt')))}))
