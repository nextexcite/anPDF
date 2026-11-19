"""Run the pinned official Forge artifact; fail rather than substitute a generator."""
import argparse,hashlib,json,pathlib,re,subprocess,tarfile,tempfile
ROOT=pathlib.Path(__file__).resolve().parents[1]
PIN=json.loads((ROOT/'build/forge-pin.json').read_text())
p=argparse.ArgumentParser();p.add_argument('--archive',required=True);p.add_argument('--out',default='generated');p.add_argument('--node',default='node');a=p.parse_args()
archive=pathlib.Path(a.archive).resolve()
assert hashlib.sha256(archive.read_bytes()).hexdigest()==PIN['artifactSha256'],'Forge artifact pin mismatch'
assert subprocess.check_output([a.node,'--version']).decode().startswith('v26.'),'qualification requires Node 26'
contract=ROOT/'contracts/v1/openapi.json';digest=hashlib.sha256(contract.read_bytes()).hexdigest()
out=ROOT/a.out;out.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(prefix='anpdf-forge-') as tmp:
    with tarfile.open(archive) as t:t.extractall(tmp,filter='data')
    launcher=pathlib.Path(tmp)/'package/bin/forge.cjs'
    for target in ['ts','capnweb','mcp']:
        subprocess.run([a.node,str(launcher),'generate','contracts/v1/openapi.json','--target',target,'--out',str(out/target)],cwd=ROOT,check=True)
        for file in (out/target).rglob('*.ts'):
            source=file.read_text();file.write_text(f'// Forge {PIN["revision"]}; contract SHA-256 {digest}\n'+source)
coverage=re.findall(r'    name: "([^"]+)"', (out/'mcp/tools.ts').read_text())
expected=sorted(v['post']['operationId'] for v in json.loads(contract.read_text())['paths'].values())
assert sorted(coverage)==expected,'generated operation coverage mismatch'
(out/'operation-index.json').write_text(json.dumps(expected,indent=2)+'\n')
files={str(f.relative_to(out)):hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted(out.rglob('*')) if f.is_file() and f.name!='provenance.json'}
(out/'provenance.json').write_text(json.dumps({'generator':PIN,'contractSha256':digest,'files':files},indent=2)+'\n')
print(json.dumps({'targets':3,'operations':len(expected),'files':len(files),'contractSha256':digest}))
