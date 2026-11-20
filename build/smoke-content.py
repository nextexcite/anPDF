"""Verify bundled PDFium discovery and content operations without a library override."""
import json,os,pathlib,subprocess,sys

binary=pathlib.Path(sys.argv[1]).resolve()
root=pathlib.Path(sys.argv[2]).resolve()
env=dict(os.environ);env.pop('ANPDF_PDFIUM_LIB',None)
def run(*args):
    return json.loads(subprocess.check_output([str(binary),*args],env=env))
operations=run('operations')['operations']
for operation in ['extractText','renderPage']:
    assert next(o for o in operations if o['operationId']==operation)['engines']==['pdfium']
text=run('extract-text','--root',str(root),'--request-json',json.dumps({'input':{'path':'three.pdf'},'pages':[1]}))['result']
assert text['engine']=='pdfium' and 'PAGE-A' in text['pages'][0]['text']
image=run('render-page','--root',str(root),'--request-json',json.dumps({'input':{'path':'three.pdf'},'page':1,'dpi':144,'output':{'path':'pdfium-smoke.png'}}))['result']
assert image['engine']=='pdfium' and (image['width'],image['height'])==(600,400)
data=(root/'pdfium-smoke.png').read_bytes()
assert data[:8]==b'\x89PNG\r\n\x1a\n' and (int.from_bytes(data[16:20],'big'),int.from_bytes(data[20:24],'big'))==(600,400)
print('Bundled PDFium discovery, text, and PNG 600x400 passed (no ANPDF_PDFIUM_LIB)')
