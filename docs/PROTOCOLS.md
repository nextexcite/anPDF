# Shared protocol behavior

The sole PDF operation/schema authority is `contracts/v1/openapi.json` (1.0.0).
`contractVersion` is the schema version, not the installed anPDF version.
The contract hash and pinned Forge provenance are checked during Rust builds.
CLI and HTTP discovery include resolved input, success and error schemas, limits,
configured capabilities (not backend health), and explicit unsupported/excluded
capabilities. MCP advertises the same operation descriptions and input/success
schemas. The discovery wall time is an upper bound, not every job's deadline.

| Operation | CLI | HTTP | MCP tool |
|---|---|---|---|
| inspectPdf | inspect / validate | POST /v1/inspect | inspectPdf |
| mergePdf | merge | POST /v1/merge | mergePdf |
| extractPdf | extract | POST /v1/extract | extractPdf |
| splitPdf | split | POST /v1/split | splitPdf |
| removePages | remove-pages | POST /v1/remove-pages | removePages |
| reorderPages | reorder | POST /v1/reorder-pages | reorderPages |
| compressPdf | compress | POST /v1/compress | compressPdf |
| extractText | extract-text | POST /v1/extract-text | extractText |
| renderPage | render-page | POST /v1/render-page | renderPage |
| encryptPdf | encrypt | POST /v1/encrypt | encryptPdf |
| decryptPdf | decrypt | POST /v1/decrypt | decryptPdf |

CLI discovery/utility commands: `operations`, `schema <operationId>`, `completions <bash|zsh|fish>`, `help [command]` / `--help` / `-h`, `version` / `--version` / `-V`. Top-level help groups commands by task and shared options by purpose for discovery, but the command syntax remains flat (`anpdf reorder`, not a nested action family). Per-command help puts operation semantics and examples first, then separates output, operation-specific, credential, safety/server, and shared backend/transport options.
When neither `--qpdf` nor `ANPDF_QPDF` is set, the CLI/server checks standard system paths (`/opt/qpdf/bin/qpdf`, `/usr/bin/qpdf`, `/opt/homebrew/bin/qpdf`, `/usr/local/bin/qpdf`) and trusted `PATH` directories (absolute, owned by the effective user or root, without group/other write bits).

For a single-output agent task, start MCP with a runner-owned publication policy:

```sh
anpdf mcp --root ./workspace --output-only compact.pdf \
  --output-operation compressPdf --output-byte-ceiling 50000
```

- `--output-only` permits creation at exactly that workspace-relative path and denies replacement and other paths with `ACCESS_DENIED`. Split needs distinct destinations and cannot succeed in that session.
- `--output-operation` optionally limits writes to one single-output operation ID. For example, `extractPdf` cannot substitute a smaller subset for a `compressPdf` job. Read-only operations are unaffected.
- `--output-byte-ceiling` optionally caps the bytes actually published. An oversize artifact returns `LIMIT_EXCEEDED` without publishing, even if the caller omits its target or requests best effort. Strict compression still returns `SIZE_TARGET_UNMET` when it misses its own target.

In a restricted session, `tools/list` omits split and any writer excluded by `--output-operation`. Read-only tools remain listed, and the available writers describe the startup policy without changing their request or response schemas. Discovery is not enforcement: a direct call to a hidden writer still goes through the publication policy. An unrestricted session still lists all eleven operations.

This is not an input allowlist or a sandbox: expose only the intended input workspace and give the agent no other way to write there. Use a `lossless-only` build as well when downgrades are forbidden.

CLI operations also accept positional shorthand instead of `--request-json`/`--request-file`; both forms produce the same contract request and identical envelopes. `<pages>` is a comma list of 1-based numbers and `a-b` ranges. For `reorder`, `<ordered-pages>` is output order rather than a selection: it must cover every input page exactly once. `3,1,2` is valid for reorder but invalid for extract, whose selection remains strictly increasing. Ranges expand ascending, so use explicit comma-separated numbers for non-ascending orders. The normal distinct-destination policy applies.

| CLI shorthand | Request fields |
|---|---|
| `anpdf inspect <doc.pdf>` | `input` |
| `anpdf merge <a.pdf> <b.pdf> [...] [-o <out.pdf>]` | `inputs` (2–20), `output` (`-o` or trailing positional) |
| `anpdf extract <doc.pdf> <pages> [-o <out.pdf>]` | `input`, `pages`, `output` |
| `anpdf split <doc.pdf> <out=pages>...` | `input`, `parts` (2–20 `output=pages` pairs) |
| `anpdf remove-pages <doc.pdf> <pages> [-o <out.pdf>]` | `input`, `pages`, `output` |
| `anpdf reorder <doc.pdf> <ordered-pages> [-o <out.pdf>]` | `input`, `pages` (full permutation in output order), `output` |
| `anpdf compress <doc.pdf> [<out.pdf>]` | `input`, `output` (defaults to `<name>-compressed.pdf`; `--mode`, `--jpeg-quality`, `--level`, `--max-bytes`, `--on-target-miss`, `--allow-larger` map to `mode`, `jpegQuality`, `compressionLevel`, `maxOutputBytes`, `onTargetMiss`, `allowLargerOutput`) |
| `anpdf extract-text <doc.pdf> <pages>` | `input`, `pages` (+`--engine`) |
| `anpdf render-page <doc.pdf> <page> [-o <image>]` | `input`, `page`, `output` (+`--dpi`, `--engine`) |
| `anpdf encrypt <doc.pdf> [<out.pdf>]` | `input`, `output` (defaults to `<name>-encrypted.pdf`) |
| `anpdf decrypt <doc.pdf> [<out.pdf>]` | `input`, `output` (defaults to `<name>-decrypted.pdf`) |

Prefer `-o` for single-output shorthand, especially for merge: without it the final positional is the destination. `--replace` sets `output.replaceExisting` (for every part in split): it can overwrite a distinct destination, never an input, and does not grant any permission to change fidelity. `--preserve metadata,bookmarks,annotations,attachments` (or `all`) maps to `preserve`. `--` terminates flag parsing for dash-leading filenames. `extract`, `remove-pages`, `reorder` and `extract-text` also accept `--pages <spec>` in place of the positional. Shorthand paths may be absolute or `~/`-prefixed: the invocation's workspace root then adapts to the files' common canonical directory, while `--root` pins it and refuses inputs outside. JSON requests (inline, file, stdin, HTTP, MCP) always stay workspace-relative to `--root`.

Error messages may append a detail suffix (` — <detail>`) naming the failing field (a JSON pointer like `/input/path`), flag or file; the envelope schema and codes are unchanged. On a terminal, a bare operation or parse-stage misuse prints usage instead of a JSON envelope or blocking on stdin, successful results print a short plain summary (the pretty envelope via `--json`), and errors print `anpdf <command>: <reason>` leading with the specific detail; non-terminal stdout/stderr keep the canonical envelopes byte-identical, and piped stdin still supplies the request. Non-UTF-8 arguments fail with `INVALID_REQUEST`.

Authenticated HTTP discovery: GET `/v1/operations` and `/v1/openapi.json`.
A canonical error is `{contractVersion, operationId, error:{code,message,retryable}}`.
Messages never include raw backend output, absolute input paths, or credentials.

| Error | HTTP | CLI |
|---|---|---|
| INVALID_REQUEST | 400 | 2 |
| NOT_FOUND / ACCESS_DENIED / INVALID_PASSWORD | 404 / 403 | 3 |
| OUTPUT_EXISTS / INPUT_CHANGED | 409 | 4 |
| INVALID_PDF / ENCRYPTED_UNSUPPORTED / SIGNED_UNSUPPORTED / FORM_UNSUPPORTED / PRESERVATION_UNKNOWN / PRESERVATION_UNMET | 422 | 5 |
| UNSUPPORTED_OPERATION | 422 | 6 |
| LIMIT_EXCEEDED / TIMEOUT | 413 / 504 | 7 |
| BACKEND_UNAVAILABLE / INTERNAL | 503 / 500 | 8 |
| UNAUTHENTICATED | 401 | HTTP only |

CLI cancellation/termination exits 130 after cancelling work. CLI-only `--dry-run`
checks the request and destination policy without writing. It does not process
the PDF or verify credentials, page bounds against the document, preservation,
or whether a compression target can be met.

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
operations; encrypted inputs remain unsupported (encryptPdf/decryptPdf are
the only password-aware operations). Engine adapters must use
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

## Password protection

`encryptPdf` applies standard AES-256 encryption (PDF revision 6) through the
qualified qpdf backend: `password` becomes the document's user password and an
optional `ownerPassword` the permissions password — when omitted a random
unguessable owner is generated, never an empty backdoor; a supplied blank one
is refused because qpdf rejects it for AES-256 (the file would open without a
password). An intentional empty
`password` produces an owner-only document that opens without a password; it is
accepted (with an `EMPTY_OPEN_PASSWORD` warning that discloses the loss of
confidentiality), while an omitted or cancelled password fails closed.
`decryptPdf` removes protection with either password, including the empty one
an owner-only file carries; both operations re-inspect the output (encrypt
re-opens it with the new password) so page count, encryption state and
forms/signatures status are verified post-rewrite, and preservation evidence
hashing runs through encryption. Passwords are bounded to the PDF
specification's 127 UTF-8 *bytes* — multi-byte characters count several bytes
each — and a NUL byte is rejected outright: the backend's C-string handling
would silently truncate it and could encrypt to a password shorter than
supplied.

Secrets never appear in subprocess argv — `--request-json` refuses literal
`password`/`ownerPassword` fields — environment, logs, errors or telemetry.
argv is exposed to the operating system at launch, so that refusal cannot undo
an inline exposure; it is a guardrail toward the safe channels, not a secrecy
guarantee. The backend hands them to qpdf through an inherited anonymous
descriptor (`memfd` on Linux) or a private `0600` staging file created before
write and cleaned on every exit path, inside a job JSON document so passwords
with line breaks reach qpdf intact. Interactive CLI sessions get echo-disabled
prompts; scripts use `--password-file`/`--owner-password-file` (files must be
`chmod 600`), `--request-file` or piped stdin. HTTP and MCP requests carry the
secret fields in the request body; the contract intentionally defines no secret
file-path fields, so a remote caller cannot name a local filesystem path to
read. Signed inputs are rejected; forms/XFA ride through intact.

## Preservation and compression

`mergePdf`, `extractPdf`, `splitPdf`, `removePages`, and `reorderPages` use the first input as
qpdf's primary document, never `--empty`. Optional closed-object `preserve`
accepts `metadata`, `bookmarks`, `annotations`, and `attachments` booleans,
default false. False means best effort, not removal. True requires full carryover
when present; `PRESERVATION_UNMET` (422, exit 5, not retryable) publishes nothing,
including when a later split part fails. Each artifact has required `preservation`
statuses: `preserved`, `omitted` (also unverified), or `absent`.

Comparisons use complete Info dictionary/XMP hashes, normalized outline structure
and mapped destinations, attachment descriptors and content hashes, and the sum
of optional per-page `annotationCount` over the pages included in each output. Optional inspect
`hasMetadata` and `preservationEvidence` support these comparisons without changing
backend trait signatures. Missing evidence is conservative: omitted, not success.
Secondary merge metadata differing from output is omitted; secondary bookmarks
or attachments are always conservatively omitted. Unknown/multi-stream attachment
variants and indirect Info values are not certified preserved. Annotation counts
are structural evidence, not proof of identical appearances or action semantics.
One `PRESERVATION_OMITTED` warning names the union of omitted aspects per response.

Split accepts independently specified subsets: parts can overlap or omit pages.
All parts are staged and verified before publication, but destination writes
are separate. If a later publication fails, newly created outputs are rolled
back where possible; previous contents of replaced outputs are not restored.

Synthetic qpdf 12.4.2 observations (three pages, Info/custom field, XMP, two
outlines to pages 1 and 3, embedded text file, text/link annotations on pages 1/3):

| Transform | Info/XMP | Outlines | Embedded file | Annotation counts |
|---|---|---|---|---|
| Extract pages 1,3 | Primary retained | Both retained, destinations remapped to 1,2 | Primary retained | 1,1 |
| Remove pages 1,3 | Primary retained | Entries retained but destinations no longer in page tree; reported omitted | Primary retained | 0 (absent on selected page) |
| Reorder pages 3,1,2 | Primary retained | Both retained, destinations remapped to 2,1 | Primary retained | 1,1,0 |
| Merge two enriched documents | Primary retained; differing secondary metadata omitted | Primary only; secondary omitted | Primary only; secondary omitted | 1,0,1,1,0,1 |
| Merge enriched primary + plain page | Primary retained | Primary retained | Primary retained | 1,0,1,0 |
| Merge plain primary + enriched secondary | Secondary omitted | Secondary omitted | Secondary omitted | 0,1,0,1 |

These observations are reproduced independently with qpdf JSON/stream extraction
in the cross-surface integration suite, not general visual preservation guarantees.
`reorderPages` requires a complete permutation: every input page exactly once, in output
order, written to a distinct output. It neither inserts, duplicates, nor deletes pages;
ascending order is valid. Non-identity reordering currently fails closed with
`PRESERVATION_UNKNOWN` for tagged input because qpdf does not rewrite and verify the
logical reading-order tree; identity order remains allowed. Forms, signatures, OCR and
existing-text replacement remain unimplemented.

`compressPdf.allowLargerOutput` defaults false. If compressed bytes are not
smaller, the already-staged validated original is committed instead. Required
`outputSource` is `original` or `compressed`; all result measurements describe
the published bytes (`savedBytes:0`, `compressionRatio:1.0` for original).
`NO_SIZE_REDUCTION` identifies which bytes were kept.

For a task that must never degrade fidelity, give the agent only an anPDF binary
built with `cargo build --locked --features lossless-only`. The policy applies
to every `compressPdf` request, including through CLI, MCP, and loopback HTTP.
It rejects requests that would enable a lossy or layered pass with
`INVALID_REQUEST`, before processing or publishing a PDF. This includes targets
that implicitly enable lossy compression; choose `mode: "lossless"` explicitly.
The build also rejects direct calls to its PDF resampling and rasterization
workers; PNG rendering still works. The policy does not change the size-miss
setting or authorize replacement. A trusted runner must withhold other PDF
writers and unrestricted anPDF binaries for the same guarantee across tools.
This is not an OS sandbox.

Explicit `mode: "lossless"` never enables JPEG recompression, downsampling, or
rasterization, even with a target. `mode: "lossy"` permits JPEG optimization and,
with a target, downsampling and eligible traced-page rasterization. In v1, omitting
the mode selects lossless unless level 2–3 is requested or a target is supplied
without a level. Level 1 is lossless and conflicts with explicit lossy mode;
levels 2–3 conflict with explicit lossless mode. Levels 2 and 3 bound image
downsampling at roughly 200 and 120 DPI respectively; without a level the lossy
ladder can descend to roughly 72 DPI. `jpegQuality` sets the initial JPEG pass,
not the ladder, and requires explicit lossy mode.

With a target, use `targetMet` to distinguish a successful write from meeting
the size requirement. `onTargetMiss: "bestEffort"` (default) writes the selected
result and warns `SIZE_TARGET_UNMET`; `"error"` returns that code (422, CLI exit 5)
and leaves the destination untouched, even when replacement was allowed.

`compressPdf.mode: "layered"` requires PDFium (`BACKEND_UNAVAILABLE` otherwise).
It tries fixed 400 DPI layered rasterization after the lossless pass, only on
eligible dense traced pages. A bilevel base uses CCITT Group 4 or Flate; detected
neutral tones use grayscale Flate patches and detected color uses separate JPEG
or lossless Flate patches for sparse marks. This is a lossy reconstruction, not
OCR or guaranteed preservation of every faint detail. Ineligible pages keep
their content. The layered candidate must be smaller than the lossless pass. If
the layered pass exceeds its deadline, the lossless pass remains the candidate;
cancellation still aborts and strict size targets still apply.
`layeredRasterization` and `PAGE_RASTERIZED` identify an adopted candidate.
`maxOutputBytes` checks the finished bytes without invoking the DPI ladder;
`compressionLevel` and `jpegQuality` conflict with `layered` and return
`INVALID_REQUEST`.

qpdf version parsing accepts only 12.x >=12.4.2; 13+, older and malformed versions
fail `BACKEND_UNAVAILABLE`. Only 12.4.2 is qualified. Other accepted versions
emit `BACKEND_VERSION_UNTESTED` once per response. `--qpdf-exact` or process-level
`ANPDF_QPDF_EXACT=1` requires exactly 12.4.2; CI and the OCI runtime enable it.
