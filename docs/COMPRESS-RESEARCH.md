# Size-targeted compression for anPDF `compress` — research

Scope: a `maxOutputBytes`-style option (e.g. "below 5 MB") for `compress` on the 0.3.x line (next release: 0.3.1 per repo versioning). Research only — no code changes or PR.

What was decided and shipped, with per-case evidence and limits, is tracked in [COMPRESSION-TECH-REVIEW.md](COMPRESSION-TECH-REVIEW.md) — the living record; new document classes get a case entry there.

## 1. Where `compress` stands today

`src/backend.rs` → `Qpdf::compress` runs:

```
--compress-streams=y --recompress-flate --compression-level=9
--object-streams=generate --remove-unreferenced-resources=yes
[+ lossy: --optimize-images --jpeg-quality=N]
```

All of it is **stream-level**: flate recompress, object streams, drop unreferenced resources, and (lossy) re-encode eligible images to DCT at a quality level. qpdf explicitly does **not resample** — an image stays at its original pixel dimensions; `--optimize-images` only rewrites it when the result is smaller, and only 8-bpc DeviceRGB/Gray(/CMYK) images qualify (`--oi-min-width/height/area` can filter candidates).

Measured on the repo's own bench corpus (`bench/corpus.py`):

| Fixture | Original | Lossless | Lossy (jpeg80) |
|---|---|---|---|
| `images.pdf` (12 × 800×800 px @ ~576 eff. DPI) | 156,527 | 153,166 (−2%) | 132,388 (−15%) |
| `scan.pdf` | 7,150 | 6,807 | 6,807 |
| `text-mixed.pdf` | 2,089 | 1,311 | 1,312 |
| `forms.pdf` | 14,604 | 8,028 | 8,037 |

The image-heavy case is the telling one: its image payload is ~153 KB at ~576 effective DPI. Resampling those images to 150 DPI + JPEG q80 would take the payload to **~13.6 KB — an ~11× cut** — vs the 15% the current implementation achieves. For oversized-image documents, **downsampling is the dominant lever and it is exactly what we don't have.**

## 2. The fine-grained technique catalog (beyond JPEG quality)

Grouped by what part of the file each lever acts on, roughly in the order a pipeline would apply them.

### A. Stream & object structure (lossless, already partially in)

| Technique | Lever | Availability |
|---|---|---|
| Flate recompress at max level | `--compress-streams` + `--recompress-flate` + `--compression-level=9` | qpdf — in |
| Object streams (PDF 1.5) | `--object-streams=generate` | qpdf — in |
| Drop unreferenced objects/resources | `--remove-unreferenced-resources=yes` | qpdf — in |
| LZW / ASCIIHex / RunLength → Flate | happens on qpdf/pikepdf save | qpdf — in |
| Merge duplicate objects & streams (dedup fonts/images) | `mutool clean -ggg/-gggg`, `pdfcpu optimize`, or own object-model pass | not in; dedup is a real win on Java-generated docs (pdfcpu's motivating case: GBs → MBs) |
| Garbage-collect orphaned objects | `mutool clean -g`, PDFOxide `garbage_collect` | not in |
| Remove linearization (hint streams) | rewrite non-linearized | minor; linearize *adds* bytes |
| Stronger deflate | `zopfli` (pure-Rust port exists) instead of zlib-9 | ~2–5% on text streams; cheap |

### B. Raster image pipeline (the dominant lever for most real files)

| Technique | What it does | Codec/tool |
|---|---|---|
| **Downsample to DPI cap** | Decode → resize (Lanczos/bicubic) → re-encode at e.g. 150–300 DPI | mutool `--*-image-subsample-dpi`, Ghostscript `-dDownsample*Images`; **no permissively-licensed OSS tool does this today** (see §5) |
| **Codec re-selection per image** | Re-encode to the cheapest legal filter for that image's character | JPEG (DCTDecode), JPEG2000 (JPXDecode), JBIG2, CCITT G4, Flate |
| **JBIG2 lossless** for bitonal (1-bpp) scans | Symbol/region coding, far better than CCITT G4 on B&W scans | `jbig2enc` (Apache-2.0, C++ on Leptonica BSD-2) — vendorable like qpdf |
| ~~JBIG2 lossy~~ | 3–10× smaller than lossless JBIG2 | **Do not ship.** Proven silent character substitution (Xerox 2013: 6↔8 in scanned numbers); OCRmyPDF removed `--jbig2-lossy` for exactly this reason. For a tool agents run unattended, silently changing digits on a contract is disqualifying. |
| **JPEG2000 (JPXDecode)** | ~20–30% better than JPEG at low bitrates; graceful on noisy scans | `openjp2` crate (pure-Rust OpenJPEG port, BSD-2, active, 274k downloads) — fits the Rust-native architecture; decoders (PDFium/poppler) support JPX |
| **Color/grayscale conversion** | RGB→DeviceGray for B&W-looking pages; drop CMYK ICC weight | image crate |
| **Bit-depth reduction / bilevel conversion** | Threshold grayscale→1-bpp, then JBIG2 | image/leptonica; **content risk on bad scans** (see §4) |
| **Palette/Indexed + quantization** | Color quantize to 256-color indexed | OCRmyPDF -O2/-O3 strategy; image crate supports |
| Drop `/SMask`, `/Mask`, `/Alternates`, thumbnails | Remove soft masks/alternate renditions | object-model pass; destructive to transparency — flag-gated |
| **MRC (mixed raster content)** | Split page into bitonal text mask (JBIG2) + downsampled color background (JPEG/JPX) — what Acrobat "adaptive compression"/DjVu do | needs segmentation (leptonica or custom); phase-2 territory |

### C. Fonts

| Technique | Note |
|---|---|
| **Subset embedded fonts to used glyphs** | Huge on un-subset CJK fonts (multi-MB → KB). fontTools (Python) or harfbuzz-subset/allsorts; Ghostscript/mutool do it automatically |
| Dedup identical embedded fonts | pdfcpu optimize / object dedup |
| Unembed standard-14 fonts | risky — rely on viewer substitution; flag-gated |
| Drop `/ToUnicode` CMaps | saves bytes but breaks text extraction — **do not** in an agent toolkit |

### D. Document-level stripping (semantics — gate behind explicit flags)

| Remove | Cost |
|---|---|
| `/PieceInfo`, thumbnails, OPI comments | safe |
| XMP `/Metadata`, `/Info` fields | conflicts with anPDF's `preservationEvidence` — flag only |
| Structure tree (`/StructTreeRoot`, tagged-PDF data) | accessibility loss — flag only |
| Attachments `/EmbeddedFiles`, OCG layers, `/AA`/`/OpenAction` | visible semantics loss — flag only; actions already reported by inspect |
| Annotations / form flattening | very destructive — anPDF already rejects forms/signed; keep rejecting |

## 3. Choosing mechanisms: a per-type classifier first

The mechanism choice is entirely determined by content mix. Detection uses signals anPDF already has or nearly has:

| Signal | Source |
|---|---|
| Text chars per page | `extractText` (PDFium, already in product) — 0 chars/page ≈ scan |
| Per-image inventory: `/Width /Height /BitsPerComponent /ColorSpace /Filter`, stream bytes | `qpdf --json` object walk (or pdfimages-style listing) |
| Image coverage vs MediaBox (is a page "one big image"?) | content-stream `Do`+`cm` geometry, or PDFium page-object bounds — the one new analysis primitive needed |
| Image byte share (Acrobat "audit space usage") | stream sizes from the same object walk |

### Type → strategy

| Document type | Detect | Primary levers | Avoid |
|---|---|---|---|
| **Born-digital book: text + inline illustrations** | chars/page high; images small-fraction | Lossless structural; downsample illustrations to 150–200 DPI; JPEG/JPX q70–85; font subset if un-subset (esp. CJK) | bilevel anything |
| **Good scanned ebook** | ~0 text; full-page images; JPEG/flate at high DPI | Downsample to 200–300 DPI; if already bitonal → JBIG2 lossless; if grayscale → JPEG/JPX or grayscale+threshold→JBIG2; optional OCR adds a tiny text layer | JPEG quality cuts alone (marginal) |
| **Poor scan (skew, speckle, shadows)** | same as above + gray/color full-page | Keep continuous-tone; downsample to ~150–200 DPI; JPX/JPEG moderate; optional cleanup pass (deskew/despeckle — leptonica, BSD-2) | thresholding to bilevel — destroys shadow/halftone content; JBIG2 lossy — corrupts digits |
| **Scanned ID / passport** | 1–2 pages; color full-page image | Conservative: 300-DPI floor, JPEG q85–90 or JPX; keep color (guilloche/microprint) | bilevel, aggressive downsample, lossy JBIG2 |
| **~100% text + raw TIFF-ish images** | high chars; large Flate/CCITT/LZW images | Re-encode images (CCITT→JBIG2 if bitonal; flate→JPEG/JPX if tone); flate-max text streams; dedup | touching text streams beyond recompress |
| **Vector-heavy docs (figures, drawings)** | images absent; big content streams | flate level 9 / zopfli; object streams; dedup | rasterizing (never silently flatten) |

Cross-cutting: encrypted/form/XFA/signed inputs stay rejected per the preservation rules — a size knob must never imply "flatten to fit."

## 4. Hitting a byte target

A byte target can't be reached analytically — image encoders don't have a closed-form size function — so the design is **audit → plan → execute → verify → descend a knob ladder**:

1. **Audit**: classify the doc; measure per-component bytes (images / content streams / fonts / other objects / xref-overhead). This is also a cheap new `inspect` or `compress`-time primitive.
2. **Non-image floor**: bytes that can't shrink (text content, structure) set the feasibility floor. If `floor ≥ maxOutputBytes`, return `targetMet=false` with achieved bytes — never silently degrade to satisfy the number (matches "never simulate success").
3. **Plan**: allocate the remaining budget across images proportionally; pick per-class DPI cap + codec + quality.
4. **Execute** the planned pass; **measure** actual output.
5. **Descend** an ordered knob ladder if still over target: lower DPI cap → lower quality → grayscaling → (per-type only) bilevel+JBIG2. Each rung is cheap; 2–3 passes converge in practice. Bounded by the job deadline; a `strict` mode fails rather than best-effort.
6. **Report**: `targetMet`, achieved bytes, techniques applied — auditability for agents.

Suggested contract shape (v1 additive, `additionalProperties:false` means it's a schema change anyway):

```
CompressRequest:
  mode, jpegQuality, allowLargerOutput        (existing)
  maxOutputBytes?: int                        (the target)
  onTargetMiss?: "bestEffort"|"error"         (default bestEffort; error → new code, e.g. SIZE_TARGET_UNMET 422)
  targetImageDpi?: int                        (optional cap override)
CompressResult:
  + targetBytes, targetMet, techniques: string[]
```

Version: `0.3.1` (small increment per the agreed versioning).

## 5. Engine/library survey — "more libraries" is right, and licenses decide

| Tool | License | Gives us | Verdict |
|---|---|---|---|
| **qpdf 12.4.2** | Apache-2.0 | All stream/object levers; JPEG re-encode | Keep — baseline for every pass |
| **Rust `image` crate** | MIT/Apache | Decode/resample (Lanczos)/grayscale/threshold | **Core of the missing resample lever** |
| **`zune-jpeg` / `jpeg-encoder`** | MIT/Apache | JPEG encode | with image crate |
| **`openjp2`** | BSD-2 | JPEG2000 encode (JPXDecode legal) | Rust-native JPX — recommended |
| **`jbig2enc`** | Apache-2.0 (Leptonica BSD-2) | Lossless JBIG2 incl. PDF fragments | Vendor binary via `build/get-*.py` pattern; bounded subprocess |
| **Leptonica** | BSD-2 | Deskew/despeckle/adaptive threshold for bad scans | Optional cleanup stage |
| **`zopfli`** | Apache-2.0 | ~3–5% better deflate | optional polish |
| **`lopdf`** | MIT | Object-model edits for in-place stream replace | if we don't drive qpdf/pikepdf-style writes |
| **pdfcpu** | BSD-3 | Resource dedup | Optional; overlap with qpdf is large |
| **PDFOxide** | permissive | `compress` = flate+GC+linearize only; editing APIs may allow stream replace | Evaluate on the PDFOxide line — *not* a resampler today |
| **OCRmyPDF** | MPL-2.0 | Proven scan pipeline (JBIG2 via jbig2enc, quantize, re-encode; OCR). **It does not downsample either** | Optional worker; heavy Python stack (pikepdf, Pillow, tesseract) |
| **pikepdf + Pillow** | MPL-2.0 / HPND | Reference impl of decode→resample→replace (in-place `image.write(jpeg, filter=/DCTDecode)`) | Pattern to port to Rust |
| **Ghostscript** | **AGPL-3.0** | The only turnkey OSS downsampler (distiller presets) | **Excluded from the product** by the AGPL gate in AGENTS.md; also reinterprets the whole file (annotations/forms/structure fidelity risk). Keep as *benchmark baseline* only |
| **mutool (MuPDF)** | **AGPL/commercial** | Closest CLI match: per-class `--*-image-subsample-dpi`, `--image-recompress-method=jpeg|j2k|fax|jbig2`, GC, dedup | Same AGPL block; also a useful benchmark baseline |
| **pdfsizeopt + sam2p** | **GPL-2.0** | multi-pass image optimization pipeline | Reference only — GPL |
| **unpaper** | GPL-2.0 | scan cleanup | Reference only; leptonica covers this ground |
| **PyMuPDF** | AGPL/commercial | — | avoid |
| Commercial (CVISION, NXPowerLite, Acrobat Optimizer) | — | size-target UX precedent: audit panel, per-class DPI/codec settings | feature reference |

**The conclusion that shapes the architecture**: the one lever that matters most — arbitrary image resampling inside a PDF — is today only available in AGPL/GPL tools. anPDF's Apache-2.0 posture (and the AGENTS.md "no AGPL backend without compatibility review" rule) means the resample+re-encode stage should be **implemented natively**: a bounded hidden worker (same `limit_worker` process pattern as `pdfium-worker`) that decodes image streams, resamples/re-encodes via `image` + `zune-jpeg`/`openjp2`/`jbig2enc` subprocess, and replaces stream bytes via a small object-model edit (lopdf) — then a qpdf pass finishes stream/object optimization and preservation reporting.

## 6. Risks & open questions for the build phase

- **SMask/transparency**: naive image replace (the pikepdf recipe) drops `/SMask` → visible change. The worker must handle mask-carrying images or skip them.
- **JBIG2**: ship *lossless generic region* mode only; lossy symbol mode stays off regardless of target pressure.
- **Inline images** (`BI…ID…EI`) need the same treatment as XObjects — qpdf's `--keep-inline-images` matters.
- **Feasibility semantics**: pick `bestEffort` (warn, report floor) vs `error` as contract default — recommend bestEffort + `targetMet:false`; silent content destruction to satisfy the number is the failure mode to design against.
- **Corpus gap**: bench has no bitonal-scan, oversampled-color-scan, un-subset-CJK, or letterhead-TIFF cases — all synthetically generatable per fixture rules; add alongside the feature for measured per-type claims.
- **Effective-DPI measurement** needs image placement geometry — verify pdfium-render page-object bounds at build time, else parse `cm`/`Do` in content streams.

## 7. Suggested build order (for the follow-up implementation ticket)

1. Space-audit primitive (per-component byte accounting inside compress; feeds classification).
2. Rust resample+re-encode worker for JPEG-family images + DPI caps (biggest lever, license-clean).
3. `maxOutputBytes` + ladder + `targetMet` reporting (v1 contract extension; ship as 0.3.1).
4. Lossless JBIG2 via vendored jbig2enc for bitonal scans.
5. Optional: JPX via openjp2, dedup, font subsetting, leptonica cleanup — staged by measured need on the expanded corpus.
