# Changelog

Notable changes follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and Semantic Versioning. Versions below 0.5.0 are development milestones without
SemVer prerelease suffixes, not production-qualified releases.

## [0.4.1] - Unreleased

### Changed
- A `--max-bytes` miss now reports what page rasterization actually did:
  `IMAGE_RESAMPLE_LIMITED` states whether the pass ran, was skipped because
  no PDFium renderer is available (e.g. a package that omits the shared
  library), exceeded the deadline, found no eligible traced-graphics pages,
  or was skipped because the document carries attachments. Raster-pass
  worker errors degrade to best-effort output plus a named reason instead
  of aborting the compression.
- Terminal progress during `compress` reports measured work, not
  predictions: lossless-pass bytes vs the target, each applied rung's
  finished bytes, rasterize eligibility and rebuilt-byte totals, and the
  verified final size.

## [0.4.0] - 2026-11-23

### Added
- Password protection: `encryptPdf` and `decryptPdf` operations (CLI
  `anpdf encrypt` / `anpdf decrypt`, HTTP `/v1/encrypt` / `/v1/decrypt`,
  MCP tools) apply and remove standard AES-256 encryption (PDF revision 6)
  via the qualified qpdf backend. Encrypt requires a non-empty user
  password and accepts an optional `ownerPassword`; when omitted, a random
  unguessable owner password is generated — never an empty one. Decrypt
  requires a valid user or owner password; a wrong password fails as
  `INVALID_PASSWORD`, decrypting an unencrypted input fails as
  `INVALID_REQUEST`, and any other operation on an encrypted input still
  fails as `ENCRYPTED_UNSUPPORTED`.
- Secrets never enter command arguments, environment, logs or errors. On a
  terminal, `anpdf encrypt`/`decrypt` prompt interactively with echo
  disabled (a blank owner answer generates one); scripts use
  `--password-file <f>` / `--owner-password-file <f>` (must be `chmod 600`),
  `--request-file` or piped stdin carrying the JSON `password` /
  `ownerPassword` fields. `--request-json` refuses literal password fields
  — argv is machine-readable. HTTP and MCP requests carry the fields in
  the body only; no filesystem path fields for secrets exist. The backend
  hands secrets to qpdf through an inherited anonymous descriptor (memfd)
  or a private staging file, never argv.
- Positional shorthand for both operations: `anpdf encrypt <doc.pdf>
  [<out.pdf>]` (default `<name>-encrypted.pdf`) and `anpdf decrypt
  <doc.pdf> [<out.pdf>]` (default `<name>-decrypted.pdf`).
- Inspect reports `encrypted: true` on protected inputs; encrypted outputs
  are re-inspected with the new password so the returned page count, forms
  and signatures status are verified after the rewrite, and
  `preservationEvidence` hashing works through encryption.
- New canonical error `INVALID_PASSWORD` (exit 3, HTTP 403).

## [0.3.4] - 2026-11-21

### Changed
- OCI image: advanced the pinned Debian snapshot to pick up glibc
  deb13u5.

## [0.3.3] - 2026-11-20

### Added
- Terminal progress reporting and human-readable byte sizes in CLI output;
  the argument parser is rebuilt on clap.
- Size-targeted compression improvements: the resample worker decodes each
  image once, measures every ladder rung and applies the gentlest rung
  that fits (~10% of target); worker memory is bounded by a per-worker
  /proc VmRSS poll under its own AS limit; deadlines scale with input
  size and the best-so-far candidate is published on exhaustion.
- `inspectPdf` parses the object graph shallowly by default; `deepCheck:
  true` opts into complete qpdf stream validation. PDF-writing operations
  keep the deep check on their inputs.

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
- On a terminal, successful results print a short plain-language summary
  (output path, sizes and saving, warnings in words) and errors lead with
  the specific reason (`anpdf <command>: <reason>`); `--json` prints the
  envelope on a terminal, and piped stdout/stderr keep the canonical
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
