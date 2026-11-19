"""Optional CJK and real synthetic-signature fixture qualification; never writes keys.
Requires reportlab, fontTools, pypdf, pyhanko. Outputs stay in ignored build/work.
"""
import argparse,datetime,hashlib,io,json,pathlib,subprocess
p=argparse.ArgumentParser();p.add_argument('--font',required=True);p.add_argument('--out',default='build/work/edges');p.add_argument('--binary',default='target/debug/anpdf');p.add_argument('--qpdf',required=True);a=p.parse_args()
root=pathlib.Path(a.out).resolve();root.mkdir(parents=True,exist_ok=True)
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools import subset
font=TTFont(a.font);font=instantiateVariableFont(font,{'wght':400},inplace=True)
sub=subset.Subsetter();sub.populate(text='中文页面测试 ABC 123');sub.subset(font);font.save(root/'subset.ttf')
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont as RLFont
from reportlab.pdfgen import canvas
pdfmetrics.registerFont(RLFont('SyntheticCJK',str(root/'subset.ttf')))
c=canvas.Canvas(str(root/'chinese.pdf'),pagesize=(360,180),invariant=1);c.setAuthor('');c.setTitle('Synthetic CJK');c.setCreator('Synthetic fixture recipe');c.setFont('SyntheticCJK',24);c.drawString(24,90,'中文页面测试 ABC 123');c.showPage();c.save()
# Original simple document provides actual page text before signing.
import importlib.util
spec=importlib.util.spec_from_file_location('fixtures',pathlib.Path(__file__).with_name('generate.py'));g=importlib.util.module_from_spec(spec);spec.loader.exec_module(g)
source=g.pdf(['SIGNED-SYNTHETIC'])
from cryptography import x509
from cryptography.hazmat.primitives import hashes,serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID
from asn1crypto import x509 as asn1x509,keys
from pyhanko.sign import signers
from pyhanko.pdf_utils.incremental_writer import IncrementalPdfFileWriter
from pyhanko.pdf_utils.reader import PdfFileReader
from pyhanko_certvalidator.registry import SimpleCertificateStore
from pyhanko_certvalidator import ValidationContext
from pyhanko.sign.validation import validate_pdf_signature
key=rsa.generate_private_key(public_exponent=65537,key_size=2048)
subject=x509.Name([x509.NameAttribute(NameOID.COMMON_NAME,'Synthetic Test')])
cert=x509.CertificateBuilder().subject_name(subject).issuer_name(subject).public_key(key.public_key()).serial_number(1).not_valid_before(datetime.datetime(2020,1,1,tzinfo=datetime.timezone.utc)).not_valid_after(datetime.datetime(2030,1,1,tzinfo=datetime.timezone.utc)).add_extension(x509.BasicConstraints(ca=True,path_length=None),True).add_extension(x509.KeyUsage(True,True,False,False,False,True,True,False,False),True).sign(key,hashes.SHA256())
cert=asn1x509.Certificate.load(cert.public_bytes(serialization.Encoding.DER))
signer=signers.SimpleSigner(cert,keys.PrivateKeyInfo.load(key.private_bytes(serialization.Encoding.DER,serialization.PrivateFormat.PKCS8,serialization.NoEncryption())),SimpleCertificateStore())
with (root/'signed.pdf').open('wb') as out:
 signers.sign_pdf(IncrementalPdfFileWriter(io.BytesIO(source)),signers.PdfSignatureMetadata(field_name='SyntheticSignature'),signer=signer,output=out)
# Verify cryptographic integrity using only the ephemeral self-signed test root.
with (root/'signed.pdf').open('rb') as f:
 sig=PdfFileReader(f).embedded_signatures[0];status=validate_pdf_signature(sig,ValidationContext(trust_roots=[cert],allow_fetching=False));assert status.intact and status.valid
signed=(root/'signed.pdf').read_bytes();assert b'SIGNED-SYNTHETIC' in signed
(root/'tampered.pdf').write_bytes(signed.replace(b'SIGNED-SYNTHETIC',b'SIGNED-TAMPERED!',1))
with (root/'tampered.pdf').open('rb') as f:
 status=validate_pdf_signature(PdfFileReader(f).embedded_signatures[0],ValidationContext(trust_roots=[cert],allow_fetching=False));assert not status.intact
# Product must refuse both signatures without implying cryptographic trust.
for name in ['signed.pdf','tampered.pdf']:
 req={'input':{'path':name},'pages':[1],'output':{'path':'blocked.pdf'}}
 r=subprocess.run([a.binary,'extract','--root',str(root),'--qpdf',a.qpdf,'--request-json',json.dumps(req)],capture_output=True,text=True)
 assert r.returncode==5 and json.loads(r.stderr)['error']['code']=='SIGNED_UNSUPPORTED',(r.stdout,r.stderr)
 assert not (root/'blocked.pdf').exists()
# Embedded CJK text must survive a structural extraction and independent text extraction.
req={'input':{'path':'chinese.pdf'},'pages':[1],'output':{'path':'chinese-subset.pdf'}}
r=subprocess.run([a.binary,'extract','--root',str(root),'--qpdf',a.qpdf,'--request-json',json.dumps(req)],capture_output=True,text=True);assert r.returncode==0,r.stderr
text=subprocess.check_output(['pdftotext',str(root/'chinese-subset.pdf'),'-']).decode();assert '中文页面测试' in text,text
for name in ['chinese','chinese-subset','signed']:
 subprocess.run(['pdftoppm','-scale-to','1000','-singlefile','-png',str(root/(name+'.pdf')),str(root/name)],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
manifest=[{'file':f.name,'sha256':hashlib.sha256(f.read_bytes()).hexdigest(),'license':'Apache-2.0; embedded CJK font OFL-1.1' if 'chinese' in f.name else 'Apache-2.0','source':'original optional fixtures/edge-cases.py; ephemeral test certificate only'} for f in root.glob('*.pdf')]
(root/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(json.dumps({'cjkEmbeddedText':'passed','signatureIntegrity':'verified synthetic test root','tamperDetection':'passed','signedTransformationRejection':'passed','renders':3}))
