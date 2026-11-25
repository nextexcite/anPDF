# Synthetic PDF backend comparison

This development harness compares external backend providers, not the anPDF
operation contract. It does not enable new product features. Forms, signatures,
OCR and existing-text replacement remain unimplemented in anPDF. A good score is not PDF/A, appearance, security or production qualification.

## Run

From the repository root, with Python 3 on Linux or macOS:

```sh
python3 -m pip install -r build/test-requirements.txt
python3 build/get-qpdf.py build/work/qpdf
python3 build/get-test-font.py build/work/font
# Install Poppler tools using the host package manager.
python3 bench/corpus.py build/work/bench-corpus
ANPDF_QPDF="$PWD/build/work/qpdf/bin/qpdf" python3 bench/harness.py \
  --corpus build/work/bench-corpus \
  --provider 'qpdf=python3 bench/providers/qpdf.py' \
  --provider 'poppler=python3 bench/providers/poppler.py' \
  --out build/work/bench-results --repeat 3 --timeout 10
python3 -m unittest bench/test_bench.py -v
```

Use the executable path printed by `get-qpdf.py` if different on your platform.
Generate into an ignored directory; do not commit generated binaries, certificates,
font files, outputs or host paths. All source documents are synthetic and the recipes
are Apache-2.0. The embedded CJK font additionally carries OFL-1.1; the pinned
font downloader retrieves its license. No real documents or private keys are used.
The ephemeral signing key is generated in memory and never serialized to disk.
The self-signed certificate represents only a synthetic test identity.

## Corpus and oracle

`manifest.json` contains `version`, `cases` and `skipped`. Each case has `id`,
`category`, relative `file`, `sha256`, `bytes`, `license`, `provenance`, and
`expected`. Optional `ocrOnly`, `nondeterministic`, and font provenance fields
explain exceptions. ReportLab invariant mode, fixed metadata, removed document
date entries and fixed font timestamps make ordinary PDFs byte deterministic
with the pinned dependencies. The real-signed case is explicitly nondeterministic
(random key and signing time); `--no-signed` disables it and records why.
Missing CJK fonts cause an explicit manifest skip, never a substitute font.
Use `--font` to select the font downloaded by the pinned script.

Cases include mixed page sizes/rotation, image-only scanned text, embedded CJK,
nested bookmarks, URI/internal links, Text/Highlight/FreeText/Square/Ink annotations,
AcroForm text/multiline/checkbox/radio/choice widgets with appearances, unsigned
and real signed fields, JPEG/Flate images, and six malformed structures.
The malformed set is bounded regression input, not fuzzing coverage.

The independent `bench/oracle.py` uses pypdf to observe the created documents.
Recipe assertions in the tests anchor those observations to known content.
Expected text for `ocrOnly` is OCR ground truth, not an assertion that a text layer
exists. No OCR task is currently scored. Expected shape (pages are one-based):

```json
{
  "pages": 1,
  "text": ["Synthetic page"],
  "pageSizes": [[360, 480]],
  "rotations": [0],
  "outlines": [{"title": "Chapter", "page": 1, "children": []}],
  "links": [{"page": 1, "uri": "https://example.invalid/"}, {"page": 1, "destPage": 1}],
  "annotations": [["Link", "Link"]],
  "formFields": [{"name": "text", "type": "Tx", "value": "sample"}],
  "signatureFields": [{"name": "signature", "signed": false}],
  "imageCount": 0,
  "metadata": {"Title": "Synthetic benchmark", "Author": "anPDF Contributors"}
}
```

PDF names omit their leading slash. Form values use their PDF string
representation (for example `/Yes`). Metadata keys omit the slash. Outlines are
nested in document order; links follow page/annotation order; annotation subtype
lists and field names are sorted. `pageSizes` are unrotated MediaBox points;
`rotations` are clockwise degrees. Malformed cases instead have
`expected: {"expectError": true}`; engines may legitimately repair them.

## External provider protocol

A provider is an argv command parsed with `shlex.split`, never a shell script
evaluation. `--provider NAME=CMD` is repeatable. Choose filename-safe provider
names. The harness executes a fresh process for every request:

* `<cmd> capabilities` writes one JSON object to stdout:
  `{"name":"engine","version":"1","tasks":["pageCount"],"artifacts":["path/to/binary"]}`.
* `<cmd> run` reads one JSON request on stdin:
  `{"task":"render","input":"absolute/input.pdf","output":"absolute/output.png","page":1,"dpi":72}`.
  It writes one JSON object on stdout, with no surrounding diagnostics. Nonzero
  exit means a clean error; termination by signal means crash. Diagnostics belong
  on stderr. Paths are local, not URLs. Output must be written only to the requested
  output path. Inputs must never be modified.
  A successful JSON response `{"status":"unsupported","reason":"BACKEND_UNAVAILABLE"}`
  can report runtime capability unavailability; it is excluded from scoring.
* Supported tasks and result shapes:
  * `pageCount`: `{"pages":3}`.
  * `extractText`: `{"text":["page one", "page two", "page three"]}`.
  * `render`: writes one PNG of the requested page at the requested DPI;
    `{"saved":true}`. Render uses rotated MediaBox dimensions, no crop override.
  * `inspect`: returns the seven feature fields `outlines`, `links`, `annotations`,
    `formFields`, `signatureFields`, `metadata`, `imageCount` shown above.
    A partial inspector may list unavailable fields in `unsupportedFields`; those
    fields are explicitly excluded, not treated as empty lists or successes.
  * `roundtrip`: opens and saves to the requested PDF output; `{"saved":true}`.
* Unadvertised tasks are recorded as `unsupported`, not failures and not scores
  of zero. A provider must advertise only tasks it can execute; document-specific
  rejection of an advertised task is a clean error, not unsupported.

qpdf supports page count and roundtrip; its warning exit code 3 is accepted by
the adapter. Poppler supports count, per-page text, and PNG rendering. Other
providers can implement this protocol without changing the harness.

### anPDF capability-registry adapter (PDFium / PDFOxide)

`bench/providers/anpdf` targets the capability-registry contract (merged in
PR #1; this adapter section was originally written against the pre-merge branch
inspected at `3c2248d`). Set `ANPDF_BIN` to a build of the current tree and
select the content engine explicitly:

```sh
ANPDF_BIN=target/debug/anpdf ANPDF_ENGINE=pdfium \
ANPDF_QPDF="$PWD/build/work/qpdf/bin/qpdf" \
python3 bench/harness.py --corpus build/work/bench-corpus \
  --provider 'pdfium=bench/providers/anpdf --engine pdfium' \
  --provider 'pdfoxide=bench/providers/anpdf --engine pdfoxide' \
  --out build/work/content-engine-results
```

The command's `--engine` overrides `ANPDF_ENGINE`. With neither set, anPDF's own
configured default is used. No fallback to another engine is attempted. Discovery
uses `anpdf operations` and its per-operation `engines` lists. Engines absent from
discovery are unsupported. Canonical `BACKEND_UNAVAILABLE` (or unavailable
operation) responses during execution are also recorded as unsupported, not
crashes. Other errors remain failures; child signals remain crashes.

The adapter copies each input into a fresh private workspace and uses relative
`input.path` / `output.path`. It maps `extract-text` to `extractText` requests
(`pages`, `engine`), batching into at most 100 pages; maps `render-page` to
`renderPage` (`page`, `dpi`, `output`, `engine`); and obtains page count via
`inspect`. Inspection is **qpdf structural inspection regardless of selected
content engine**: its request contract does not accept `engine` and its result
does not expose detailed outlines, links, annotations, fields, signatures or image
counts. The adapter reports only mapped metadata and explicitly marks the other
inspect fields unsupported. Do not attribute its metadata/page-count scores to
PDFium/PDFOxide or interpret them as full feature inspection. There is no roundtrip
mapping in this adapter.

Deployment artifacts include the anPDF executable and any `artifacts` paths
reported at discovery level (or on selected engine descriptor objects, if a
provider extends discovery that way). The inspected registry currently reports
engine IDs only and no library paths: use `ANPDF_ARTIFACTS` (host path-separator
delimited) to declare provider shared libraries or staged deployment directories.
Without those declarations, the size is explicitly incomplete. Provider sessions
need only build/configure their engine, point this adapter at their binary, and
run the same corpus command. This branch does not implement or qualify either
engine itself; no product contract/generated artifacts were changed here.

## Measurements and interpretation

`results.json` retains every repeat's response, status, exit code, time, peak RSS and scores,
including inspect per-field precision/recall and preservation component scores.
It deliberately does not store raw stderr or absolute artifact/input paths.
`report.md` summarizes provider × category and an ALL row with each dimension:

* Correctness: exact page count; text uses Unicode-codepoint Levenshtein distance
  after removing whitespace (no ASCII tokenization), averaged per page. OCR-only
  extraction is recorded but excluded, including clean errors and protocol errors;
  crashes and timeouts still score zero as safety failures. Inspect compares flattened field leaves
  with precision/recall/F1, preserving list order and multiplicity. Render checks
  every page for exact ceiling-rounded pixel dimensions and nonblank RGB variance.
  This is a structural smoke check, **not** pixel/appearance fidelity.
* Malformed: clean rejection or a task-shaped JSON result is success; crash,
  timeout or invalid stdout JSON is failure. Render outputs must be decodable
  images and roundtrip outputs must have a PDF header. Repaired output semantics
  are not correctness-scored or independently certified as valid PDFs.
* Preservation: roundtrip output is reopened with **pypdf, not the provider**.
  Score averages retention recall for features present in the input, plus exact
  page count and normalized text. Feature lists match multisets of canonical
  items, so inserting or reordering entries does not erase unchanged items;
  duplicate multiplicity is retained. Annotation page indices remain significant.
  Outlines are matched per bookmark (title, target page, and ancestor-title path),
  not as whole subtrees: adding a child retains its parent and existing siblings,
  while removing or moving a bookmark loses that bookmark's feature.
  Signed cases additionally require identical
  CMS signature bytes and successful pyHanko integrity verification, using only
  their embedded synthetic root and with network fetching disabled. A rewrite
  that invalidates a signature loses that component even if its field survives.
  This does not establish signer trust, widget visual fidelity, incremental update
  policy, or unsupported exotic feature preservation. Added features are not
  preservation losses; inspect precision penalizes additions separately.
* Performance: monotonic wall time around process creation/execution. Repeats are
  summarized by median; report timing is the median of these run medians. No warm
  in-process engine timings or cache controls are claimed.
* Memory: per-invocation `os.wait4` `ru_maxrss`, Linux KiB converted to bytes,
  macOS already bytes; summary is maximum. Includes Python wrapper overhead and
  waited-for child high-water information, not a sampled sum of concurrent
  descendant RSS. Do not compare it to an in-process allocator metric.
* Deployment: sum of unique resolved files in declared artifact paths (directories
  recurse). Reference adapters declare executables only, **not** their shared
  libraries, fonts, Python runtime or full container closure. Declare a staged
  deployment directory for a meaningful complete-package size comparison.

Unsupported coverage is visible through supported-run counts and capabilities;
do not rank providers on the aggregate score without considering task coverage.
No single weighted winner score is emitted. Error cases score zero for ordinary
inputs. Scoring/oracle errors are explicit in raw samples. Timeout kills the
provider process group. The harness itself is not a sandbox: run trusted adapters
only, on synthetic/local inputs, with OS isolation for adversarial engines.
Oracle parsing of normal roundtrip outputs is in-process and not resource-bounded;
do not use this harness for arbitrary untrusted PDFs or malicious providers.

Linux CI runs these unit tests and the reference comparison, uploading the Markdown
report. This is separate from product qualification and does not qualify macOS
benchmark memory behavior or other providers.
