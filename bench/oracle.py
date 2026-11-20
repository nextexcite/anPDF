"""Independent pypdf feature observations; page numbers are one-based."""
import hashlib
from pypdf import PdfReader


def inspect_pdf(path):
    reader = PdfReader(path)
    pages = list(reader.pages)
    refs = {p.indirect_reference.idnum: i + 1 for i, p in enumerate(pages)}

    def destination(dest):
        if isinstance(dest, str):
            dest = reader.named_destinations.get(dest)
            return reader.get_destination_page_number(dest) + 1 if dest else None
        if isinstance(dest, list) and dest:
            return refs.get(getattr(dest[0], 'idnum', None))
        return None

    def outlines(items):
        result = []
        for item in items:
            if isinstance(item, list):
                if result:
                    result[-1]['children'] = outlines(item)
            else:
                result.append({'title': item.title,
                               'page': reader.get_destination_page_number(item) + 1,
                               'children': []})
        return result

    links, annotations = [], []
    for i, page in enumerate(pages, 1):
        subtypes = []
        for ref in page.get('/Annots', []):
            annot = ref.get_object()
            subtype = str(annot.get('/Subtype', '')).lstrip('/')
            subtypes.append(subtype)
            if subtype == 'Link':
                action = annot.get('/A', {})
                if action.get('/S') == '/URI':
                    links.append({'page': i, 'uri': str(action['/URI'])})
                else:
                    links.append({'page': i, 'destPage': destination(action.get('/D', annot.get('/Dest')))})
        annotations.append(sorted(subtypes))
    fields, signatures = [], []
    for name, field in (reader.get_fields() or {}).items():
        kind = str(field.get('/FT', '')).lstrip('/')
        if kind == 'Sig':
            signatures.append({'name': name, 'signed': bool(field.get('/V'))})
        else:
            value = field.get('/V', '')
            fields.append({'name': name, 'type': kind, 'value': str(value)})
    return {'pages': len(pages), 'text': [p.extract_text() or '' for p in pages],
            'pageSizes': [[float(p.mediabox.width), float(p.mediabox.height)] for p in pages],
            'rotations': [int(p.get('/Rotate', 0)) % 360 for p in pages],
            'outlines': outlines(reader.outline), 'links': links, 'annotations': annotations,
            'formFields': sorted(fields, key=lambda f: f['name']),
            'signatureFields': sorted(signatures, key=lambda f: f['name']),
            'imageCount': sum(len(p.images) for p in pages),
            'metadata': {str(k).lstrip('/'): str(v) for k, v in (reader.metadata or {}).items()}}


def signature_evidence(path):
    """Cryptographic integrity, not signer identity or public trust."""
    from pyhanko.pdf_utils.reader import PdfFileReader
    from pyhanko.sign.validation import validate_pdf_signature
    from pyhanko_certvalidator import ValidationContext
    with open(path, 'rb') as stream:
        signatures = PdfFileReader(stream).embedded_signatures
        return [{'name': sig.field_name,
                 'contents': hashlib.sha256(sig.pkcs7_content).hexdigest(),
                 'valid': bool((status := validate_pdf_signature(
                     sig, ValidationContext(trust_roots=[sig.signer_cert], allow_fetching=False))).intact and status.valid)}
                for sig in signatures]
