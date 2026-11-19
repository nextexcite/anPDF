"""Install official qpdf test binaries, preserving archive links, with published checksums."""
import hashlib,os,pathlib,platform,stat,sys,urllib.request,zipfile
system={'Linux':'linux','Darwin':'macos'}[platform.system()]
arch={'x86_64':'x86_64','arm64':'arm64','aarch64':'aarch64'}[platform.machine()]
if system=='macos' and arch=='aarch64':arch='arm64'
root=pathlib.Path(sys.argv[1]).resolve();root.mkdir(parents=True,exist_ok=True)
name=f'qpdf-12.4.2-bin-{system}-{arch}.zip';base='https://github.com/qpdf/qpdf/releases/download/v12.4.2/'
checks=urllib.request.urlopen(base+'qpdf-12.4.2.sha256',timeout=60).read().decode();data=urllib.request.urlopen(base+name,timeout=120).read();digest=hashlib.sha256(data).hexdigest()
assert any(line.split()[-1].lstrip('*')==name and line.split()[0]==digest for line in checks.splitlines()),'qpdf checksum mismatch'
import io
with zipfile.ZipFile(io.BytesIO(data)) as archive:
 for info in archive.infolist():
  path=root/info.filename;assert path.resolve().is_relative_to(root),'unsafe archive path'
  if info.is_dir():path.mkdir(parents=True,exist_ok=True);continue
  path.parent.mkdir(parents=True,exist_ok=True);content=archive.read(info)
  if stat.S_ISLNK(info.external_attr>>16):
   target=content.decode();assert '/' not in target;path.symlink_to(target)
  else:path.write_bytes(content);path.chmod((info.external_attr>>16)&0o777)
print(root/'bin/qpdf')
