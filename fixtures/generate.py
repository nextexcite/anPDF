"""Deterministic, original synthetic PDFs; no external data, dates, or identity."""
import hashlib,json,pathlib,sys

def pdf(labels, extra='', objects=(), rotated=False):
    data=[b'<< /Type /Catalog /Pages 2 0 R '+extra.encode()+b' >>', b'']
    kids=[]
    for n,label in enumerate(labels):
        page=len(data)+1;stream=page+1;kids.append(f'{page} 0 R')
        content=f'BT /F1 24 Tf 40 120 Td ({label}) Tj ET'.encode()
        data.append(f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {300+n*10} 200] /Rotate {90 if rotated and n%2 else 0} /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> /Contents {stream} 0 R >>'.encode())
        data.append(b'<< /Length '+str(len(content)).encode()+b' >>\nstream\n'+content+b'\nendstream')
    data[1]=f'<< /Type /Pages /Count {len(labels)} /Kids [{" ".join(kids)}] >>'.encode()
    data.extend(o.encode() for o in objects)
    result=bytearray(b'%PDF-1.7\n%\xe2\xe3\xcf\xd3\n');offsets=[0]
    for i,o in enumerate(data,1):
        offsets.append(len(result));result.extend(f'{i} 0 obj\n'.encode()+o+b'\nendobj\n')
    xref=len(result);result.extend(f'xref\n0 {len(data)+1}\n0000000000 65535 f \n'.encode())
    for offset in offsets[1:]:result.extend(f'{offset:010d} 00000 n \n'.encode())
    result.extend(f'trailer\n<< /Size {len(data)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode());return bytes(result)

def generate(root):
    root=pathlib.Path(root);root.mkdir(parents=True,exist_ok=True)
    fixtures={
        'one.pdf':(pdf(['PAGE-ONE']),1),
        'three.pdf':(pdf(['PAGE-A','PAGE-B','PAGE-C']),3),
        'mixed.pdf':(pdf(['ROTATE-A','ROTATE-B'],rotated=True),2),
        'empty.pdf':(pdf([]),0),
        'form.pdf':(pdf(['FORM'], '/AcroForm 5 0 R', ['<< /Fields [6 0 R 7 0 R] /NeedAppearances true >>','<< /FT /Tx /T (repeat) /V <FEFF4E2D6587> >>','<< /FT /Tx /T (repeat) /V (duplicate) >>']),1),
        'xfa.pdf':(pdf(['XFA'], '/AcroForm 5 0 R',['<< /Fields [] /XFA (synthetic) >>']),1),
        'signature-marker.pdf':(pdf(['SIGNATURE-MARKER'], '/Perms << /DocMDP 5 0 R >>',['<< /Type /Sig /ByteRange [0 1 2 3] /Contents <0000> >>']),1),
        'action.pdf':(pdf(['ACTION'], '/OpenAction << /S /JavaScript /JS (synthetic inert test) >>'),1),
        'damaged.pdf':(b'%PDF-1.7\nnot a document\n',None),
        'warning.pdf':(pdf(['XREF-WARNING']).replace(b'startxref\n',b'startxref\n999999%'),1),
    }
    for name in ['@args.pdf','-flag.pdf','中文路径.pdf']:fixtures[name]=(pdf(['SAFE-PATH']),1)
    manifest=[]
    for name,(data,count) in fixtures.items():
        (root/name).write_bytes(data)
        manifest.append({'file':name,'sha256':hashlib.sha256(data).hexdigest(),'bytes':len(data),'pages':count,'license':'Apache-2.0','source':'original deterministic fixtures/generate.py'})
    (root/'manifest.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2)+'\n')
if __name__=='__main__':generate(sys.argv[1])
