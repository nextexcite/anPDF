"""Generate public synthetic PDFs: python3 -m bench.corpus OUTDIR."""
import argparse
import datetime
import hashlib
import io
import json
import pathlib
import re
import sys

if __package__ in (None, ''):
    sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from bench.oracle import inspect_pdf
from PIL import Image, ImageDraw
from pypdf import PdfReader, PdfWriter
from pypdf.generic import ArrayObject, DictionaryObject, NameObject, NumberObject, TextStringObject
from reportlab.lib.utils import ImageReader
from reportlab.pdfgen import canvas


def signed_pdf(source):
    from cryptography import x509
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import rsa
    from cryptography.x509.oid import NameOID
    from asn1crypto import x509 as asn1x509, keys
    from pyhanko.sign import signers
    from pyhanko.pdf_utils.incremental_writer import IncrementalPdfFileWriter
    from pyhanko_certvalidator.registry import SimpleCertificateStore
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'Synthetic Test')])
    cert = (x509.CertificateBuilder().subject_name(subject).issuer_name(subject)
            .public_key(key.public_key()).serial_number(1)
            .not_valid_before(datetime.datetime(2020, 1, 1, tzinfo=datetime.timezone.utc))
            .not_valid_after(datetime.datetime(2040, 1, 1, tzinfo=datetime.timezone.utc))
            .add_extension(x509.BasicConstraints(ca=True, path_length=None), True)
            .add_extension(x509.KeyUsage(True, True, False, False, False, True, True, False, False), True)
            .sign(key, hashes.SHA256()))
    signer = signers.SimpleSigner(
        asn1x509.Certificate.load(cert.public_bytes(serialization.Encoding.DER)),
        keys.PrivateKeyInfo.load(key.private_bytes(serialization.Encoding.DER,
            serialization.PrivateFormat.PKCS8, serialization.NoEncryption())), SimpleCertificateStore())
    output = io.BytesIO()
    signers.sign_pdf(IncrementalPdfFileWriter(io.BytesIO(source)),
                     signers.PdfSignatureMetadata(field_name='SyntheticSignature'), signer=signer, output=output)
    return output.getvalue()


def generate(out, font=None, real_signed=True):
    out = pathlib.Path(out)
    out.mkdir(parents=True, exist_ok=True)
    manifest = {'version': 1, 'cases': [], 'skipped': []}

    def record(name, category, data, **flags):
        path = out / (name + '.pdf')
        path.write_bytes(data)
        expected = {'expectError': True} if category == 'malformed' else inspect_pdf(path)
        if flags.get('ocrOnly'):
            expected['text'] = ['SCANNED SYNTHETIC PAGE']
        manifest['cases'].append(dict(id=name, category=category, file=path.name,
            sha256=hashlib.sha256(data).hexdigest(), bytes=len(data), license='Apache-2.0',
            provenance='Original synthetic recipe bench/corpus.py; no source documents', expected=expected, **flags))

    def document(draw):
        stream = io.BytesIO()
        c = canvas.Canvas(stream, pagesize=(360, 480), invariant=1, pageCompression=0)
        c.setTitle('Synthetic benchmark')
        c.setAuthor('anPDF Contributors')
        c.setCreator('anPDF synthetic corpus')
        draw(c)
        c.save()
        return re.sub(rb'/(?:CreationDate|ModDate)\s*\([^)]*\)',
                      lambda match: b' ' * len(match.group()), stream.getvalue())

    def baseline(c):
        for i, size in enumerate([(360, 480), (480, 360), (300, 300)], 1):
            c.setPageSize(size)
            c.setPageRotation(90 if i == 2 else 0)
            c.drawString(30, 100, f'Synthetic page {i}')
            c.showPage()
    base = document(baseline)
    record('text-mixed', 'text', base)

    scan = Image.new('RGB', (720, 960), 'white')
    ImageDraw.Draw(scan).text((60, 200), 'SCANNED SYNTHETIC PAGE', fill='black', font_size=32)
    record('scan', 'scanned', document(lambda c: c.drawImage(ImageReader(scan), 0, 0, 360, 480)), ocrOnly=True)

    font = pathlib.Path(font or 'build/work/font/NotoSansSC.ttf')
    if font.is_file():
        from fontTools.ttLib import TTFont
        from fontTools.varLib.instancer import instantiateVariableFont
        from fontTools import subset
        from reportlab.pdfbase import pdfmetrics
        from reportlab.pdfbase.ttfonts import TTFont as RLFont
        text = '中文页面测试 日本語テスト ABC 123'
        f = instantiateVariableFont(TTFont(font), {'wght': 400}, inplace=True)
        f.recalcTimestamp = False
        sub = subset.Subsetter(); sub.populate(text=text); sub.subset(f)
        buffer = io.BytesIO(); f.save(buffer); buffer.seek(0)
        pdfmetrics.registerFont(RLFont('SyntheticCJK', buffer))
        def cjk(c):
            c.setFont('SyntheticCJK', 14); c.drawString(20, 240, text)
        record('cjk', 'cjk', document(cjk), embeddedFontLicense='OFL-1.1',
               fontProvenance='Pinned Google Fonts NotoSansSC via build/get-test-font.py')
        # Un-subsetted variant: registers the full instantiated font without
        # fontTools subsetting, exercising the font-subsetting lever.
        whole = instantiateVariableFont(TTFont(font), {'wght': 400}, inplace=True)
        whole.recalcTimestamp = False
        whole_buffer = io.BytesIO(); whole.save(whole_buffer); whole_buffer.seek(0)
        pdfmetrics.registerFont(RLFont('SyntheticCJKFull', whole_buffer))
        def cjk_full(c):
            c.setFont('SyntheticCJKFull', 14); c.drawString(20, 240, text)
        record('cjk-unsubset', 'cjk', document(cjk_full), embeddedFontLicense='OFL-1.1',
               fontProvenance='Pinned Google Fonts NotoSansSC via build/get-test-font.py')
    else:
        manifest['skipped'].append({'id': 'cjk', 'reason': 'Pinned OFL font unavailable; run build/get-test-font.py'})
        manifest['skipped'].append({'id': 'cjk-unsubset', 'reason': 'Pinned OFL font unavailable; run build/get-test-font.py'})

    # Oversampled color scan: 1600x1200 RGB drawn into 300x225pt (~384dpi),
    # far above any resample rung — the downsample lever must engage.
    big = Image.new('RGB', (1600, 1200))
    big.putdata([((x * 251 + y) % 256, (x + y * 17) % 256, (x * 3 + y) % 256)
                 for y in range(1200) for x in range(1600)])
    record('oversampled-scan', 'scanned',
           document(lambda c: c.drawImage(ImageReader(big), 0, 0, 300, 225)), ocrOnly=True)

    # Bitonal scan: 1-bit image like a fax/photocopy, also oversampled.
    mono = Image.new('1', (1600, 2200))
    mono.putdata([255 if ((x // 24) + (y // 28)) % 3 == 0 else 0
                  for y in range(2200) for x in range(1600)])
    record('bitonal-scan', 'scanned',
           document(lambda c: c.drawImage(ImageReader(mono), 0, 0, 306, 421)), ocrOnly=True)

    # Letterhead contract: mostly text with a small embedded logo strip,
    # the "text + raw image letterhead" class from the size-target research.
    logo = Image.new('RGB', (600, 120), 'white')
    ImageDraw.Draw(logo).rectangle((10, 10, 590, 110), outline='black', width=4)
    ImageDraw.Draw(logo).text((40, 45), 'SYNTHETIC LETTERHEAD', fill='black', font_size=40)
    def letterhead(c):
        c.drawImage(ImageReader(logo), 30, 380, 300, 60)
        for i in range(14):
            c.drawString(30, 340 - i * 22, f'Synthetic contract clause {i + 1}')
    record('letterhead', 'text', document(letterhead))

    def bookmarks(c):
        for i, (title, level) in enumerate([('Chapter', 0), ('Section', 1), ('Appendix', 0)]):
            c.bookmarkPage(str(i)); c.addOutlineEntry(title, str(i), level=level)
            c.drawString(30, 240, title); c.showPage()
    record('bookmarks', 'bookmarks', document(bookmarks))

    def links(c):
        c.drawString(30, 240, 'External and internal links')
        c.linkURL('https://example.invalid/', (30, 200, 250, 220))
        c.linkRect('', 'target', (30, 160, 250, 180)); c.showPage()
        c.bookmarkPage('target'); c.drawString(30, 240, 'Destination')
    record('links', 'links', document(links))

    writer = PdfWriter(clone_from=PdfReader(io.BytesIO(base)))
    from pypdf.annotations import Text, Highlight, FreeText, Rectangle
    from pypdf.generic import FloatObject
    for annotation in [Text(rect=(20, 20, 40, 40), text='Synthetic note'),
                       Highlight(rect=(20, 80, 180, 100), quad_points=ArrayObject([FloatObject(x) for x in [20,100,180,100,20,80,180,80]])),
                       FreeText(text='Synthetic free text', rect=(20, 120, 180, 150)),
                       Rectangle(rect=(20, 170, 180, 200)),
                       DictionaryObject({NameObject('/Type'): NameObject('/Annot'), NameObject('/Subtype'): NameObject('/Ink'),
                           NameObject('/Rect'): ArrayObject([NumberObject(x) for x in [20,220,180,260]]),
                           NameObject('/InkList'): ArrayObject([ArrayObject([NumberObject(x) for x in [20,220,80,250,180,230]])])})]:
        writer.add_annotation(0, annotation)
    stream = io.BytesIO(); writer.write(stream)
    record('annotations', 'annotations', stream.getvalue())

    def forms(c):
        c.drawString(30, 450, 'Synthetic form')
        form = c.acroForm
        form.textfield(name='text', value='sample', x=30, y=390, width=180, height=24)
        form.textfield(name='multiline', value='line one\nline two', fieldFlags='multiline', x=30, y=300, width=180, height=60)
        form.checkbox(name='check', checked=True, x=30, y=260)
        for i, value in enumerate(['one', 'two']):
            form.radio(name='radio', value=value, selected=i == 0, x=30 + i * 40, y=220)
        form.choice(name='choice', value='red', options=['red', 'blue'], x=30, y=160, width=180)
    record('forms', 'forms', document(forms))
    writer = PdfWriter(clone_from=PdfReader(io.BytesIO(base)))
    sig = DictionaryObject({NameObject('/FT'): NameObject('/Sig'), NameObject('/T'): TextStringObject('UnsignedSignature'),
        NameObject('/Type'): NameObject('/Annot'), NameObject('/Subtype'): NameObject('/Widget'),
        NameObject('/Rect'): ArrayObject([NumberObject(x) for x in [20,20,160,60]])})
    ref = writer._add_object(sig)
    writer.pages[0][NameObject('/Annots')] = ArrayObject([ref])
    writer._root_object[NameObject('/AcroForm')] = writer._add_object(DictionaryObject({NameObject('/Fields'): ArrayObject([ref])}))
    stream = io.BytesIO(); writer.write(stream)
    record('unsigned', 'signatures', stream.getvalue())
    if real_signed:
        record('signed', 'signatures', signed_pdf(base), nondeterministic=True)
    else:
        manifest['skipped'].append({'id': 'signed', 'reason': 'Disabled explicitly with --no-signed'})

    def images(c):
        for i in range(12):
            image = Image.new('RGB', (800, 800), (i * 19, 80, 180))
            ImageDraw.Draw(image).text((50, 50), f'Synthetic image {i}', fill='white', font_size=40)
            if i % 2:
                data = io.BytesIO(); image.save(data, 'JPEG'); data.seek(0); image = data
            c.drawImage(ImageReader(image), 10 + (i % 3) * 110, 20 + (i // 3) * 110, 100, 100)
    record('images', 'image-heavy', document(images))
    record('truncated', 'malformed', base[:len(base)//2])
    record('bad-xref', 'malformed', re.sub(rb'startxref\s+\d+', b'startxref\n1', base))
    record('stream-length', 'malformed', re.sub(rb'/Length \d+', b'/Length 99999999', base, count=1))
    record('missing-root', 'malformed', re.sub(rb'/Root \d+ 0 R', b'/NoRoot null', base))
    cycle = b'%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [2 0 R] /Count 1 >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF\n'
    record('page-cycle', 'malformed', cycle)
    record('garbage', 'malformed', b'Not a PDF\x00\xff\n')
    (out / 'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('outdir'); parser.add_argument('--font'); parser.add_argument('--no-signed', action='store_true')
    args = parser.parse_args()
    result = generate(args.outdir, args.font, not args.no_signed)
    print(json.dumps({'cases': len(result['cases']), 'skipped': result['skipped']}))
