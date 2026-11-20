"""Inventory exact native Rust license alternatives; copy upstream legal text only.
This does not replace artifact/license review for a release or include OCI OS packages.
"""
import argparse,hashlib,json,pathlib,re,subprocess,tomllib,urllib.request
ROOT=pathlib.Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser()
p.add_argument('--target',default='x86_64-unknown-linux-gnu')
p.add_argument('--features',default='',help='Cargo features to verify with --check')
p.add_argument('--check',action='store_true',help='Verify the selected build without rewriting the all-features inventory')
a=p.parse_args()
if a.features and not a.check:p.error('--features requires --check; regeneration always includes all features')
cmd=['cargo','metadata','--locked','--format-version','1','--filter-platform',a.target]
if not a.check:cmd.append('--all-features')
elif a.features:cmd.extend(['--features',a.features])
m=json.loads(subprocess.check_output(cmd,cwd=ROOT))
nodes={n['id']:n for n in m['resolve']['nodes']};reachable=set();stack=[m['resolve']['root']]
while stack:
 i=stack.pop()
 if i in reachable:continue
 reachable.add(i);stack.extend(d['pkg'] for d in nodes[i]['deps'])
lock=tomllib.loads((ROOT/'Cargo.lock').read_text());checks={(p['name'],p['version']):p.get('checksum') for p in lock['package']}
out=ROOT/'THIRD_PARTY_LICENSES';components=[]
if not a.check:out.mkdir(exist_ok=True)
upstream_notices={
 ('alloc-stdlib','0.3.0'):('dropbox/rust-alloc-no-stdlib','LICENSE','c0c56f26d9c051cac4d200c34c84e7ae9aaa853e01a982a1df08b09931e518ae'),
 ('qcms','0.3.0'):('FirefoxGraphics/qcms','COPYING','36d847ae882f6574ebc72f56a4f354e4f104fde4a584373496482e97d52d31bc'),
 ('taffy','0.14.0'):('DioxusLabs/taffy','LICENSE','f97daf1a0124413dccf399a4e6626b4b74acd05282f80b6d64ac82225650b77a'),
}
approved={'0BSD','Apache-2.0','BSD-2-Clause','BSD-3-Clause','BSL-1.0','CC0-1.0','ISC','MIT','MIT-0','Unicode-3.0','Unlicense','Zlib'}

def add(data,original):
 digest=hashlib.sha256(data).hexdigest();name=f'THIRD_PARTY_LICENSES/{digest}.txt'
 if not a.check:(ROOT/name).write_bytes(data)
 return {'file':name,'originalName':original,'sha256':digest}

def upstream_notice(root,repo,name,digest):
 cached=out/f'{digest}.txt'
 if cached.is_file():
  data=cached.read_bytes()
 else:
  vcs=json.loads((root/'.cargo_vcs_info.json').read_text())['git']['sha1']
  with urllib.request.urlopen(f'https://raw.githubusercontent.com/{repo}/{vcs}/{name}',timeout=30) as response:
   data=response.read()
 assert hashlib.sha256(data).hexdigest()==digest,'upstream notice checksum mismatch: '+repo
 return add(data,name)

def select_license(expression,label):
 assert expression,'missing license expression: '+label
 licenses=set(re.findall(r'[\w.-]+',expression.replace('Apache-2.0 WITH LLVM-exception','Apache-2.0')))-{'AND','OR'}
 assert licenses and licenses<=approved,'review required: '+label+' '+expression
 if 'AND Unicode-3.0' in expression:return 'Apache-2.0 AND Unicode-3.0'
 if expression=='(Apache-2.0 OR MIT) AND BSD-3-Clause':return 'MIT AND BSD-3-Clause'
 if expression=='BSD-3-Clause/MIT':return 'BSD-3-Clause'
 if ' AND ' in expression:return expression
 return next((license for license in ('MIT','Apache-2.0') if license in licenses),expression)

for c in sorted(m['packages'],key=lambda c:(c['name'],c['version'])):
 if c['id'] not in reachable or c['name']=='anpdf':continue
 key=c['name'],c['version']
 root=pathlib.Path(c['manifest_path']).parent
 files=[f for f in root.iterdir() if f.is_file() and f.name.lower().startswith(('license','copying','notice','copyright','unlicense'))]
 notices=[add(f.read_bytes(),f.name) for f in sorted(files)]
 if not notices and c['name'] in ['uuid-simd','vsimd']:
  notices=[upstream_notice(root,'Nugine/simd','LICENSE','71674605ec4c087fe9eb534e3e4f9e26eb2e4aabcd76a29fd156c6a844d44b3d')]
 if not notices and key in upstream_notices:notices=[upstream_notice(root,*upstream_notices[key])]
 expression=c['license'];selected=select_license(expression,' '.join(key))
 scope='native Rust dependency graph; build and runtime'
 decision='permissive alternative inventoried; release artifact review pending'
 if c['name']=='pdf_oxide':
  font_notice='src/fonts/assets/LICENSE-DejaVu'
  notices.append(add((root/font_notice).read_bytes(),font_notice))
  selected='Apache-2.0'
  scope='direct runtime dependency of the optional pdfoxide feature'
  decision='direct dependency and bundled DejaVu notice reviewed; release artifact review pending'
 assert notices,'missing legal text: '+c['name']
 components.append({'component':c['name'],'version':c['version'],'artifactSha256':checks[key],'source':f'https://crates.io/crates/{c["name"]}/{c["version"]}','spdxExpression':expression,'selectedAlternative':selected,'scope':scope,'modifications':'none','notices':notices,'decision':decision})
manifest_path=ROOT/'license-manifest.json'
pdfium_entry={'component':'pdfium-binaries','version':'chromium/8165','source':'https://github.com/bblanchon/pdfium-binaries/releases/tag/chromium/8165','scope':'dynamically loaded external backend bundled in standard releases','modifications':'none','decision':'upstream notices retained; review target-specific notices before redistribution'}
if a.check:
 manifest=json.loads(manifest_path.read_text())
 assert manifest['target']==a.target,'inventory target mismatch'
 recorded={(c['component'],c['version']):c for c in manifest['components']}
 assert len(recorded)==len(manifest['components']),'duplicate inventory components'
 external=recorded.get((pdfium_entry['component'],pdfium_entry['version']),{})
 assert external.get('notices') and all(external.get(k)==v for k,v in pdfium_entry.items()),'missing or stale PDFium inventory'
 for c in components:
  key=c['component'],c['version']
  assert recorded.get(key)==c,'missing or stale license entry: '+' '.join(key)
 for c in manifest['components']:
  for n in c['notices']:
   assert hashlib.sha256((ROOT/n['file']).read_bytes()).hexdigest()==n['sha256'],'notice checksum mismatch: '+n['file']
else:
 pdfium=ROOT/'build/work/pdfium'
 assert pdfium.is_dir(),'run build/get-pdfium.py build/work/pdfium before inventorying external notices'
 assert 'BUILD=8165' in (pdfium/'VERSION').read_text(),'unexpected PDFium release'
 notices=[add(f.read_bytes(),str(f.relative_to(pdfium))) for f in sorted([pdfium/'LICENSE',*(pdfium/'licenses').iterdir()]) if f.is_file()]
 manifest_path.write_text(json.dumps({'target':a.target,'scope':'Rust native dependency inventory (all features); OCI OS and external backend artifacts require separate SBOM review','components':components+[{**pdfium_entry,'notices':notices}]},indent=2)+'\n')
print(json.dumps({'nativeComponents':len(components),'uniqueNoticeFiles':len(list(out.glob('*.txt'))),'checked':a.check}))
