# Changelog

Notable changes follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and Semantic Versioning. Versions below 0.5.0 are development milestones without
SemVer prerelease suffixes, not production-qualified releases.

## [0.3.2] - 2026-11-20

### Added
- Positional CLI shorthand for every PDF operation: `anpdf inspect <doc.pdf>`,
  `anpdf merge <a.pdf> <b.pdf> [...] <out.pdf>`, `anpdf extract <doc.pdf> <pages>
  [-o <out.pdf>]`, `anpdf split <doc.pdf> <out=pages>...`, `anpdf remove-pages
  <doc.pdf> <pages>`, `anpdf compress <doc.pdf> [<out.pdf>]` (output defaults to
  `<name>-compressed.pdf`), `anpdf extract-text <doc.pdf> <pages>` and
  `anpdf render-page <doc.pdf> <page> [-o <image>]`. Shorthand flags cover
  `--output`/`-o`, `--replace`, `--preserve`, `--mode`, `--jpeg-quality`,
  `--level`, `--max-bytes`, `--on-target-miss`, `--allow-larger`, `--engine` and
  `--dpi`; `--` terminates flag parsing for dash-leading filenames. Shorthand
  produces the same contract request and identical result envelopes as
  `--request-json`/`--request-file`.
- `<pages>` shorthand accepts comma lists and ranges (`1,3`, `2-5`), and is
  also available as the `--pages <spec>` flag for extract, remove-pages and
  extract-text.
- Shorthand paths may be absolute or start with `~` (expanded via `$HOME`):
  the invocation's workspace root adapts to the files' common canonical
  directory, so `cd /tmp && anpdf compress ~/Downloads/x.pdf` works. An
  explicit `--root` pins the sandbox and refuses absolute inputs outside it;
  JSON requests, HTTP and MCP keep workspace-relative paths unchanged.
- Error envelopes carry a detail suffix naming the failing field, flag or
  file — e.g. `/input/path: 1 is not of type "string"`, `--level expects
  1, 2 or 3`, `input 'missing.pdf'`. The error schema and exit codes are
  unchanged, and envelopes stay identical across CLI, HTTP and MCP.

### Changed
- A bare `anpdf` or `anpdf <operation>` on a terminal prints command usage and
  exits instead of blocking on stdin; piped stdin still supplies the JSON
  request.
- On a terminal, successful results print pretty-printed JSON and errors
  print `anpdf <command>: <reason>`; piped stdout/stderr keep the canonical
  envelopes byte-identical.
- Parse-stage `INVALID_REQUEST`/`UNSUPPORTED_OPERATION` print usage on a
  terminal; non-terminal stderr keeps the canonical JSON error envelope
  byte-identical.
- Non-UTF-8 arguments fail with `INVALID_REQUEST` instead of panicking.
- `anpdf serve` now fails fast with `INVALID_REQUEST` when `--token-file` is
  missing, at parse time.
- `--help`/`-h` works in any position for known commands (e.g.
  `anpdf compress doc.pdf --help`).
- Shell completions (bash/zsh/fish) cover the new shorthand flags and complete
  filenames for positional arguments.

## [0.3.1] - 2026-11-20

### Added
- Size-targeted compression: `maxOutputBytes` descends a DPI/JPEG-quality
  ladder through a sandboxed Rust resample worker (image XObjects decoded via
  lopdf, resampled with Lanczos3, re-encoded as JPEG) until the target is met
  or the ladder is exhausted. `compressionLevel` 1–3 presets the descent floor
  (1 = lossless only, 2 ≈ 200dpi at high quality, 3 ≈ 120dpi); `onTargetMiss`
  chooses best-effort publication with a `SIZE_TARGET_UNMET` warning or a 422
  failure. Results report `techniques`, `targetBytes` and `targetMet`.
- New synthetic fixtures (`oversampled.pdf`, `bitonal.pdf`) and benchmark
  corpus cases (oversampled color scan, bitonal scan, un-subsetted CJK,
  letterhead) covering the resample paths.

## [0.3.0] - 2026-11-20

### Added
- Capability registry and transport-neutral planning for structural, text and
  rendering backends; consistent engine discovery and canonical errors (PR #1).
- Subprocess-isolated PDFium embedded-text extraction with PDF-space spans and
  PNG rendering across CLI, HTTP and MCP. Pinned binary acquisition and notices
  cover Linux x64/arm64 and macOS arm64 (PR #5).
- Synthetic PDF corpus and external-provider comparison harness, including CJK,
  preservation and malformed-input scoring (PR #2).
- Per-output qpdf preservation reporting for metadata, bookmarks, annotations and
  attachments, optional required aspects, and inspection annotation counts (PR #3).

### Changed
- PDFium is the default text/render engine in standard native, Homebrew and OCI
  release recipes. Bundled-library discovery supports flat tarballs and Cellar
  layouts; `--no-default-features` retains structural-only builds (PR #8).
- qpdf 12.x >=12.4.2 is accepted; only 12.4.2 is qualified, other versions warn,
  and exact-version mode is required in CI/OCI (PR #3).
- Compression reports the bytes actually published and keeps validated original
  bytes when compression would not reduce size unless explicitly overridden (PR #3).

### Fixed
- Full-page PDFium text extraction on offset/rotated pages and UTF-16 decoding of
  supplementary characters; rendering excludes annotations and form widgets (PR #5).
- Privacy checks bind historical notice sanctions to each reaching ref's manifest
  and narrowly reviewed historical/web-merge identity exceptions (PRs #6 and #7).

### Known limitations
- PDFOxide is distributed only as a separate preview tag, not in standard releases.
- No OCR, form filling, signing, existing-text replacement or page-reordering API.
  Transforms reject encrypted, signed, form/XFA-bearing and preservation-unknown
  documents; secondary merge bookmarks/attachments are not merged.
- Text extraction is not OCR; structural success and benchmark scores do not
  certify visual correctness, signature trust, PDF/A, malware safety or production readiness.
- HTTP remains authenticated and loopback-only. Native macOS and linux/arm64 release
  artifacts require tag-time platform and vulnerability/license qualification.
- The Homebrew SHA-256 remains a deliberate placeholder until the v0.3.0 tarball
  is published and independently verified in a follow-up commit.

## [0.2.0] - 2026-11-20

### Added
- Bash, Zsh and Fish completions; macOS arm64 release tarball and Homebrew formula.
- Native multi-architecture linux/amd64 and linux/arm64 OCI release workflow with
  restricted smoke tests, auditable binaries, SBOMs and vulnerability/license audits.

### Changed
- Development milestone versioning without prerelease suffixes below 0.5.0.

[0.3.0]: https://github.com/nextexcite/anPDF/compare/v0.2.0...main
[0.2.0]: https://github.com/nextexcite/anPDF/releases/tag/v0.2.0
