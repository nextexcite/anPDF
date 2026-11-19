"""Download the pinned official Forge preview package and verify its SHA-256."""
import hashlib,json,pathlib,sys,urllib.request
ROOT=pathlib.Path(__file__).resolve().parents[1]
PIN=json.loads((ROOT/'build/forge-pin.json').read_text())
out=pathlib.Path(sys.argv[1]).resolve();out.parent.mkdir(parents=True,exist_ok=True)
url=f'https://pkg.pr.new/cloudflare/forge/@cloudflare/forge@{PIN["revision"]}'
req=urllib.request.Request(url,headers={'User-Agent':'anpdf-build/0.1'})
data=urllib.request.urlopen(req,timeout=120).read()
assert hashlib.sha256(data).hexdigest()==PIN['artifactSha256'],'Forge archive checksum mismatch'
out.write_bytes(data);print(out)
