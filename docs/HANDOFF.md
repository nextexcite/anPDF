# Project handoff — anPDF

Canonical status and next-action record for maintainers. Updated 2026-11-23.
This file is the single place for **current state, roadmap, and the prioritized
queue**; the documents it links hold the durable detail. Keep it small: when a
claim here stabilizes or ships, move the evidence into the owning document and
leave only the link.

## 1. Where the project stands

**Baseline:** `0.4.1` staged on `main` (unpublished). Latest published build is
[v0.4.0](https://github.com/nextexcite/anPDF/releases/tag/v0.4.0) — a development
prerelease, not production-qualified. The PDFOxide engine ships separately as
[v0.3.0-pdfoxide-preview.1](https://github.com/nextexcite/anPDF/releases/tag/v0.3.0-pdfoxide-preview.1);
it is an evaluation artifact, not part of standard releases.

Ten contract operations ship across CLI, loopback HTTP, and MCP stdio:
`inspectPdf`, `mergePdf`, `extractPdf`, `splitPdf`, `removePages`, `compressPdf`,
`extractText`, `renderPage`, `encryptPdf`, `decryptPdf` — see
[PROTOCOLS.md](PROTOCOLS.md). Not implemented: page reordering, form fill,
signatures, OCR, existing-text replacement. Structural inspection only; no
visual, cryptographic, PDF/A or malware certification.

| Area | State | Evidence |
|---|---|---|
| Contract + dispatcher | Canonical OpenAPI v1; handwritten Rust adapters; Forge 0.2.0 pinned build-time generation | [FORGE.md](FORGE.md), [ADR-002.md](ADR-002.md) |
| Structural engine | qpdf 12.4.2 qualified; 12.x>=12.4.2 accepted with warning; `ANPDF_QPDF_EXACT=1` in CI/OCI | [PROTOCOLS.md](PROTOCOLS.md) |
| Text/render engine | PDFium `chromium/8165` bundled in standard releases; `--no-default-features` for structural-only | [BUILD.md](BUILD.md#default-content-engine) |
| Compression | Lossless stream passes + image descent (ladder, ~10% fit) + page rasterization on target miss; methods chosen automatically from content evidence | [COMPRESSION-TECH-REVIEW.md](COMPRESSION-TECH-REVIEW.md) |
| Encryption | AES-256 R6 encrypt/decrypt; secrets accepted via echo-off prompts, `chmod 600` files, stdin, and request bodies — `--request-json` refuses literal password fields (argv is machine-readable at launch); backend hands them to qpdf via memfd/private staging, never argv/env/logs | [PROTOCOLS.md](PROTOCOLS.md#password-protection), [SECURITY.md](../SECURITY.md) |
| Release machinery | Multi-arch OCI index (GHCR) + macOS arm64 tarball + Homebrew tap `nextexcite/homebrew-anpdf`; build caching; per-arch SBOM/Grype/license audit gates | [BUILD.md](BUILD.md) |
| Qualification | Per-version evidence and remaining gates | [QUALIFICATION.md](QUALIFICATION.md) |

## 2. Historical vs. current

Read these as dated records, not current claims:

- **Design/experiment docs:** [COMPRESS-RESEARCH.md](COMPRESS-RESEARCH.md) is the
  pre-implementation survey (mechanism catalog, license analysis). Decisions that
  shipped moved to the living [COMPRESSION-TECH-REVIEW.md](COMPRESSION-TECH-REVIEW.md);
  research rows marked *researched, not shipped* remain candidates.
- **Engine evaluation:** PDFium vs PDFOxide ran as the bench comparison; the call
  was PDFium standard / PDFOxide separate preview tag. The bench adapter text
  describing "the unmerged capability-registry contract in PR #1" refers to the
  pre-merge state at `3c2248d`; the registry has long since merged.
- **Qualification log:** [QUALIFICATION.md](QUALIFICATION.md) keeps one section
  per version; sections for superseded versions are historical evidence, and
  only the newest section describes the current baseline.
- **Changelog:** [CHANGELOG.md](../CHANGELOG.md) is append-only history; unreleased
  work sits in the top section.

## 3. Roadmap (revised)

Intention ordering, not commitments. Each phase ships only with the gates in
[QUALIFICATION.md](QUALIFICATION.md).

1. **Consolidate 0.4.x.** Publish staged 0.4.1; keep fixes on 0.4.x — no 0.5.0
   without explicit owner approval. Close the compression untested-class log.
2. **Qualification debt.** linux/arm64 native test run, macOS RSS-limit
   strategy, malformed/fuzz corpus growth, external MCP/HTTP client conformance,
   Forge fidelity gaps, formal OS/backend license sign-off.
3. **Everyday operations.** Page reordering; structural repair depth;
   space-audit primitive inside compress.
4. **Documents and signatures.** AcroForm inspect/fill; visual signatures;
   certificate-based signing/validation as a separate preservation-checked track.
5. **Richer capabilities.** OCR, cautious existing-text editing, optional
   backends, agent/desktop/cloud integration; remote HTTP hosting is its own
   threat-model project.

## 4. Prioritized next actions

Ordered. "Done" means the acceptance criteria hold with execution evidence
linked in the owning doc.

1. **Close installed-binary gaps on the macOS package (release gate).**
   The real Homebrew install path is proven on macOS arm64 CI (`macos-26`
   runner: formula install, `brew test`, bundled renderer, rasterize-backed
   `compress --max-bytes`); the fixture document was synthetic. Still missing:
   any `encrypt`/`decrypt` smoke on the installed binary (formula `test` block
   and `containers/smoke.sh` both skip it) and a human-driven `brew install`
   session log. *Acceptance:* formula test or tap CI exercises encrypt/decrypt
   on the installed build; `smoke.sh` gains an encrypt step; one real
   `brew install anpdf` run logged in QUALIFICATION.md.
2. **Publish v0.4.1** — staged on `main`; requires owner go-ahead, green routine CI
   on the staging commit, and item 1's installed-binary gates closed. The release
   workflow runs [full qualification](BUILD.md#ci-matrix-and-pinned-github-actions)
   on that commit and blocks publication on failure.
   *Acceptance:* tag build green end-to-end; GHCR index + macOS tarball + tap
   formula updated to 0.4.1; artifact/image sizes reported; CHANGELOG section
   dated; release notes name shipped features (v0.4.0's were a generic
   template that omitted `pageRasterization`).
3. **Compression untested classes** (degraded scans, ID/passport scans,
   CCITT/TIFF-dominant docs, mixed traced+live pages). *Acceptance:* one case
   entry per class in COMPRESSION-TECH-REVIEW.md §4 using the §5 template —
   runnable command, measured bytes/time, preservation checks, adopt/skip/revisit.
4. **Forge fidelity and redistributable artifact** — probes pass but listed gaps
   stand; a redistributable artifact is a release gate. *Acceptance:* gap list
   in FORGE.md shrinks or is waived per-item with rationale.
5. **External-client conformance** — MCP/HTTP exercised only by the in-repo
   harness today. *Acceptance:* a third-party MCP client and an HTTP client run
   the documented sequences without drift; results logged in QUALIFICATION.md.
6. **linux/arm64 + macOS native runs** — the native arm64 *release smoke*
   passed (real `ubuntu-26.04-arm` execution, not emulation), but the full
   integration corpus has never run natively on arm64; macOS RSS enforcement
   is a `proc_pidinfo` watchdog, not `RLIMIT_AS`. *Acceptance:* full suite
   runs natively on both; QUALIFICATION.md updated.
7. **License sign-off + advisory tracking** — 155-component inventory
   reconciled, formal review open; v0.4.0 Grype reports 78 High + 64 Medium
   *non-fixable* advisories under a fixable-only gate. *Acceptance:* signed-off
   manifest diff for new deps since 0.4.0; non-fixable advisory count recorded
   per release.
8. **Per-release performance evidence** — CI captures only the qpdf/poppler
   reference comparison; no PDFium/PDFOxide product-engine numbers exist for
   v0.4.0. *Acceptance:* `bench/harness.py` run with `ANPDF_BIN`/`ANPDF_ENGINE`
   against the release binary, `results.json` archived, rasterize cost measured
   (currently qualitative only: "order of a minute").

### Interface-parity and metadata gaps (architecture audit, verified against `main` @ `3e09d75`)

Recorded as plan items, not implementation changes. Each is a decision plus a
small contract/doc touch-up; implement deliberately, not opportunistically.

9. **`--dry-run` parity decision.** CLI-only today; HTTP/MCP dispatch
   `dry_run=false`. AGENTS.md rule 1 wants equivalent dry-run behavior across
   surfaces; the contract has no `dryRun` schema and the response bypasses
   `valid_output`. *Acceptance:* either `dryRun` is added to request schemas for
   HTTP/MCP parity, or AGENTS.md/PROTOCOLS.md scope it explicitly as CLI-only —
   one statement of the rule, no drift.
10. **Discovery can't disclose resampler/rasterizer availability.** `engines`
   lists text/render providers; a client learns `pageRasterization` didn't run
   only at miss time via `IMAGE_RESAMPLE_LIMITED`. *Acceptance:* discovery
   surfaces resample-worker/rasterizer presence (e.g. a compress `techniques`/
   `engines` field), or the omission is waived with rationale in PROTOCOLS.md.
11. **Deadline ceiling vs per-operation defaults.** Discovery advertises
    `wallSeconds:600`; `Control::default()` is 60s and compress scales 60→600.
    The advertised number is a ceiling, not a per-op bound. *Acceptance:*
    discovery output or PROTOCOLS.md states the ceiling semantics; no false
    per-operation claim.
12. **Stale contract metadata (tracked follow-up, not in this docs PR).**
    `x-anpdf-status` still reads "development 0.3.0 contract" and the Dockerfile
    `APP_VERSION` default is `0.3.0`; `contractVersion:1.0.0` is a schema
    version, not the product version, and docs should say so. Updating them
    alters the published schema/hash surface, so it rides the next contract
    touch. *Acceptance:* both corrected with the next contract change, and
    PROTOCOLS.md explains `contractVersion` semantics.

Audit verified-consistent (no action): ContentEngine length bound is 32 on both
schema and registry; HTTP `Host` pinning and the pinned MCP `protocolVersion`
behave as documented; the `validate` CLI alias is cosmetic.

## 5. Keeping this record current

- **One fact, one home.** Version = `Cargo.toml` + CHANGELOG; operations =
  `contracts/v1/openapi.json`; shipped mechanisms = owning doc
  (PROTOCOLS/BUILD/tech-review); status and queue = this file. Link, don't copy.
- **New document class or failure** → a §4 case in
  [COMPRESSION-TECH-REVIEW.md](COMPRESSION-TECH-REVIEW.md), not here.
- **New release** → update CHANGELOG; add a QUALIFICATION.md section and mark
  the old one historical; bump the baseline line in §1 here.
- **Decision changes** (engine picks, versioning, gates) → edit this file's §3/§4
  and the owning doc in the same commit; never leave two versions of a decision.
- **Never commit** real documents, private filenames, host paths, secrets or
  derived data — fixtures stay synthetic per AGENTS.md and fixture rules.
