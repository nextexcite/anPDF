# Compression technical review — living record

Purpose: one durable place to record what each compression case taught us —
the evidence used to classify content, the candidates tried, what shipped, what
was rejected, and the limits we accepted. Every new document class or failure
gets its own entry under §4 using the template in §5. Companion to
`COMPRESS-RESEARCH.md` (the pre-implementation survey; this file records
decisions, that one lists possibilities).

## 1. Shipped baseline (main)

`compress --max-bytes <N>` (lossy) today runs two passes inside a bounded,
isolated worker:

1. **Image descent.** Each embedded image is decoded once, resized and
   trial-encoded at every rung of a DPI/quality ladder plus interpolated
   midpoints (bounded parallelism); the gentlest rung fitting under target is
   applied in a single JPEG generation. On a deadline or an unreachable target,
   the smallest result measured is published as best effort with
   `SIZE_TARGET_UNMET`.
2. **Page rasterization on miss.** When the descent still exceeds target, pages
   carrying **positive evidence of traced vector art** are re-rendered to a JPEG
   and their `Contents`/`Resources` rebuilt in place at the gentlest fitting
   rung; the technique is reported as `pageRasterization` with a
   `PAGE_RASTERIZED` warning (fine zoom detail lost on rebuilt pages). Pages
   without that evidence — or any uncertainty — retain their original objects
   byte-identical.

Both passes respect the job deadline, which scales with input size, and run
under the same worker isolation (memory cap, CPU bound, deadline) as resample.

## 2. Decisions and counterexamples

| Case | Evidence | Chosen mechanism | Rejected / deferred | Why |
|---|---|---|---|---|
| Scanned pages, large image payload | image bytes dominate; resample reaches target | image descent (per-image resample + re-encode) | — | dominant lever; shipped 0.3.x |
| Traced-vector pages (text drawn as paths, ~1e3 paint ops/page) | dense path ops, no text ops, no annots, no layer/blend | whole-page rasterization in place | path simplification, stream recompression | non-image streams dominate → no image lever reaches target |
| Page with live text/link + photo + line art | text ops / Annots present | photo resampled; text, link, line art untouched | whole-page rasterize | searchable content must not be discarded |
| Tagged/structured document | `MarkInfo`/`StructTreeRoot`/`OCProperties` present | bail entirely | partial rasterize | semantics would be silently dropped |
| Rotated / shifted / cropped page | `Rotate != 0`, nonzero `MediaBox` origin, `CropBox != MediaBox` | keep original objects | offset/transform math | correctness beats coverage — placement would clip or double-rotate |
| `Do` to Image XObject | resolves to `/Subtype /Image` | eligible alongside path evidence | — | already pixels |
| `Do` to Form XObject | nested content scanned | eligible only when no text/layered ops inside | blind rasterize | nested live text would be flattened |
| Undecodable content | `Content::decode` fails | keep original objects | rasterize anyway | cannot prove no live text |
| Content stream shared by rasterized + retained pages | stream ref appears on both | stream survives removal | unconditional delete | dangling reference on retained page |
| Indirect `Contents` array (reference → array of streams) | ref resolves to `Object::Array` | resolve into stream refs | treat as single ref | inner streams escaped tracking |

Rule learned: classification needs **positive evidence of the thing you want to
destroy**, not absence of evidence of what you want to keep.

## 3. Granularity ladder — alternatives and limits

| Level | Mechanisms | State | Limits |
|---|---|---|---|
| Stream/object | flate recompress, object streams, drop unreferenced, dedup, zopfli | shipped (lossless tier) | bounded single-digit % on stream-dominated docs |
| Image | DPI cap + JPEG re-encode, codec re-selection, gray conversion | shipped | only resamples what resample sees; ~10%-under-target landing; one JPEG generation keeps quality but bounds the ladder |
| Region | recompress photo region / isolated traced group inside a mixed page | **not attempted** | unsafe separation: layering, clip/blend semantics, shared resources — retained instead (revisit per case) |
| Page | render traced pages to raster at rung DPI/quality | shipped | zoom detail capped at render DPI; whole-page granularity — mixed pages are skipped, not split |
| Document | strip metadata/tags/annots/attachments | not shipped | destructive to semantics; flag-gated only |

Document-level invariants that every path must respect: page count, page box
geometry, annotations/links/forms/signatures, tagged structure, attachments,
and extractable text survive untouched unless the page itself was positively
classified and rebuilt — and rebuilt pages keep their box and page identity.

## 4. Case log

One entry per document class or reported failure, using §5's template.

### Case 0 — traced-vector document misses a byte target (the trigger case)

- **Class:** pages drawn as dense path ops; effectively no resamplable images.
- **Baseline:** stock descent published ~input size with `SIZE_TARGET_UNMET` —
  a correct best-effort, but useless to the user.
- **Candidate:** page rasterization gated on traced evidence.
- **Result (synthetic `traced.pdf` fixture):** 2 pages @ target 50,000B →
  41,239B, `targetMet`, `pageRasterization` + `PAGE_RASTERIZED`.
- **Preservation checks:** page count/box intact; mixed fixture's retained page
  kept its annotation and extractable text anchor.
- **Adopted:** yes, automatic on descent miss.
- **Known limits:** whole-page granularity; zoom fidelity bounded by rung DPI;
  elapsed ~60 s on a large traced doc (deadline-bounded).

## 5. Case template (copy per new class)

```
### Case N — <document class or reported behavior>

- Class: <content profile; how you proved it (operator histogram, image bytes,
  stream inventory — never guesses from size alone)>
- Fixture: <synthetic generator change; privacy-safe content only>
- Baseline: <command>, <input bytes> → <bytes>, <time>, warnings, techniques
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
count).

## 6. Standing criteria

- **Adopt** when the mechanism is evidence-gated, preserves document semantics,
  lands gentlest-fit under target, and its limits are stated plainly.
- **Skip** when separation cannot be proven safe (mixed pages, layered content,
  undecodable streams, uncertain box/rotation) — retention is the correct
  behavior, not a gap.
- **Revisit** when a skipped class recurs in real reports and a safe isolation
  test exists (e.g. a Form XObject provably separable from its page).
