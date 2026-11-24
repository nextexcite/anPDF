# Compression technical review — living record

Purpose: one durable place to record what each compression case taught us —
the evidence used to classify content, the candidates tried, what shipped, what
was rejected, and the limits we accepted. Every new document class or failure
gets its own entry under §4 using the template in §5. Companion to
`COMPRESS-RESEARCH.md` (the pre-implementation survey; this file records
decisions, that one lists possibilities).

Versioning of claims below: **released v0.3.4** covers the scaled deadline,
decode-once worker, exact rung bytes with interpolated midpoints, and
best-effort publish on a miss. **Released v0.4.0:** automatic page
rasterization on a descent miss (technique `pageRasterization`).
**Released v0.4.1:** raster-pass outcome reporting and measured terminal progress.
**Released v0.4.4:** explicit lossless mode never enters the lossy descent, even with a target. Unless a claim says otherwise, mechanisms listed
as shipped are invoked code, not proposals.

## 1. What runs today

`compress --mode lossy --max-bytes <N>` can invoke the following bounded worker passes. A bare target can still enable them for v1 compatibility; an explicit lossless request cannot.

1. **Image descent.** Each embedded image is decoded once, resized and
   trial-encoded at every rung of a DPI/quality ladder plus interpolated
   midpoints (bounded parallelism); the gentlest rung fitting under target is
   applied in a single JPEG generation. RGB images whose channels never diverge
   are detected achromatic and re-encoded as grayscale JPEG
   (`achromatic_to_gray` in `src/resample.rs`). On a deadline or an unreachable
   target, best effort can publish with `SIZE_TARGET_UNMET`; a strict target miss publishes nothing.
2. **Page rasterization on miss.** When the descent still exceeds
   target, pages carrying **positive evidence of traced vector art** are
   re-rendered to a JPEG and their `Contents`/`Resources` rebuilt in place at
   the gentlest fitting rung; the technique is reported as
   `pageRasterization` with a `PAGE_RASTERIZED` warning. Lossless qpdf stream and object rewrites are separate from lossy image optimization. The opt-in `layered` mode uses a fixed 400 DPI rather than this descent; see [protocols](PROTOCOLS.md#preservation-and-compression).

Both passes respect the job deadline, which scales with input size
(`compress_deadline_targeted` in `src/backend.rs`), and run under the worker's
memory/CPU caps.

## 2. Decisions and counterexamples

| Case | Evidence | Chosen mechanism | Rejected / deferred | Why |
|---|---|---|---|---|
| Scanned pages, large image payload | image bytes dominate; resample reaches target | image descent (per-image resample + JPEG re-encode) | — | dominant lever; shipped |
| Traced-vector pages (text drawn as paths, ~1e3 paint ops/page) | dense path ops, no text ops, no annots, no layer/blend | whole-page rasterization in place | path simplification, stream recompression | non-image streams dominate → no image lever reaches target |
| Page with live text/link + photo + line art | text ops / Annots present | photo resampled by descent; raster pass retains the page untouched | whole-page rasterize | searchable content must not be discarded |
| Tagged/structured document | `MarkInfo`/`StructTreeRoot`/`OCProperties` present | raster pass returns `protected-structure`; descent may still run | partial rasterize | semantics would be silently dropped |
| Rotated / shifted / cropped page | Effective (including inherited) rotation is nonzero, `MediaBox` origin is nonzero, or `CropBox` differs from `MediaBox` | raster pass retains original objects | offset/transform math | correctness beats coverage — placement would clip or double-rotate |
| `Do` to Image XObject | resolves to `/Subtype /Image` | eligible alongside path evidence | — | already pixels |
| `Do` to Form XObject | nested content decoded and scanned | eligible only when no text/layered ops inside | blind rasterize | nested live text would be flattened |
| Undecodable content | `Content::decode` fails | retained | rasterize anyway | cannot prove no live text |
| Content stream shared by rasterized + retained pages | stream ref appears on both | stream survives removal | unconditional delete | dangling reference on retained page |
| Indirect `Contents` array (reference → array of streams) | ref resolves to `Object::Array` | resolve into stream refs | treat as single ref | inner streams escaped tracking |

Rule learned: classification needs **positive evidence of the thing you want to
destroy**, not absence of evidence of what you want to keep.

## 3. Granularity ladder — alternatives and limits

| Level | Mechanisms | State | Limits |
|---|---|---|---|
| Stream/object | flate recompress, object streams, drop unreferenced resources | shipped (all builds; lossless tier) | bounded single-digit % on stream-dominated docs |
| Stream/object | object/stream dedup, zopfli re-deflate | researched, **not shipped** | small gains; extra engine weight |
| Image | DPI cap + JPEG re-encode; achromatic→grayscale | shipped | only resamples images the descent sees; one JPEG generation bounds quality floor |
| Image | codec re-selection (JPX/JBIG2/CCITT), palette quantization | researched, **not shipped** | engine additions; JBIG2-lossy permanently excluded (silent glyph substitution) |
| Region | recompress photo region or isolated traced group inside a mixed page | **not attempted** | unsafe separation: layering, clip/blend, shared resources — retained instead; revisit per case |
| Page | render traced pages to raster at rung DPI/quality | on main | zoom detail capped at render DPI; whole-page granularity — mixed pages are skipped, not split |
| Document | strip metadata/tags/annots/attachments | **not shipped** | destructive to semantics; would need an explicit opt-in |

Preservation scope, stated exactly: "bail entirely" and "retains original
objects" describe the **rasterization pass** — a retained page's `Contents`
and `Resources` objects are byte-identical after that pass. The lossless
rewrite and image descent around it still recompress streams and resample
images per their own existing behavior; the final file is never claimed
byte-identical to the input.

## 4. Case log

One entry per document class or reported failure, using §5's template.

### Case 0 — traced-vector document misses a byte target (the trigger case)

- **Class:** pages drawn as dense path ops; effectively no resamplable images.
- **Fixture:** synthetic `traced.pdf` — produced by `python3 fixtures/generate.py`
  (`traced_pdf(2)`: two pages, ~900 painted path ops each, no text ops).
- **Baseline recipe (executable):** `anpdf compress traced.pdf -o out.pdf
  --max-bytes 50000` — pre-fix code published ~input size with
  `SIZE_TARGET_UNMET` (correct best-effort, useless to the user).
- **Historical result (original A4 fixture, before the layout fix below):**
  41,239 bytes with `targetMet` and `PAGE_RASTERIZED`. With the layout fix,
  a repeat on that A4 fixture returned 61,853 bytes with `SIZE_TARGET_UNMET`;
  the raster pass hit its deadline. The compact fixture in Case 2 exercises
  successful rasterization, not the original A4 byte target.
- **Preservation checks:** page count/box intact; in `mixed-elements.pdf` the
  retained page kept its link annotation and `LIVE-TEXT-ANCHOR` extractable
  text while its photo was resampled by the descent.
- **Adopted:** yes, automatic on descent miss (main).
- **Known limits:** whole-page granularity; zoom fidelity bounded by rung DPI;
  elapsed on the order of a minute for a large traced doc (deadline-bounded).

### Case 1 — distributed package without the renderer silently skips rasterization

- **Class:** environment regression, not a content class — the macOS package
  installed without `libpdfium` (the v0.4.0 Homebrew formula omitted the
  bundled `libpdfium.dylib`).
- **Observed:** `compress --max-bytes` on traced content missed the target
  with `SIZE_TARGET_UNMET` + `IMAGE_RESAMPLE_LIMITED` claiming
  `pageRasterization` "reduces them further" — the pass never ran because
  `resolve_pdfium` found no library, so `plan.rasterizer` was `None` and the
  gate skipped it silently. A rasterize worker error would likewise have been
  invisible: `TIMEOUT` was swallowed and other codes aborted the whole job.
- **Verified builds:** reproduced on the local release build with
  `ANPDF_PDFIUM_LIB` unset — identical miss warnings, rasterize never
  attempted; with the library present, the same fixture rasterizes and meets
  target. Not yet proven: the literal macOS arm64 tarball (this environment
  is Linux; the package regression test simulates absence via env).
- **Adopted fix (0.4.1):** a miss now names what the pass did —
  `skipped: no PDFium renderer is available` / `exceeded the processing
  deadline` / `the renderer failed (<code>)` / `no eligible traced-graphics
  pages` / `document carries attachments` — inside `IMAGE_RESAMPLE_LIMITED`,
  and raster worker errors degrade to best-effort + the named reason instead
  of aborting. Terminal progress shows measured candidate bytes per step.
- **Lesson:** gating a pass on an optional capability must surface the gate's
  outcome; a suggestion that reads like an available action is a bug report
  waiting to happen.

### Case 2 — rasterization shrinks page content into a corner

- **Fixture:** `traced_pdf(2)` in `fixtures/generate.py`, now on 238×337 pt
  pages to keep the test fast without reducing the traced stream's complexity.
- **Recipe:** `anpdf compress traced.pdf out.pdf --max-bytes 50000`, with
  qpdf 12.4.2 and PDFium `chromium/8165` on Linux amd64.
- **Baseline:** 160,216 → 47,427 bytes; `targetMet` was true, but the ink on
  page 1 occupied only 57×75 pixels of a 238×337 render at 72 DPI. PDFium's
  matrix render path was given a larger bitmap without a matching transform.
- **Fixed:** 44,786 bytes, still under target, with the page 1 ink bounds
  restored to the original `(0, 26, 238, 337)` (Poppler, 72 DPI). `qpdf --check`
  found no syntax or stream errors. The integration test checks ink placement
  on both pages in addition to page count and MediaBox dimensions.
- **Verdict:** use the same scaled render for measurement and assembly, and
  for `render-page`. Geometry is preserved on the fixture; JPEG remains lossy.

### Untested classes — next entries needed

From the original classification survey, still without a case:

- **Degraded scans** — skew, bleed-through, shadows, noise (the "awful scan
  quality" profile): resample quality/legibility thresholds unknown; bilevel
  conversion flagged content-risk in the research.
- **IDs/passports** — small-format scans where every glyph matters; rasterize
  DPI floor and legibility checks need explicit thresholds.
- **Text-dominant docs with TIFF/PS-derived bitmaps** — monochrome or CCITT
  sources: codec re-selection (CCITT G4 / JBIG2 lossless) is the likely lever,
  not shipped.

**Existing mixed-page coverage:** `mixed_page_pdf()` in `fixtures/generate.py` pairs a traced-only page with a second page containing a photo, live text, a link and sparse line art. The full `tests/integration.py` corpus checks rasterization and skip warnings, surviving selectable text and the annotation count. This does not establish region-level compression or cover every dense traced/text combination.

## 5. Case template (copy per new class)

```
### Case N — <document class or reported behavior>

- Class: <content profile; how you proved it (operator histogram, image bytes,
  stream inventory — never guesses from size alone)>
- Fixture: <generator change in fixtures/generate.py; synthetic content only>
- Baseline recipe: <exact command>, <input bytes> → <bytes>, <time>, warnings
- Candidate(s): <mechanism + where in the granularity ladder>
- Result: <bytes vs target>, <time per phase if relevant>
- Legibility: <100% crop comparison — small text / photo / fine strokes>
- Preservation: <inspect: page count, boxes, annots/links; text extraction;
  forms/signatures/tags; structural validity>
- Verdict: adopt / skip / revisit — <reason and conditions to revisit>
- Follow-ups: <open risks, thresholds to tune>
```

Fixture rules: synthetic only (generator in `fixtures/generate.py`); never
commit real documents, filenames, outputs or crops derived from them; keep
cases small enough that the suite stays fast (rasterize work scales with page
count). A fixture name alone is not a case — link the generator function and
give the runnable command.

## 6. Standing criteria

- **Adopt** when the mechanism is evidence-gated, preserves document semantics,
  lands gentlest-fit under target, and its limits are stated plainly.
- **Skip** when separation cannot be proven safe (mixed pages, layered content,
  undecodable streams, uncertain box/rotation) — retention is the correct
  behavior, not a gap.
- **Revisit** when a skipped class recurs in real reports and a safe isolation
  test exists (e.g. a Form XObject provably separable from its page).
