"""Real qpdf cross-surface qualification. Requires built anpdf, qpdf 12.4.2, Poppler."""
import hashlib,importlib.util,json,os,pathlib,shutil,socket,subprocess,sys,tempfile,time,urllib.request,urllib.error,zlib
from pypdf import PdfWriter
from pypdf.annotations import Text, Link
from pypdf.generic import DecodedStreamObject, NameObject
ROOT=pathlib.Path(__file__).resolve().parents[1]
BIN=pathlib.Path(os.environ.get('ANPDF_BINARY',ROOT/'target/debug/anpdf')).resolve()
QPDF=pathlib.Path(os.environ.get('ANPDF_QPDF') or shutil.which('qpdf') or '/usr/bin/qpdf').resolve()
PDFOXIDE=os.environ.get('ANPDF_EXPECT_PDFOXIDE')=='1'
spec=importlib.util.spec_from_file_location('fixtures',ROOT/'fixtures/generate.py');gen=importlib.util.module_from_spec(spec);spec.loader.exec_module(gen)

def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def mcp_send(p,v):p.stdin.write(json.dumps(v)+'\n');p.stdin.flush()
def mcp_read(p):
    line=p.stdout.readline();assert line,'MCP closed unexpectedly';return json.loads(line)
def http(port,token,path,data=None,extra=None):
    headers={'Authorization':'Bearer '+token};headers.update(extra or {})
    if data is not None:headers['Content-Type']='application/json'
    r=urllib.request.Request(f'http://127.0.0.1:{port}'+path,data=None if data is None else json.dumps(data).encode(),headers=headers)
    try:
        with urllib.request.urlopen(r,timeout=15) as r:return r.status,json.load(r)
    except urllib.error.HTTPError as e:return e.code,json.load(e)

def build_pdf(objs):
    res=bytearray(b'%PDF-1.7\n');offs=[0]
    for i,o in enumerate(objs,1):offs.append(len(res));res.extend(f'{i} 0 obj\n'.encode()+o+b'\nendobj\n')
    xref=len(res);res.extend(f'xref\n0 {len(objs)+1}\n0000000000 65535 f \n'.encode())
    for o in offs[1:]:res.extend(f'{o:010d} 00000 n \n'.encode())
    res.extend(f'trailer\n<< /Size {len(objs)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode());return bytes(res)

def render_fixture(annotations=False, oversized=False, paint=True, page_entries=b''):
    box=b'/MediaBox [0 0 9000 100]' if oversized else b'/MediaBox [0 0 300 200] /CropBox [20 30 170 110] /Rotate 90'
    annots=b' /Annots [5 0 R]' if annotations else b''
    content=b'1 0 0 rg 40 40 50 30 re f' if paint else b''
    objs=[
        b'<< /Type /Catalog /Pages 2 0 R >>',
        b'<< /Type /Pages /Count 1 /Kids [3 0 R] >>',
        b'<< /Type /Page /Parent 2 0 R '+box+annots+page_entries+b' /Resources << >> /Contents 4 0 R >>',
        b'<< /Length '+str(len(content)).encode()+b' >>\nstream\n'+content+b'\nendstream',
    ]
    if annotations:
        appearance=b'0 0 1 rg 0 0 45 45 re f'
        objs.extend([
            b'<< /Type /Annot /Subtype /Square /Rect [90 50 135 95] /F 4 /AP << /N 6 0 R >> >>',
            b'<< /Type /XObject /Subtype /Form /BBox [0 0 45 45] /Resources << >> /Length '
            +str(len(appearance)).encode()+b' >>\nstream\n'+appearance+b'\nendstream',
        ])
    return build_pdf(objs)

def check_license_regeneration(root):
    root.mkdir()
    for name in ('build/licenses.py','build/get-pdfium.py','build.rs','Cargo.toml','Cargo.lock','src/main.rs','src/lib.rs'):
        dest=root/name;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/name,dest)
    shutil.copytree(ROOT/'THIRD_PARTY_LICENSES',root/'THIRD_PARTY_LICENSES')
    pdfium=ROOT/'build/work/pdfium'
    if pdfium.is_dir():
        target=root/'build/work/pdfium';shutil.copytree(pdfium/'licenses',target/'licenses')
        for name in ('LICENSE','VERSION'):shutil.copyfile(pdfium/name,target/name)
    else:
        subprocess.run([sys.executable,str(root/'build/get-pdfium.py'),str(root/'build/work/pdfium')],check=True)
    command=[sys.executable,str(root/'build/licenses.py')]
    subprocess.run(command,check=True)
    subprocess.run([*command,'--check'],check=True)
    subprocess.run([*command,'--check','--features','pdfoxide'],check=True)
    assert (root/'license-manifest.json').read_bytes()==(ROOT/'license-manifest.json').read_bytes()

def rich_pdf(path,title):
    w=PdfWriter()
    for _ in range(3):w.add_blank_page(width=300,height=200)
    w.add_metadata({'/Title':title,'/Author':'Synthetic','/Custom':'Synthetic custom metadata'})
    xmp=DecodedStreamObject();xmp.set_data(b'<?xpacket begin=""?><synthetic>metadata</synthetic><?xpacket end="w"?>')
    xmp[NameObject('/Type')]=NameObject('/Metadata');xmp[NameObject('/Subtype')]=NameObject('/XML')
    w.root_object[NameObject('/Metadata')]=w._add_object(xmp)
    w.add_outline_item('First page',0);w.add_outline_item('Last page',2)
    w.add_attachment('sample.txt',b'Synthetic attachment')
    w.add_annotation(0,Text(rect=(10,10,30,30),text='Synthetic note'))
    w.add_annotation(2,Link(rect=(10,10,30,30),url='https://example.invalid/'))
    with path.open('wb') as f:w.write(f)

def qpdf_features(path):
    v=json.loads(subprocess.check_output([str(QPDF),'--json=2','--json-stream-data=none',str(path)]));objs=v['qpdf'][1]
    def resolve(value):return objs['obj:'+value]['value'] if isinstance(value,str) else value
    trailer=objs['trailer']['value'];catalog=resolve(trailer['/Root'])
    xmp=catalog.get('/Metadata')
    if xmp:xmp=subprocess.check_output([str(QPDF),'--show-object='+xmp.removesuffix(' R').replace(' ',','),'--filtered-stream-data',str(path)])
    attachments={key:subprocess.check_output([str(QPDF),'--show-attachment='+key,str(path)]) for key in v['attachments']}
    annots=[len(resolve(resolve(page['object']).get('/Annots',[]))) for page in v['pages']]
    return {'metadata':resolve(trailer.get('/Info',{})),'xmp':xmp,'outlines':[(o['title'],o['destpageposfrom1']) for o in v['outlines']],'attachments':attachments,'annotations':annots}

def run():
 with tempfile.TemporaryDirectory(prefix='anpdf-tests-') as t:
    base=pathlib.Path(t);roots={s:base/s for s in ['cli','http','mcp']}
    if PDFOXIDE:check_license_regeneration(base/'license-inventory')
    geometry_objects=[b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Count 3 /Kids [3 0 R 5 0 R 7 0 R] >>']
    for box,rotation,x,y in [('[500 600 800 800]',0,540,720),('[0 0 300 200]',90,240,120),('[0 0 300 200]',270,240,120)]:
        stream=f'BT /F1 12 Tf {x} {y} Td (PAGE-A) Tj ET'.encode();number=len(geometry_objects)+1
        geometry_objects.append(f'<< /Type /Page /Parent 2 0 R /MediaBox {box} /CropBox {box} /Rotate {rotation} /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> /Contents {number+1} 0 R >>'.encode())
        geometry_objects.append(b'<< /Length '+str(len(stream)).encode()+b' >>\nstream\n'+stream+b'\nendstream')
    bomb_data=b' '*(40*1024*1024)
    for _ in range(3):bomb_data=zlib.compress(bomb_data,9)
    bomb_pdf=build_pdf([
        b'<< /Type /Catalog /Pages 2 0 R >>',
        b'<< /Type /Pages /Count 1 /Kids [3 0 R] >>',
        b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << >> /Contents 4 0 R >>',
        b'<< /Filter [/FlateDecode /FlateDecode /FlateDecode] /Length '+str(len(bomb_data)).encode()+b' >>\nstream\n'+bomb_data+b'\nendstream',
    ])
    bad_filter_pdf=build_pdf([
        b'<< /Type /Catalog /Pages 2 0 R >>',
        b'<< /Type /Pages /Count 1 /Kids [3 0 R] >>',
        b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << >> /Contents 4 0 R >>',
        b'<< /Filter /UnknownDecode /Length 4 >>\nstream\nabcd\nendstream',
    ])
    cyclic_pdf=build_pdf([
        b'<< /Type /Catalog /Pages 2 0 R >>',
        b'<< /Type /Pages /Count 1 /Kids [2 0 R] /Parent 2 0 R >>',
    ])
    for root in roots.values():
        gen.generate(root)
        if PDFOXIDE:
            (root/'crop.pdf').write_bytes(render_fixture())
            (root/'annotation.pdf').write_bytes(render_fixture(annotations=True))
            (root/'oxide-blank.pdf').write_bytes(render_fixture(paint=False))
            (root/'oversized.pdf').write_bytes(render_fixture(oversized=True))
            (root/'custom-unit.pdf').write_bytes(render_fixture(page_entries=b' /UserUnit 2.5'))
            (root/'default-unit.pdf').write_bytes(render_fixture(page_entries=b' /UserUnit 1'))
            (root/'null-unit.pdf').write_bytes(render_fixture(page_entries=b' /UserUnit null'))
        rich_pdf(root/'primary.pdf','Primary');rich_pdf(root/'secondary.pdf','Secondary')
        large=PdfWriter();large.add_blank_page(width=300,height=200)
        large.add_attachment('large.txt',b'x'*(33*1024*1024))
        with (root/'large-attachment.pdf').open('wb') as f:large.write(f)
        (root/'text-geometry.pdf').write_bytes(build_pdf(geometry_objects))
        (root/'blank.pdf').write_bytes(gen.pdf(['']))
        (root/'annotated.pdf').write_bytes(build_pdf([
            b'<< /Type /Catalog /Pages 2 0 R /AcroForm 7 0 R >>',
            b'<< /Type /Pages /Count 1 /Kids [3 0 R] >>',
            b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << >> /Contents 4 0 R /Annots [5 0 R 6 0 R] >>',
            b'<< /Length 0 >>\nstream\n\nendstream',
            b'<< /Type /Annot /Subtype /Square /Rect [0 0 300 200] /F 4 /AP << /N 8 0 R >> >>',
            b'<< /Type /Annot /Subtype /Widget /FT /Tx /T (synthetic) /Rect [0 0 300 200] /F 4 /AP << /N 8 0 R >> >>',
            b'<< /Fields [6 0 R] >>',
            b'<< /Type /XObject /Subtype /Form /BBox [0 0 300 200] /Resources << >> /Length 26 >>\nstream\n1 0 0 rg 0 0 300 200 re f\n\nendstream',
        ]))
        (root/'huge.pdf').write_bytes(gen.pdf(['HUGE']).replace(b'300 200',b'9000 9000'))
        if os.environ.get('ANPDF_TEST_FONT'):
            from fontTools.ttLib import TTFont
            from fontTools.varLib.instancer import instantiateVariableFont
            from fontTools import subset
            from reportlab.pdfbase import pdfmetrics
            from reportlab.pdfbase.ttfonts import TTFont as RLFont
            from reportlab.pdfgen import canvas
            font=instantiateVariableFont(TTFont(os.environ['ANPDF_TEST_FONT']),{'wght':400},inplace=True)
            sub=subset.Subsetter();sub.populate(text='中文页面测试 ABC 123');sub.subset(font);font.save(root/'test-font.ttf')
            pdfmetrics.registerFont(RLFont('SyntheticCJK',str(root/'test-font.ttf')))
            c=canvas.Canvas(str(root/'provider-cjk.pdf'),pagesize=(360,180),invariant=1);c.setAuthor('');c.setFont('SyntheticCJK',24);c.drawString(24,90,'中文页面测试 ABC 123');c.showPage();c.save()
        subprocess.run([str(QPDF),'--encrypt','','synthetic-only','256','--',str(root/'one.pdf'),str(root/'encrypted.pdf')],check=True,capture_output=True)
        (root/'truncated.pdf').write_bytes(b'%PDF-1.7\n1 0 obj\n<< /Type /Catalog')
        (root/'bad-header.pdf').write_bytes(b'NOT-A-PDF-HEADER\x00\xff')
        (root/'bad-filter.pdf').write_bytes(bad_filter_pdf)
        (root/'cyclic.pdf').write_bytes(cyclic_pdf)
        (root/'flate-bomb.pdf').write_bytes(bomb_pdf)
        if os.environ.get('ANPDF_EDGE_FIXTURES'):
            for name in ['chinese.pdf','signed.pdf','tampered.pdf']:shutil.copyfile(pathlib.Path(os.environ['ANPDF_EDGE_FIXTURES'])/name,root/name)
        (root/'alias.pdf').hardlink_to(root/'one.pdf');(root/'symlink.pdf').symlink_to('one.pdf')
    token='synthetic-loopback-test-credential-only';token_file=base/'http-credential';token_file.write_text(token);token_file.chmod(0o600)
    with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
    server=subprocess.Popen([str(BIN),'serve','--root',str(roots['http']),'--qpdf',str(QPDF),'--port',str(port),'--token-file',str(token_file)],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    mcp=subprocess.Popen([str(BIN),'mcp','--root',str(roots['mcp']),'--qpdf',str(QPDF)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    try:
        for _ in range(100):
            try:
                status,discovery=http(port,token,'/v1/operations');assert status==200;break
            except (OSError,urllib.error.URLError):time.sleep(.05)
        else:raise AssertionError('HTTP did not start')
        mcp_send(mcp,{'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'synthetic-test','version':'1'}}});assert mcp_read(mcp)['result']['protocolVersion']=='2025-11-25'
        mcp_send(mcp,{'jsonrpc':'2.0','method':'notifications/initialized'})
        mcp_send(mcp,{'jsonrpc':'2.0','id':2,'method':'tools/list'});tools=mcp_read(mcp)['result']['tools']
        assert {t['name'] for t in tools}=={o['operationId'] for o in discovery['operations']}
        for tool in tools:
            op=next(o for o in discovery['operations'] if o['operationId']==tool['name']);assert tool['inputSchema']==op['inputSchema'];assert tool['outputSchema']==op['outputSchema']
        pdfium=any(o['operationId']=='extractText' and 'pdfium' in o['engines'] for o in discovery['operations'])
        engines=(['pdfium'] if pdfium else [])+(['pdfoxide'] if PDFOXIDE else [])
        preferred=next(iter(engines),None)
        for name,read_only in [('extractText',True),('renderPage',False)]:
            op=next(o for o in discovery['operations'] if o['operationId']==name)
            assert op['engines']==engines
            assert op['status'].startswith('implemented' if engines else 'unavailable')
            tool=next(t for t in tools if t['name']==name);assert tool['annotations']['readOnlyHint']==read_only
        cli=json.loads(subprocess.check_output([str(BIN),'operations']));assert cli==discovery
        assert http(port,'invalid','/v1/operations')[0]==401
        assert http(port,token,'/v1/operations',extra={'Origin':'https://example.invalid'})[0]==403
        assert http(port,token,'/v1/operations',extra={'Host':'example.invalid'})[0]==403
        assert http(port,token,'/v1/openapi.json')[1]==json.loads((ROOT/'contracts/v1/openapi.json').read_text())
        corpus=[
          ('extract-text',{'input':{'path':'three.pdf'},'pages':[1,3]},None if engines else 'BACKEND_UNAVAILABLE'),
          ('extract-text',{'input':{'path':'three.pdf'},'pages':[2,1]},'INVALID_REQUEST'),
          ('extract-text',{'input':{'path':'three.pdf'},'pages':[1],'engine':'../engine'},'INVALID_REQUEST'),
          ('render-page',{'input':{'path':'three.pdf'},'page':1,'output':{'path':'page.png'}},None if engines else 'BACKEND_UNAVAILABLE'),
          ('render-page',{'input':{'path':'three.pdf'},'page':0,'output':{'path':'page.png'}},'INVALID_REQUEST'),
          ('render-page',{'input':{'path':'three.pdf'},'page':1,'dpi':301,'output':{'path':'page.png'}},'INVALID_REQUEST'),
          ('inspect',{'input':{'path':'three.pdf'}},None),
          ('inspect',{'input':{'path':'empty.pdf'}},'INVALID_PDF'),
          ('inspect',{'input':{'path':'mixed.pdf'}},None),
          ('inspect',{'input':{'path':'form.pdf'}},None),
          ('inspect',{'input':{'path':'xfa.pdf'}},None),
          ('inspect',{'input':{'path':'signature-marker.pdf'}},None),
          ('inspect',{'input':{'path':'action.pdf'}},None),
          ('inspect',{'input':{'path':'中文路径.pdf'}},None),
          ('inspect',{'input':{'path':'@args.pdf'}},None),
          ('inspect',{'input':{'path':'-flag.pdf'}},None),
          ('inspect',{'input':{'path':'warning.pdf'}},None),
          ('merge',{'inputs':[{'path':'three.pdf'},{'path':'one.pdf'}],'output':{'path':'merged.pdf'}},None),
          ('extract',{'input':{'path':'three.pdf'},'pages':[1,3],'output':{'path':'subset.pdf'}},None),
          ('split',{'input':{'path':'three.pdf'},'parts':[{'pages':[1,2],'output':{'path':'split-a.pdf'}},{'pages':[3],'output':{'path':'split-b.pdf'}}]},None),
          ('split',{'input':{'path':'three.pdf'},'parts':[{'pages':[1],'output':{'path':'split-a.pdf','replaceExisting':True}},{'pages':[2,3],'output':{'path':'split-c.pdf'}}]},None),
          ('split',{'input':{'path':'three.pdf'},'parts':[{'pages':[1],'output':{'path':'split-rollback.pdf'}},{'pages':[2],'output':{'path':'split-b.pdf'}}]},'OUTPUT_EXISTS'),
          ('split',{'input':{'path':'three.pdf'},'parts':[{'pages':[1],'output':{'path':'split-dup.pdf','replaceExisting':True}},{'pages':[2],'output':{'path':'split-dup.pdf','replaceExisting':True}}]},'INVALID_REQUEST'),
          ('split',{'input':{'path':'three.pdf'},'parts':[{'pages':[2,1],'output':{'path':'bad1.pdf'}},{'pages':[3],'output':{'path':'bad2.pdf'}}]},'INVALID_REQUEST'),
          ('split',{'input':{'path':'three.pdf'},'parts':[{'pages':[1],'output':{'path':'bad1.pdf'}},{'pages':[4],'output':{'path':'bad2.pdf'}}]},'INVALID_REQUEST'),
          ('split',{'input':{'path':'one.pdf'},'parts':[{'pages':[1],'output':{'path':'alias.pdf','replaceExisting':True}},{'pages':[1],'output':{'path':'bad2.pdf'}}]},'ACCESS_DENIED'),
          ('split',{'input':{'path':'form.pdf'},'parts':[{'pages':[1],'output':{'path':'bad1.pdf'}},{'pages':[1],'output':{'path':'bad2.pdf'}}]},'FORM_UNSUPPORTED'),
          ('split',{'input':{'path':'signature-marker.pdf'},'parts':[{'pages':[1],'output':{'path':'bad1.pdf'}},{'pages':[1],'output':{'path':'bad2.pdf'}}]},'SIGNED_UNSUPPORTED'),
          ('remove-pages',{'input':{'path':'three.pdf'},'pages':[2],'output':{'path':'trimmed.pdf'}},None),
          ('remove-pages',{'input':{'path':'three.pdf'},'pages':[1,3],'output':{'path':'trimmed.pdf','replaceExisting':True}},None),
          ('remove-pages',{'input':{'path':'three.pdf'},'pages':[1,2,3],'output':{'path':'empty-out.pdf'}},'INVALID_REQUEST'),
          ('remove-pages',{'input':{'path':'three.pdf'},'pages':[4],'output':{'path':'bad.pdf'}},'INVALID_REQUEST'),
          ('remove-pages',{'input':{'path':'form.pdf'},'pages':[1],'output':{'path':'bad.pdf'}},'FORM_UNSUPPORTED'),
          ('remove-pages',{'input':{'path':'signature-marker.pdf'},'pages':[1],'output':{'path':'bad.pdf'}},'SIGNED_UNSUPPORTED'),
          ('compress',{'input':{'path':'three.pdf'},'output':{'path':'compressed.pdf'}},None),
          ('compress',{'input':{'path':'three.pdf'},'output':{'path':'compressed-lossy.pdf'},'mode':'lossy','jpegQuality':75},None),
          ('compress',{'input':{'path':'three.pdf'},'output':{'path':'bad-compress.pdf'},'mode':'lossless','jpegQuality':75},'INVALID_REQUEST'),
          ('compress',{'input':{'path':'form.pdf'},'output':{'path':'bad.pdf'}},'FORM_UNSUPPORTED'),
          ('compress',{'input':{'path':'signature-marker.pdf'},'output':{'path':'bad.pdf'}},'SIGNED_UNSUPPORTED'),
          ('merge',{'inputs':[{'path':'three.pdf'},{'path':'one.pdf'}],'output':{'path':'merged.pdf'}},'OUTPUT_EXISTS'),
          ('extract',{'input':{'path':'three.pdf'},'pages':[2],'output':{'path':'subset.pdf','replaceExisting':True}},None),
          ('inspect',{'input':{'path':'damaged.pdf'}},'INVALID_PDF'),
          ('inspect',{'input':{'path':'truncated.pdf'}},'INVALID_PDF'),
          ('inspect',{'input':{'path':'bad-header.pdf'}},'INVALID_PDF'),
          ('inspect',{'input':{'path':'bad-filter.pdf'}},'INVALID_PDF'),
          ('inspect',{'input':{'path':'cyclic.pdf'}},'INVALID_PDF'),
          ('inspect',{'input':{'path':'flate-bomb.pdf'}},None),
          ('compress',{'input':{'path':'flate-bomb.pdf'},'output':{'path':'bomb-recompressed.pdf'}},None),
          ('inspect',{'input':{'path':'encrypted.pdf'}},'ENCRYPTED_UNSUPPORTED'),
          ('inspect',{'input':{'path':'missing.pdf'}},'NOT_FOUND'),
          ('inspect',{'input':{'path':'../one.pdf'}},'ACCESS_DENIED'),
          ('inspect',{'input':{'path':'symlink.pdf'}},'ACCESS_DENIED'),
          ('inspect',{'input':{'path':'https://example.invalid/a.pdf'}},'ACCESS_DENIED'),
          ('inspect',{'input':{'path':'one.pdf'},'extra':True},'INVALID_REQUEST'),
          ('extract',{'input':{'path':'three.pdf'},'pages':[2,1],'output':{'path':'bad.pdf'}},'INVALID_REQUEST'),
          ('extract',{'input':{'path':'three.pdf'},'pages':[1,1],'output':{'path':'bad.pdf'}},'INVALID_REQUEST'),
          ('extract',{'input':{'path':'three.pdf'},'pages':[4],'output':{'path':'bad.pdf'}},'INVALID_REQUEST'),
          ('extract',{'input':{'path':'one.pdf'},'pages':[1],'output':{'path':'alias.pdf','replaceExisting':True}},'ACCESS_DENIED'),
          ('extract',{'input':{'path':'form.pdf'},'pages':[1],'output':{'path':'blocked.pdf'}},'FORM_UNSUPPORTED'),
          ('extract',{'input':{'path':'xfa.pdf'},'pages':[1],'output':{'path':'blocked.pdf'}},'FORM_UNSUPPORTED'),
          ('extract',{'input':{'path':'signature-marker.pdf'},'pages':[1],'output':{'path':'blocked.pdf'}},'SIGNED_UNSUPPORTED'),
        ]
        if PDFOXIDE:
            oxide_cases=[
                ('extract-text',{'input':{'path':'mixed.pdf'},'pages':[2],'engine':'pdfoxide'},None),
                ('extract-text',{'input':{'path':'oxide-blank.pdf'},'pages':[1]},None),
                ('extract-text',{'input':{'path':'encrypted.pdf'},'pages':[1]},'ENCRYPTED_UNSUPPORTED'),
                ('extract-text',{'input':{'path':'truncated.pdf'},'pages':[1]},'INVALID_PDF'),
                ('extract-text',{'input':{'path':'missing.pdf'},'pages':[1],'engine':'not-installed'},'BACKEND_UNAVAILABLE'),
                ('render-page',{'input':{'path':'oversized.pdf'},'page':1,'dpi':72,'output':{'path':'oversized.png'}},'LIMIT_EXCEEDED'),
                ('render-page',{'input':{'path':'crop.pdf'},'page':1,'dpi':72,'engine':'pdfoxide','output':{'path':'crop.png'}},None),
                ('render-page',{'input':{'path':'custom-unit.pdf'},'page':1,'dpi':72,'output':{'path':'custom-unit.png'}},'UNSUPPORTED_OPERATION'),
                ('render-page',{'input':{'path':'default-unit.pdf'},'page':1,'dpi':72,'output':{'path':'default-unit.png'}},None),
                ('render-page',{'input':{'path':'null-unit.pdf'},'page':1,'dpi':72,'output':{'path':'null-unit.png'}},None),
                ('render-page',{'input':{'path':'annotation.pdf'},'page':1,'dpi':72,'output':{'path':'oxide-annotation.png'}},None),
                ('render-page',{'input':{'path':'oxide-blank.pdf'},'page':1,'dpi':72,'output':{'path':'blank.png'}},None),
                ('render-page',{'input':{'path':'crop.pdf'},'page':1,'dpi':72,'output':{'path':'crop.png'}},'OUTPUT_EXISTS'),
            ]
            for _,request,_ in oxide_cases:request.setdefault('engine','pdfoxide')
            corpus.extend(oxide_cases)
        if os.environ.get('ANPDF_EDGE_FIXTURES'):
            corpus.extend([
              ('inspect',{'input':{'path':'chinese.pdf'}},None),
              ('extract',{'input':{'path':'chinese.pdf'},'pages':[1],'output':{'path':'cjk-output.pdf'}},None),
              ('inspect',{'input':{'path':'signed.pdf'}},None),
              ('inspect',{'input':{'path':'tampered.pdf'}},None),
              ('extract',{'input':{'path':'signed.pdf'},'pages':[1],'output':{'path':'blocked.pdf'}},'SIGNED_UNSUPPORTED'),
              ('extract',{'input':{'path':'tampered.pdf'},'pages':[1],'output':{'path':'blocked.pdf'}},'SIGNED_UNSUPPORTED')])
        corpus.extend([
          ('inspect',{'input':{'path':'large-attachment.pdf'}},None),
          ('extract',{'input':{'path':'large-attachment.pdf'},'pages':[1],'output':{'path':'large-extract.pdf'}},None),
          ('extract',{'input':{'path':'large-attachment.pdf'},'pages':[1],'output':{'path':'large-unmet.pdf'},'preserve':{'attachments':True}},'PRESERVATION_UNMET'),
          ('extract',{'input':{'path':'primary.pdf'},'pages':[1,3],'output':{'path':'rich-extract.pdf'},'preserve':{'metadata':True,'bookmarks':True,'annotations':True,'attachments':True}},None),
          ('remove-pages',{'input':{'path':'primary.pdf'},'pages':[1,3],'output':{'path':'rich-remove.pdf'}},None),
          ('merge',{'inputs':[{'path':'primary.pdf'},{'path':'secondary.pdf'}],'output':{'path':'rich-merge.pdf'}},None),
          ('merge',{'inputs':[{'path':'primary.pdf'},{'path':'one.pdf'}],'output':{'path':'rich-primary.pdf'},'preserve':{'metadata':True,'bookmarks':True,'annotations':True,'attachments':True}},None),
          ('merge',{'inputs':[{'path':'one.pdf'},{'path':'primary.pdf'}],'output':{'path':'rich-secondary.pdf'}},None),
          ('merge',{'inputs':[{'path':'primary.pdf'},{'path':'secondary.pdf'}],'output':{'path':'unmet.pdf'},'preserve':{'metadata':True}},'PRESERVATION_UNMET'),
          ('remove-pages',{'input':{'path':'primary.pdf'},'pages':[1,3],'output':{'path':'unmet.pdf'},'preserve':{'bookmarks':True}},'PRESERVATION_UNMET'),
          ('split',{'input':{'path':'primary.pdf'},'parts':[{'pages':[1,2,3],'output':{'path':'unmet-split-a.pdf'}},{'pages':[2],'output':{'path':'unmet-split-b.pdf'}}],'preserve':{'bookmarks':True}},'PRESERVATION_UNMET'),
          ('split',{'input':{'path':'primary.pdf'},'parts':[{'pages':[1,3],'output':{'path':'rich-split-a.pdf'}},{'pages':[2],'output':{'path':'rich-split-b.pdf'}}]},None),
          ('compress',{'input':{'path':'compressed.pdf'},'output':{'path':'fallback.pdf'}},None),
          ('compress',{'input':{'path':'compressed.pdf'},'output':{'path':'larger.pdf'},'allowLargerOutput':True},None),
        ])
        if pdfium:
            corpus.extend([
              ('extract-text',{'input':{'path':'text-geometry.pdf'},'pages':[1,2,3]},None),
              ('extract-text',{'input':{'path':'blank.pdf'},'pages':[1],'engine':'pdfium'},None),
              ('extract-text',{'input':{'path':'three.pdf'},'pages':[4]},'INVALID_REQUEST'),
              ('render-page',{'input':{'path':'three.pdf'},'page':4,'output':{'path':'missing.png'}},'INVALID_REQUEST'),
              ('render-page',{'input':{'path':'huge.pdf'},'page':1,'dpi':300,'output':{'path':'huge.png'}},'LIMIT_EXCEEDED'),
              ('render-page',{'input':{'path':'three.pdf'},'page':1,'dpi':72,'engine':'pdfium','output':{'path':'small.png'}},None),
              ('render-page',{'input':{'path':'blank.pdf'},'page':1,'dpi':72,'output':{'path':'blank-render.png'}},None),
              ('render-page',{'input':{'path':'annotated.pdf'},'page':1,'dpi':72,'output':{'path':'annotation.png'}},None),
              ('extract-text',{'input':{'path':'three.pdf'},'pages':[1],'engine':'absent'},'BACKEND_UNAVAILABLE'),
            ])
            if os.environ.get('ANPDF_TEST_FONT'):
                corpus.append(('extract-text',{'input':{'path':'provider-cjk.pdf'},'pages':[1]},None))
        originals={surface:{f.name:digest(f) for f in root.glob('*.pdf')} for surface,root in roots.items()}
        responses={}
        for n,(command,request,error) in enumerate(corpus,10):
            op={'inspect':'inspectPdf','merge':'mergePdf','extract':'extractPdf','split':'splitPdf','remove-pages':'removePages','compress':'compressPdf','extract-text':'extractText','render-page':'renderPage'}[command]
            cli=subprocess.run([str(BIN),command,'--root',str(roots['cli']),'--qpdf',str(QPDF),'--request-json',json.dumps(request)],capture_output=True,text=True)
            assert bool(cli.returncode)==bool(error),(n,command,cli.returncode,cli.stderr,cli.stdout);a=json.loads(cli.stderr if error else cli.stdout)
            assert not (cli.stdout if error else cli.stderr),(n,'framing')
            status,b=http(port,token,'/v1/'+command,request)
            mcp_send(mcp,{'jsonrpc':'2.0','id':n,'method':'tools/call','params':{'name':op,'arguments':request}})
            result=mcp_read(mcp)['result'];c=json.loads(result['content'][0]['text'])
            assert result['isError']==bool(error)
            if not error:assert result['structuredContent']==c
            assert a==b==c,(n,'surface mismatch',a,b,c)
            if error:assert a['error']['code']==error,(n,a)
            else:assert status==200
            if not error and command in ('extract-text','render-page'):
                assert a['result']['engine']==request.get('engine',preferred)
            if not error and command=='extract-text' and a['result']['engine']=='pdfoxide':
                expected={'three.pdf':['PAGE-A','PAGE-C'],'mixed.pdf':['ROTATE-B'],'oxide-blank.pdf':['']}[request['input']['path']]
                pages=a['result']['pages'];assert [p['page'] for p in pages]==request['pages']
                for page,label in zip(pages,expected):
                    assert page['text'].strip()==label,(label,page)
                    if not label:
                        assert page['spans']==[]
                    else:
                        spans=page['spans'];assert ''.join(s['text'] for s in spans)==label,(label,spans)
                        bounds=spans[0]['bounds'];assert 39 <= bounds['left'] <= 41 and 100 <= bounds['bottom'] <= 125,bounds
            if not error and command=='render-page' and a['result']['engine']=='pdfoxide':
                result=a['result'];name=request['output']['path']
                assert result['engine']=='pdfoxide' and result['mediaType']=='image/png'
                assert (result['width'],result['height'])==((600,400) if name=='page.png' else (80,150))
                assert result['dpi']==request.get('dpi',144)
                for root in roots.values():
                    png=root/name
                    assert png.read_bytes()[:8]==b'\x89PNG\r\n\x1a\n'
                    assert png.stat().st_size==result['bytes'] and digest(png)==result['sha256']
            if error=='BACKEND_UNAVAILABLE':assert status==503 and cli.returncode==8
            if not error and 'output' in request:responses[request['output']['path']]=a
            if not error and command=='extract-text' and a['result']['engine']=='pdfium':
                pages=a['result']['pages'];assert [p['page'] for p in pages]==request['pages']
                if request['input']['path']=='blank.pdf':assert pages[0]['text']=='' and pages[0]['spans']==[]
                else:
                    expected='中文页面测试' if request['input']['path']=='provider-cjk.pdf' else 'PAGE-A'
                    assert expected in pages[0]['text'],pages
                    assert expected in ''.join(s['text'] for s in pages[0]['spans']),pages
                    for p in pages:
                        assert all(s['text'] in p['text'] for s in p['spans']),p
                        for span in p['spans']:
                            b=span['bounds'];offset=500 if request['input']['path']=='text-geometry.pdf' and p['page']==1 else 0
                            assert offset<=b['left']<b['right']<=offset+360 and 0<=b['bottom']<b['top']<=(800 if offset else 200)
            if not error and command=='render-page' and a['result']['engine']=='pdfium':
                expected=(300,200) if request.get('dpi')==72 else (600,400)
                for root in roots.values():
                    data=(root/request['output']['path']).read_bytes();assert data[:8]==b'\x89PNG\r\n\x1a\n'
                    assert (int.from_bytes(data[16:20],'big'),int.from_bytes(data[20:24],'big'))==expected
            for surface,root in roots.items():
                for name,h in originals[surface].items():assert digest(root/name)==h,(n,'input changed',name)
        for root in roots.values():
            assert (root/'page.png').exists()==bool(engines)
            if PDFOXIDE:
                assert not (root/'oversized.png').exists()
                assert not (root/'custom-unit.png').exists()
                assert digest(root/'default-unit.png')==digest(root/'crop.png')
                assert digest(root/'null-unit.png')==digest(root/'crop.png')
                assert digest(root/'crop.png')==digest(root/'oxide-annotation.png')!=digest(root/'blank.png')
        assert all(not (root/name).exists() for root in roots.values() for name in ['missing.png','huge.png'])
        if pdfium:
            for root in roots.values():assert (root/'annotation.png').read_bytes()==(root/'blank-render.png').read_bytes()
            req=json.dumps({'input':{'path':'three.pdf'},'pages':[1],'engine':'pdfium'})
            r=subprocess.run([str(BIN),'extract-text','--root',str(roots['cli']),'--pdfium-lib',str(base/'missing-library'),'--request-json',req],capture_output=True,text=True)
            assert r.returncode==8 and json.loads(r.stderr)['error']['code']=='BACKEND_UNAVAILABLE' and not r.stdout
            for command in ['operations','completions']:
                args=[str(BIN),command]+(['bash'] if command=='completions' else [])
                assert 'pdfium-worker' not in subprocess.check_output(args,text=True)
        # Independent semantic page order, not just matching hashes.
        text=subprocess.check_output(['pdftotext',str(roots['cli']/'merged.pdf'),'-']).decode()
        assert [text.index(s) for s in ['PAGE-A','PAGE-B','PAGE-C','PAGE-ONE']]==sorted(text.index(s) for s in ['PAGE-A','PAGE-B','PAGE-C','PAGE-ONE'])
        text=subprocess.check_output(['pdftotext',str(roots['cli']/'subset.pdf'),'-']).decode();assert 'PAGE-B' in text and 'PAGE-A' not in text
        text=subprocess.check_output(['pdftotext',str(roots['cli']/'split-a.pdf'),'-']).decode();assert 'PAGE-A' in text and 'PAGE-B' not in text
        text=subprocess.check_output(['pdftotext',str(roots['cli']/'split-c.pdf'),'-']).decode();assert text.index('PAGE-B')<text.index('PAGE-C') and 'PAGE-A' not in text
        text=subprocess.check_output(['pdftotext',str(roots['cli']/'trimmed.pdf'),'-']).decode();assert 'PAGE-B' in text and 'PAGE-A' not in text and 'PAGE-C' not in text
        text=subprocess.check_output(['pdftotext',str(roots['cli']/'compressed.pdf'),'-']).decode();assert [text.index(s) for s in ['PAGE-A','PAGE-B','PAGE-C']]==sorted(text.index(s) for s in ['PAGE-A','PAGE-B','PAGE-C'])
        text=subprocess.check_output(['pdftotext',str(roots['cli']/'compressed-lossy.pdf'),'-']).decode();assert [text.index(s) for s in ['PAGE-A','PAGE-B','PAGE-C']]==sorted(text.index(s) for s in ['PAGE-A','PAGE-B','PAGE-C'])
        comp_res=json.loads(subprocess.check_output([str(BIN),'compress','--root',str(roots['cli']),'--qpdf',str(QPDF),'--request-json',json.dumps({'input':{'path':'three.pdf'},'output':{'path':'compressed.pdf','replaceExisting':True}})]))['result']
        assert comp_res['mode']=='lossless' and comp_res['pageCount']==3 and comp_res['savedBytes']==comp_res['inputBytes']-comp_res['bytes'] and comp_res['savedBytes']>0
        recomp=json.loads(subprocess.check_output([str(BIN),'compress','--root',str(roots['cli']),'--qpdf',str(QPDF),'--request-json',json.dumps({'input':{'path':'compressed.pdf'},'output':{'path':'recompressed.pdf'}})]))
        assert recomp['result']['outputSource']=='original' and recomp['result']['sha256']==digest(roots['cli']/'compressed.pdf') and recomp['result']['savedBytes']==0 and recomp['result']['compressionRatio']==1.0 and any(w['code']=='NO_SIZE_REDUCTION' for w in recomp['warnings'])
        expected={
          'rich-extract.pdf':('preserved','preserved','preserved','preserved'),
          'rich-remove.pdf':('preserved','omitted','absent','preserved'),
          'rich-merge.pdf':('omitted','omitted','preserved','omitted'),
          'rich-primary.pdf':('preserved','preserved','preserved','preserved'),
          'rich-secondary.pdf':('omitted','omitted','preserved','omitted'),
        }
        for root in roots.values():
            assert not (root/'large-unmet.pdf').exists()
            assert len(subprocess.check_output([str(QPDF),'--show-attachment=large.txt',str(root/'large-extract.pdf')]))==33*1024*1024
            primary=qpdf_features(root/'primary.pdf')
            for name,statuses in expected.items():
                report=responses[name];features=qpdf_features(root/name)
                assert report['result']['preservation']==dict(zip(['metadata','bookmarks','annotations','attachments'],statuses)),(name,report)
                assert features['annotations']=={'rich-extract.pdf':[1,1],'rich-remove.pdf':[0],'rich-merge.pdf':[1,0,1,1,0,1],'rich-primary.pdf':[1,0,1,0],'rich-secondary.pdf':[0,1,0,1]}[name],(name,features)
                if name!='rich-secondary.pdf':
                    assert features['metadata']==primary['metadata'] and features['xmp']==primary['xmp'] and features['attachments']==primary['attachments']
                else:assert features['metadata']=={} and not features['xmp'] and not features['attachments'] and not features['outlines']
                if name=='rich-remove.pdf':assert features['outlines']==[('First page',None),('Last page',None)]
                elif name=='rich-extract.pdf':assert features['outlines']==[('First page',1),('Last page',2)]
                elif name!='rich-secondary.pdf':assert features['outlines']==primary['outlines']
                omitted=[key for key,value in report['result']['preservation'].items() if value=='omitted']
                warnings=[w for w in report['warnings'] if w['code']=='PRESERVATION_OMITTED']
                assert len(warnings)==bool(omitted)
                if omitted:assert all(key in warnings[0]['message'] for key in omitted)
            for name in ['unmet.pdf','unmet-split-a.pdf','unmet-split-b.pdf']:assert not (root/name).exists()
            assert digest(root/'fallback.pdf')==digest(root/'compressed.pdf')
        assert responses['fallback.pdf']['result']['outputSource']=='original'
        assert responses['large-extract.pdf']['result']['preservation']['attachments']=='omitted'
        assert any(w['code']=='PRESERVATION_OMITTED' and 'attachments' in w['message'] for w in responses['large-extract.pdf']['warnings'])
        assert responses['larger.pdf']['result']['outputSource']=='compressed' and responses['larger.pdf']['result']['bytes']>=responses['larger.pdf']['result']['inputBytes']
        for root in roots.values():assert not (root/'split-rollback.pdf').exists()
        # Verify enriched inspectPdf metadata, page geometry, rotation, and action signals.
        mixed_inspect=json.loads(subprocess.check_output([str(BIN),'inspect','--root',str(roots['cli']),'--qpdf',str(QPDF),'--request-json',json.dumps({'input':{'path':'mixed.pdf'}})]))['result']
        assert mixed_inspect['pdfVersion']=='1.7' and mixed_inspect['actions']=='absent' and mixed_inspect['hasOutline'] is False and mixed_inspect['hasAttachments'] is False and mixed_inspect['metadata']=={}
        assert mixed_inspect['pages']==[{'pageNumber':1,'rotation':0,'widthPoints':300.0,'heightPoints':200.0,'annotationCount':0},{'pageNumber':2,'rotation':90,'widthPoints':310.0,'heightPoints':200.0,'annotationCount':0}]
        action_inspect=json.loads(subprocess.check_output([str(BIN),'inspect','--root',str(roots['cli']),'--qpdf',str(QPDF),'--request-json',json.dumps({'input':{'path':'action.pdf'}})]))['result']
        assert action_inspect['actions']=='present'
        # A version-reporting wrapper still delegates every PDF command to real qpdf.
        version_wrapper=base/'version-qpdf'
        version_wrapper.write_text('#!/usr/bin/python3\nimport os,sys\nif sys.argv[1:]==["--version"]: print("qpdf version 12.4.10")\nelse: os.execv('+repr(str(QPDF))+', ['+repr(str(QPDF))+']+sys.argv[1:])\n');version_wrapper.chmod(0o700)
        env=dict(os.environ);env['ANPDF_QPDF_EXACT']='0'
        args=[str(BIN),'merge','--root',str(roots['cli']),'--qpdf',str(version_wrapper),'--request-json',json.dumps({'inputs':[{'path':'one.pdf'},{'path':'three.pdf'}],'output':{'path':'version-output.pdf'}})]
        r=subprocess.run(args,capture_output=True,text=True,env=env);assert r.returncode==0,r.stderr
        assert len([w for w in json.loads(r.stdout)['warnings'] if w['code']=='BACKEND_VERSION_UNTESTED'])==1
        (roots['cli']/'version-output.pdf').unlink()
        for strict_args,strict_env in [(args+['--qpdf-exact'],env),(args,dict(env,ANPDF_QPDF_EXACT='1'))]:
            r=subprocess.run(strict_args,capture_output=True,text=True,env=strict_env)
            assert r.returncode==8 and json.loads(r.stderr)['error']['code']=='BACKEND_UNAVAILABLE',r
        # Verify CLI help/version/completions and trusted-PATH qpdf auto-discovery.
        for args in [['--help'],['-h'],['help'],['split','--help'],['completions','--help'],['completions','bash'],['completions','zsh'],['completions','fish'],['--version'],['-V'],['version']]:
            r=subprocess.run([str(BIN),*args],capture_output=True,text=True);assert r.returncode==0 and r.stdout and not r.stderr,(args,r)
        if not any(pathlib.Path(p).exists() for p in ['/opt/qpdf/bin/qpdf','/usr/bin/qpdf','/opt/homebrew/bin/qpdf','/usr/local/bin/qpdf']):
            env=dict(os.environ);env.pop('ANPDF_QPDF',None);env['PATH']=f"{QPDF.parent}:/usr/bin:/bin"
            r=subprocess.run([str(BIN),'inspect','--root',str(roots['cli']),'--request-json',json.dumps({'input':{'path':'one.pdf'}})],capture_output=True,text=True,env=env)
            assert r.returncode==0 and json.loads(r.stdout)['result']['pageCount']==1,r.stderr
        # Surface controls: request files, stdin, dry-run, invalid flags and malformed JSON.
        req={'input':{'path':'three.pdf'},'pages':[1],'output':{'path':'dry.pdf'}}
        result=subprocess.run([str(BIN),'extract','--root',str(roots['cli']),'--dry-run'],input=json.dumps(req),text=True,capture_output=True);assert result.returncode==0 and json.loads(result.stdout)['dryRun'];assert not (roots['cli']/'dry.pdf').exists()
        sreq={'input':{'path':'three.pdf'},'parts':[{'pages':[1],'output':{'path':'dry-1.pdf'}},{'pages':[2,3],'output':{'path':'dry-2.pdf'}}]}
        result=subprocess.run([str(BIN),'split','--root',str(roots['cli']),'--dry-run'],input=json.dumps(sreq),text=True,capture_output=True);assert result.returncode==0 and json.loads(result.stdout)['dryRun'];assert not (roots['cli']/'dry-1.pdf').exists() and not (roots['cli']/'dry-2.pdf').exists()
        for args in [['inspect','--bogus'],['inspect','--request-json','{'],['completions'],['completions','bogus'],['fill-forms']]:
            r=subprocess.run([str(BIN),*args],capture_output=True,text=True);assert r.returncode and not r.stdout;json.loads(r.stderr)
        mcp_send(mcp,{'jsonrpc':'2.0','id':100,'method':'tools/call','params':{'name':'fillForms','arguments':{}}});assert mcp_read(mcp)['error']['code']==-32602
        print(json.dumps({'crossSurfaceCases':len(corpus),'surfaces':3,'inputPreservation':'passed','pageOrder':'Poppler text verified','httpPolicy':'passed','mcpLifecycle':'passed'}))
    finally:
        mcp.stdin.close();mcp.wait(timeout=10);assert not mcp.stderr.read(),'MCP stderr leakage'
        server.terminate();server.wait(timeout=10);assert not server.stdout.read(),'HTTP stdout leakage'
if __name__=='__main__':run()
