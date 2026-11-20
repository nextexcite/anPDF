"""Fetch pinned dynamic PDFium binaries and upstream notices for local qualification."""
import hashlib,io,pathlib,platform,sys,tarfile,urllib.request

VERSION='chromium/8165'
PINS={
 'linux-x64':'ba7865e16c0271daab75256790be06c26272190afe6e2a70eae0bb39bb99089c',
 'linux-arm64':'338f5dcc907a20331812ae8ab01b16ff18966f4a247d790c8ad8fb49029d768b',
 'mac-arm64':'112e74d717f3e0310273b70f3c8780e8a61ff11b34834124465c8305c841d350',
}
system={'Linux':'linux','Darwin':'mac'}[platform.system()]
arch={'x86_64':'x64','aarch64':'arm64','arm64':'arm64'}[platform.machine()]
target=f'{system}-{arch}'
root=pathlib.Path(sys.argv[1]).resolve();root.mkdir(parents=True,exist_ok=True)
url=f'https://github.com/bblanchon/pdfium-binaries/releases/download/{VERSION.replace("/","%2F")}/pdfium-{target}.tgz'
data=urllib.request.urlopen(url,timeout=120).read()
assert hashlib.sha256(data).hexdigest()==PINS[target],'PDFium checksum mismatch'
with tarfile.open(fileobj=io.BytesIO(data),mode='r:gz') as archive:
 archive.extractall(root,filter='data')
library=root/'lib'/('libpdfium.dylib' if system=='mac' else 'libpdfium.so')
assert library.is_file(),'missing dynamic PDFium library'
print(library)
