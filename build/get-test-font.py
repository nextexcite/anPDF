"""Get an immutable OFL CJK test font; it is never part of the product runtime."""
import hashlib,pathlib,sys,urllib.request
root=pathlib.Path(sys.argv[1]);root.mkdir(parents=True,exist_ok=True)
base='https://raw.githubusercontent.com/google/fonts/2894aab31764f10f29c421bdfd2340d3b382d384/ofl/notosanssc/'
data=urllib.request.urlopen(base+'NotoSansSC%5Bwght%5D.ttf',timeout=120).read();assert hashlib.sha256(data).hexdigest()=='a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da'
(root/'NotoSansSC.ttf').write_bytes(data)
(root/'OFL.txt').write_bytes(urllib.request.urlopen(base+'OFL.txt',timeout=30).read())
