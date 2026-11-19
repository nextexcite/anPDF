"""Real qpdf cross-surface qualification. Requires built anpdf, qpdf 12.4.2, Poppler."""
import hashlib,importlib.util,json,os,pathlib,shutil,socket,subprocess,tempfile,time,urllib.request,urllib.error
ROOT=pathlib.Path(__file__).resolve().parents[1]
BIN=pathlib.Path(os.environ.get('ANPDF_BINARY',ROOT/'target/debug/anpdf')).resolve()
QPDF=pathlib.Path(os.environ.get('ANPDF_QPDF','/usr/bin/qpdf')).resolve()
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

def run():
 with tempfile.TemporaryDirectory(prefix='anpdf-tests-') as t:
    base=pathlib.Path(t);roots={s:base/s for s in ['cli','http','mcp']}
    for root in roots.values():
        gen.generate(root)
        subprocess.run([str(QPDF),'--encrypt','','synthetic-only','256','--',str(root/'one.pdf'),str(root/'encrypted.pdf')],check=True,capture_output=True)
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
        cli=json.loads(subprocess.check_output([str(BIN),'operations']));assert cli==discovery
        assert http(port,'invalid','/v1/operations')[0]==401
        assert http(port,token,'/v1/operations',extra={'Origin':'https://example.invalid'})[0]==403
        assert http(port,token,'/v1/operations',extra={'Host':'example.invalid'})[0]==403
        assert http(port,token,'/v1/openapi.json')[1]==json.loads((ROOT/'contracts/v1/openapi.json').read_text())
        corpus=[
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
          ('merge',{'inputs':[{'path':'three.pdf'},{'path':'one.pdf'}],'output':{'path':'merged.pdf'}},'OUTPUT_EXISTS'),
          ('extract',{'input':{'path':'three.pdf'},'pages':[2],'output':{'path':'subset.pdf','replaceExisting':True}},None),
          ('inspect',{'input':{'path':'damaged.pdf'}},'INVALID_PDF'),
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
        originals={surface:{f.name:digest(f) for f in root.glob('*.pdf')} for surface,root in roots.items()}
        for n,(command,request,error) in enumerate(corpus,10):
            op={'inspect':'inspectPdf','merge':'mergePdf','extract':'extractPdf'}[command]
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
            for surface,root in roots.items():
                for name,h in originals[surface].items():assert digest(root/name)==h,(n,'input changed',name)
        # Independent semantic page order, not just matching hashes.
        text=subprocess.check_output(['pdftotext',str(roots['cli']/'merged.pdf'),'-']).decode()
        assert [text.index(s) for s in ['PAGE-A','PAGE-B','PAGE-C','PAGE-ONE']]==sorted(text.index(s) for s in ['PAGE-A','PAGE-B','PAGE-C','PAGE-ONE'])
        text=subprocess.check_output(['pdftotext',str(roots['cli']/'subset.pdf'),'-']).decode();assert 'PAGE-B' in text and 'PAGE-A' not in text
        # Surface controls: request files, stdin, dry-run, invalid flags and malformed JSON.
        req={'input':{'path':'three.pdf'},'pages':[1],'output':{'path':'dry.pdf'}}
        result=subprocess.run([str(BIN),'extract','--root',str(roots['cli']),'--dry-run'],input=json.dumps(req),text=True,capture_output=True);assert result.returncode==0 and json.loads(result.stdout)['dryRun'];assert not (roots['cli']/'dry.pdf').exists()
        for args in [['inspect','--bogus'],['inspect','--request-json','{'],['compress']]:
            r=subprocess.run([str(BIN),*args],capture_output=True,text=True);assert r.returncode and not r.stdout;json.loads(r.stderr)
        mcp_send(mcp,{'jsonrpc':'2.0','id':100,'method':'tools/call','params':{'name':'compress','arguments':{}}});assert mcp_read(mcp)['error']['code']==-32602
        print(json.dumps({'crossSurfaceCases':len(corpus),'surfaces':3,'inputPreservation':'passed','pageOrder':'Poppler text verified','httpPolicy':'passed','mcpLifecycle':'passed'}))
    finally:
        mcp.stdin.close();mcp.wait(timeout=10);assert not mcp.stderr.read(),'MCP stderr leakage'
        server.terminate();server.wait(timeout=10);assert not server.stdout.read(),'HTTP stdout leakage'
if __name__=='__main__':run()
