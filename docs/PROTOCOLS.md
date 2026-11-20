# Shared protocol behavior

The sole PDF operation/schema authority is `contracts/v1/openapi.json` (1.0.0).
The contract hash and pinned Forge provenance are checked during Rust builds.
Discovery includes resolved input, success and error schemas, limits, capability
status, and explicit unsupported/excluded capabilities.

| Operation | CLI | HTTP | MCP tool |
|---|---|---|---|
| inspectPdf | inspect / validate | POST /v1/inspect | inspectPdf |
| mergePdf | merge | POST /v1/merge | mergePdf |
| extractPdf | extract | POST /v1/extract | extractPdf |
| splitPdf | split | POST /v1/split | splitPdf |
| removePages | remove-pages | POST /v1/remove-pages | removePages |
| compressPdf | compress | POST /v1/compress | compressPdf |
| extractText | extract-text | POST /v1/extract-text | extractText |
| renderPage | render-page | POST /v1/render-page | renderPage |

CLI discovery/utility commands: `operations`, `schema <operationId>`, `completions <bash|zsh|fish>`, `help [command]` / `--help` / `-h`, `version` / `--version` / `-V`.
When neither `--qpdf` nor `ANPDF_QPDF` is set, the CLI/server checks standard system paths (`/opt/qpdf/bin/qpdf`, `/usr/bin/qpdf`, `/opt/homebrew/bin/qpdf`, `/usr/local/bin/qpdf`) and trusted `PATH` directories (absolute, owned by the effective user or root, without group/other write bits).

Authenticated HTTP discovery: GET `/v1/operations` and `/v1/openapi.json`.
A canonical error is `{contractVersion, operationId, error:{code,message,retryable}}`.
Messages never include raw backend output, absolute input paths, or credentials.

| Error | HTTP | CLI |
|---|---|---|
| INVALID_REQUEST | 400 | 2 |
| NOT_FOUND / ACCESS_DENIED | 404 / 403 | 3 |
| OUTPUT_EXISTS / INPUT_CHANGED | 409 | 4 |
| INVALID_PDF / ENCRYPTED_UNSUPPORTED / SIGNED_UNSUPPORTED / FORM_UNSUPPORTED / PRESERVATION_UNKNOWN / PRESERVATION_UNMET | 422 | 5 |
| UNSUPPORTED_OPERATION | 422 | 6 |
| LIMIT_EXCEEDED / TIMEOUT | 413 / 504 | 7 |
| BACKEND_UNAVAILABLE / INTERNAL | 503 / 500 | 8 |
| UNAUTHENTICATED | 401 | HTTP only |

CLI cancellation/termination exits 130 after cancelling work. `--dry-run` returns
a surface-control response identifying that content validation still requires
processing; it is not a PDF success result or a fourth PDF operation.

### Text and rendering engine

`extractText` takes `input` and a strictly increasing list of 1–100 one-based
`pages`. It returns each requested page's plain text and positioned spans in
engine reading order. Bounds are axis-aligned in unrotated PDF default user
space (origin not rebased to the crop box, x right, y up). A page without
embedded text can return empty text and spans; there is no OCR. The compact
success envelope is limited to 8 MiB.

`renderPage` takes `input`, a one-based `page`, `output` and optional
`dpi` (36–300, default 144). It writes a PNG of the visible page after crop
and rotation, on white and without annotations, and returns the path, SHA-256,
byte count, media type and pixel dimensions. Rendering is limited to 8192
pixels per side, 16,777,216 total pixels and 64 MiB. The normal `output.replaceExisting`
policy applies and the original PDF is never overwritten.

Both operations accept an optional `engine` ID and report the selected
`engine` in the result. CLI and HTTP discovery list configured `engines` in
preference order; they do not probe engine health. MCP lists both schemas even
when a backend is not installed. Missing providers, including an explicit
engine not registered for that capability, return `BACKEND_UNAVAILABLE` before
opening the input. No request is retried on a different engine. The qpdf-only
build (`--no-default-features`) exposes both contracts without a content provider.

Standard releases use PDFium for both operations; qpdf continues to handle
structural operations and content preflight. The default Cargo build includes
PDFium support, but still dynamically loads its library in the isolated worker.
Library selection is `--pdfium-lib`, then non-empty `ANPDF_PDFIUM_LIB`, then
`../lib/anpdf/libpdfium.{so,dylib}` relative to the canonical executable directory,
then `libpdfium.{so,dylib}` beside that executable. Explicit paths must be absolute
and override automatic discovery even when missing. Only regular library files
register the engine; automatic candidates must be in trusted directories.
Discovery lists `pdfium` when configured (without probing library health).
Omitting `engine` selects PDFium. PDFOxide is distributed as a separate preview tag.

The shared dispatcher preflights PDFs with qpdf, enforces input and destination
policy, and gives the selected provider only private staging paths and the
shared job deadline/cancellation control. Signed and form-bearing inputs
are not blocked by the PDF preservation policy for these non-mutating
operations; encrypted inputs remain unsupported. Engine adapters must use
bounded workers and enforce raster/text limits before allocating or emitting
large results. The gateway validates the result schema and bounds, and checks
the PNG signature and dimensions before atomically publishing the artifact.

#### Experimental PDFOxide build

`cargo build --locked --features pdfoxide` registers `pdfoxide` for both
operations on CLI, HTTP, and MCP. qpdf 12.4.2 is still required for structural
preflight. For comparison runs, set `"engine":"pdfoxide"` explicitly rather
than relying on provider order:

```sh
target/debug/anpdf extract-text --root ./docs --request-json '{"input":{"path":"sample.pdf"},"pages":[1],"engine":"pdfoxide"}'
target/debug/anpdf render-page --root ./docs --request-json '{"input":{"path":"sample.pdf"},"page":1,"dpi":144,"output":{"path":"page.png"},"engine":"pdfoxide"}'
```

The adapter uses `pdf_oxide` 0.3.78's embedded text and native PNG renderer in
a short-lived subprocess. It does not enable OCR. Missing embedded fonts may
use system fonts, so control the font environment when comparing pixels across
machines. With `extractText`, filled form text-field values can appear in
`text` but are not included in `spans`, so callers cannot locate those values
from the extraction result. For pages with a non-default PDF `/UserUnit`,
`renderPage` returns `UNSUPPORTED_OPERATION` rather than an image at the
wrong DPI.
The feature build is for evaluation, not the default distribution.
Run the existing cross-surface fixture suite with `ANPDF_EXPECT_PDFOXIDE=1`
and `ANPDF_QPDF` pointing at the pinned qpdf.

A separate experimental artifact can be built with
`cargo build --locked --release --features pdfoxide`; the standard release
workflow does not enable this feature. The Rust dependency and bundled font
notices are indexed in `license-manifest.json`; verify them with
`python3 build/licenses.py --check --features pdfoxide`. See
[PDFOxide preview releases](BUILD.md#pdfoxide-preview-releases) for packaging
and release checks.

MCP is newline-delimited JSON-RPC 2.0 on stdio, negotiating version 2025-11-25.
Send each line and await its response; initialization notification has no response:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"example-client","version":"1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspectPdf","arguments":{"input":{"path":"three.pdf"}}}}
```

Successful MCP tools return validated `structuredContent` plus equivalent JSON
text. Application failures use `isError:true` and canonical JSON error text.
Invalid JSON-RPC/protocol requests use protocol errors. Unknown tools are not
registered. `notifications/cancelled` cancels the matching active request. EOF
cancels active work and waits for cleanup. No log messages are written to stdout.
The adapter uses the owned contract for tool schemas and truthful read-only and
local-operation annotations; annotations do not grant filesystem permissions.

The integration corpus drives all three surfaces with identical requests and
compares complete domain envelopes, errors, discovery schemas, and input hashes.
The example MCP sequence, CLI invocations and authenticated HTTP operations are
exercised by that harness, not by an external client certification suite.

## Preservation and compression

`mergePdf`, `extractPdf`, `splitPdf`, and `removePages` use the first input as
qpdf's primary document, never `--empty`. Optional closed-object `preserve`
accepts `metadata`, `bookmarks`, `annotations`, and `attachments` booleans,
default false. False means best effort, not removal. True requires full carryover
when present; `PRESERVATION_UNMET` (422, exit 5, not retryable) publishes nothing,
including when a later split part fails. Each artifact has required `preservation`
statuses: `preserved`, `omitted` (also unverified), or `absent`.

Comparisons use complete Info dictionary/XMP hashes, normalized outline structure
and mapped destinations, attachment descriptors and content hashes, and the sum
of optional per-page `annotationCount` over selected pages. Optional inspect
`hasMetadata` and `preservationEvidence` support these comparisons without changing
backend trait signatures. Missing evidence is conservative: omitted, not success.
Secondary merge metadata differing from output is omitted; secondary bookmarks
or attachments are always conservatively omitted. Unknown/multi-stream attachment
variants and indirect Info values are not certified preserved. Annotation counts
are structural evidence, not proof of identical appearances or action semantics.
One `PRESERVATION_OMITTED` warning names the union of omitted aspects per response.

Synthetic qpdf 12.4.2 observations (three pages, Info/custom field, XMP, two
outlines to pages 1 and 3, embedded text file, text/link annotations on pages 1/3):

| Transform | Info/XMP | Outlines | Embedded file | Annotation counts |
|---|---|---|---|---|
| Extract pages 1,3 | Primary retained | Both retained, destinations remapped to 1,2 | Primary retained | 1,1 |
| Remove pages 1,3 | Primary retained | Entries retained but destinations no longer in page tree; reported omitted | Primary retained | 0 (absent on selected page) |
| Merge two enriched documents | Primary retained; differing secondary metadata omitted | Primary only; secondary omitted | Primary only; secondary omitted | 1,0,1,1,0,1 |
| Merge enriched primary + plain page | Primary retained | Primary retained | Primary retained | 1,0,1,0 |
| Merge plain primary + enriched secondary | Secondary omitted | Secondary omitted | Secondary omitted | 0,1,0,1 |

These observations are reproduced independently with qpdf JSON/stream extraction
in the cross-surface integration suite, not general visual preservation guarantees.
Page reordering, forms, signatures, OCR and existing-text replacement remain
unimplemented.

`compressPdf.allowLargerOutput` defaults false. If compressed bytes are not
smaller, the already-staged validated original is committed instead. Required
`outputSource` is `original` or `compressed`; all result measurements describe
the published bytes (`savedBytes:0`, `compressionRatio:1.0` for original).
`NO_SIZE_REDUCTION` identifies which bytes were kept.

qpdf version parsing accepts only 12.x >=12.4.2; 13+, older and malformed versions
fail `BACKEND_UNAVAILABLE`. Only 12.4.2 is qualified. Other accepted versions
emit `BACKEND_VERSION_UNTESTED` once per response. `--qpdf-exact` or process-level
`ANPDF_QPDF_EXACT=1` requires exactly 12.4.2; CI and the OCI runtime enable it.
