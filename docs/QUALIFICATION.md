# Preliminary qualification evidence

Only the newest section describes the current baseline; older sections are
historical evidence kept for audit. Status and the prioritized queue live in
[HANDOFF.md](HANDOFF.md).

## 0.5.0

[v0.5.0](https://github.com/nextexcite/anPDF/releases/tag/v0.5.0) was published on 2026-11-25 as a prerelease/development milestone, not production qualification. Tag `v0.5.0` points at merge commit `76ae8214e7a0cd4f897372cc8306d139ed977a48`. Release workflow run [45629808514](https://github.com/nextexcite/anPDF/actions/runs/45629808514) validated the tag/version, reran qualification on that exact tree, and only then built and published artifacts. Every qualification lane completed successfully: `native`, `pdfium (ubuntu-26.04)`, `pdfium (ubuntu-26.04-arm)`, `pdfium (macos-26)`, `oci-smoke`, `forge-output`, and `privacy`. This is the first exact-release-tree hosted native Linux arm64 full-corpus evidence; the macOS lane also exercised the hosted unit suite containing the RSS-limit test.

Before tagging, the candidate branch passed locally on macOS arm64: format, Clippy, 41 unit tests, debug build, pinned Forge regeneration/probes, the official TypeScript MCP client probe (11 tools), and 154 cross-surface integration cases across CLI, HTTP and MCP. That local run included input preservation, Poppler page-order verification, deterministic input/output filesystem-race rejection, MCP cancellation/EOF worker cleanup, raw HTTP disconnect cancellation, malformed-input/compression-bomb cases, and deterministic pinned-backend byte-for-byte structural goldens for merge, extract/subset, remove-pages, and three reorder permutations. Manual hosted validation run [45614123002](https://github.com/nextexcite/anPDF/actions/runs/45614123002) also passed every lane at merge commit `d5e406a`; it predates the release-candidate commit, so the exact release evidence is run 45629808514.

### Published artifact verification

- GHCR multi-architecture index: `ghcr.io/nextexcite/anpdf@sha256:57e46fbee5d146b0f4cb757271621dacffc9b23449849ed22a40a300efdd9850`.
  - linux/amd64 build digest: `sha256:087f18dfd6acb84a483a85de93aca1a20c891ff78294b7839324d3d119190533`.
  - linux/arm64 build digest: `sha256:c4736cc2b313279818ad9dedc8a6ce91e1aefbb38fc07717bccc31ae3f1d2fd1`.
  - The release workflow pulled each per-architecture published digest and repeated the restricted smoke test before creating the index, then inspected, pulled, and smoked the multi-architecture index.
  - Locally on Apple Silicon, both published per-architecture digests pulled successfully. The exact linux/arm64 digest also passed merge, PDFium text extraction, PDFium rendering, and password round-trip checks with networking disabled, a read-only root filesystem, all capabilities dropped, `no-new-privileges`, pid/memory/CPU limits, and a non-root image user. The local Podman compatibility layer does not support the Docker tmpfs `uid`/`gid` mount options used by `containers/smoke.sh`, so that supplemental local run used a writable bind-mounted output directory; the hosted workflow remains the exact smoke evidence.
- macOS arm64 tarball: `anpdf-0.5.0-macos-arm64.tar.gz`, 6,598,852 bytes, SHA-256 `7e9f38935186b4a12ed13398fc0127a843d193b2c8542d1df36a5222476e817e`. The downloaded published asset matched the published `SHA256SUMS-macos-arm64.txt`.
- On Apple Silicon, the extracted published tarball reported `anpdf 0.5.0`; reordering `3,1,2` produced text order `PAGE-C`, `PAGE-A`, `PAGE-B`; PDFium extraction returned `PAGE-A`; and `build/smoke-protection.sh` passed with qpdf confirming that the encrypted file required a password and the decrypted copy retained `PAGE-ONE`.
- Homebrew tap dispatch run [45633961394](https://github.com/nextexcite/homebrew-anpdf/actions/runs/45633961394) succeeded; tap commit `6bb981b70` (`update anpdf to v0.5.0`) points at the same tarball URL and SHA-256. A local `brew upgrade anpdf` installed 0.5.0 and `brew test anpdf` passed.
- Release assets include per-architecture SPDX SBOMs, Grype reports, OCI audit reports, digest files, checksum files, the macOS tarball, and the macOS checksum file. The downloaded SPDX, Grype, audit, per-architecture digest, and index digest files all matched their published SHA-256 checksum files.

### Exact OCI license/advisory audit

The 0.5.0 release workflow regenerated scans from the exact release images before push. Passing the policy gate is not a vulnerability-free claim.

| Architecture | SBOM packages | Debian entries | Rust crates | Missing expected Debian packages | Unexpected Rust crates | Disallowed licenses | Fixable High/Critical |
| --- | ---: | ---: | ---: | --- | --- | --- | --- |
| linux/amd64 | 269 | 81 | 186 | 0 | 0 | 0 | 0 |
| linux/arm64 | 267 | 81 | 184 | 0 | 0 | 0 | 0 |

Both architectures recorded 53 distinct Debian license expressions and the same Grype severity totals: 81 High, 64 Medium, 10 Low, 49 Negligible, and 6 Unknown. By fix state for High/Medium: 27 High and 12 Medium were `not-fixed`; 54 High and 52 Medium were `wont-fix`. One `fixed/Unknown` match was also recorded. The detailed per-architecture reports are published as `oci-audit-report-linux-amd64.json` and `oci-audit-report-linux-arm64.json`.

### Engineering license inventory

- Apart from the intentional root package version edits, the dependency and packaging inputs were unchanged from v0.4.4: Cargo dependency resolution, `containers/Dockerfile`, the qpdf/PDFium/Forge pins, `license-manifest.json`, and `THIRD_PARTY_LICENSES` are byte-identical.
- `build/licenses.py --check` passed for the default graph (196 native Rust components) and for the all-features inventory basis with `pdfoxide` (287 native Rust components). The Rust expressions resolve only to the reviewed permissive set already represented in `license-manifest.json`; legal notice hashes are retained in `THIRD_PARTY_LICENSES`.
- qpdf 12.4.2 is Apache-2.0. Standard releases dynamically load the pinned `pdfium-binaries` chromium/8165 archive and retain its upstream notice bundle. The OCI image copies the qpdf, PDFium, Rust and Debian copyright/notice files.
- Debian copyright metadata includes GPL/LGPL-family expressions from the base runtime package set. The packages are unmodified Debian binaries with their copyright files retained, but this engineering inventory is not formal legal sign-off or a copyleft/source-offer opinion.
- Build-time TypeScript qualification uses 36 locked npm package entries: 24 Apache-2.0, 10 MIT, and 2 ISC. Node/npm, Forge, TypeScript, MCP client/server, and Cap'n Web packages are build-time qualification tools, not product runtime dependencies or embedded release components.

## 0.4.4

[v0.4.4](https://github.com/nextexcite/anPDF/releases/tag/v0.4.4) was published on 2026-11-24 after the [release workflow](https://github.com/nextexcite/anPDF/actions/runs/45366810966) passed. The GHCR `0.4.4` index digest was `sha256:41081c3e31c8ae12f529e51f32c6312e2dc808b92bd91df7fca33b4e3614f207`; it contains the release builds for Linux amd64 and arm64. The downloaded macOS arm64 tarball, published checksum, and updated Homebrew formula agreed on SHA-256 `9c85e6c97ad4b6dce2f4e4f75a882bcb70e31f86d79c7d3e254f893dbd2f3a46`.

On Apple Silicon, a local 0.4.4 release tarball passed bundled PDFium and
password-protection smoke tests. `brew install` and `brew test` passed using a
local copy of the public tap formula pointed at that tarball. The Homebrew-
installed executable also passed `build/smoke-protection.sh` with Homebrew's
qpdf 12.4.2 on PATH and no `ANPDF_QPDF` or `ANPDF_PDFIUM_LIB` override: qpdf
confirmed the encrypted file required a password, and the decrypted copy was
valid and retained its single page and `PAGE-ONE` text. Homebrew needed a
metadata refresh to replace the satellite's older qpdf 12.3.2 bottle.

The local `linux/amd64` image built from `containers/Dockerfile` passed
`containers/smoke.sh` as UID 65532 with a read-only root and no network,
including the password round-trip. Pinned Syft/Grype and `build/oci-audit.py`
passed on the saved image: 269 SBOM packages, no unexpected Rust crates or
disallowed licenses, and no fixable High/Critical matches. The scan still
reported 81 High and 64 Medium matches; a passing policy gate is not a
vulnerability-free claim. The hosted release includes per-architecture audit reports; the arm64 image smoke is not the full native arm64 integration corpus.

## 0.4.3

Adds cross-surface coverage for a layered compression target miss: the
fixed-resolution result is returned without an image-downsampling pass.

## 0.4.2

Brings opt-in layered compression to the 0.4.x line. The tag workflow runs full
automated qualification and notifies the regular Homebrew tap after publication.
This remains a development milestone, not a production-qualified release.

## 0.5.0-alpha.1

Adds opt-in layered PDF compression. The tag workflow runs full CI and
publishes development artifacts only after their automated checks pass.
The remaining qualification gaps below still apply; this is not a
production-qualified release.

## 0.4.x

The current baseline adds AES-256 R6 `encryptPdf`/`decryptPdf` with protected
secret channels: secrets are accepted via echo-off prompts, `chmod 600` files,
stdin, and request bodies — `--request-json` refuses literal password fields
because argv is already machine-readable at launch — and the backend hands
them to qpdf via memfd/private staging, never argv/env/logs. It adds automatic
page rasterization when a compression byte target is missed
(`pageRasterization`, PDFium-backed, evidence-gated to traced-graphics pages),
miss-outcome reporting inside `IMAGE_RESAMPLE_LIMITED`, measured terminal
progress, and the clap-built CLI. PDFium remains the default text/render
engine; qpdf stays structural; PDFOxide ships only as its own preview tag.
Fixes stay on the 0.4.x line; no 0.5.0 without explicit approval.

[v0.4.0](https://github.com/nextexcite/anPDF/releases/tag/v0.4.0) (tag
`293b0432`, prerelease, 2026-11-23) published: macOS tarball
`anpdf-0.4.0-macos-arm64.tar.gz` (6.55 MB, sha256 `0ba2ef00…`), multi-arch OCI
index `ghcr.io/nextexcite/anpdf@sha256:6eb166d1…` (amd64 40.72 MB / arm64
40.92 MB compressed, flattened rootfs), per-arch SPDX SBOMs, Grype reports,
`oci-audit-report` and `SHA256SUMS`. Every tag-workflow job green on the tag
commit (actions runs 45096553653/45097091234): real `ubuntu-26.04-arm` arm64
execution (not emulation) for the arm64 image — build, restricted smoke
(`--network=none --read-only --cap-drop=ALL --no-new-privileges`, UID 65532),
SBOM/Grype/license audit, push-by-digest, then a pull of the published digest
re-smoked; `macos-26` arm64 tarball built and smoke-tested against pinned qpdf
12.4.2; the tap updater fired `repository_dispatch` and committed the formula
update within seconds (the v0.3.4 empty-token failure is resolved; cron
fallback retained).

**Audit counts** (both arches): 268/266 SBOM packages, 81 Debian packages,
185/183 Rust crates; 0 disallowed licenses, 0 unexpected crates, 0 missing deb
packages, **0 fixable High/Critical**. Grype still reports **78 High + 64
Medium non-fixable advisories** — a passing fixable-only gate, not a
vulnerability-free claim; the non-fixable count should be tracked across
releases.

**Package regression evidence: the real Homebrew install on macOS arm64 CI was
tested; the document was synthetic.** The tap's own CI runs on a real
`macos-26` runner — installs the formula, `brew test`, a
baseline-without-libpdfium vs patched render/extract check, and a real
`compress --max-bytes` asserting `pageRasterization` in techniques and
`targetMet` on a high-entropy synthetic fixture. The formula's `revision 1`
installs the bundled `libpdfium.dylib` so the original v0.4.0 packaging miss
(library omitted) forces reinstall. What that does *not* prove: a human-driven
`brew install` session; the in-repo package regression test simulates renderer
absence via env on Linux.

**Missing automated packaging regressions:** no installed-binary
`encrypt`/`decrypt` smoke anywhere — the formula `test` block covers
version/operations/completions/render-page only, and `containers/smoke.sh`
skips encryption. Separately, a one-off AES-256 check against the released
Linux image exists but is not part of release gating.

**Benchmarks:** CI produces only the reference-provider comparison (qpdf
correctness 1.000/~31 ms; poppler 0.988/~46 ms — `backend-comparison`
artifact). **No product-engine (PDFium/PDFOxide) numbers were recorded for
v0.4.0** — the `bench/providers/anpdf` adapter needs `ANPDF_BIN` +
`ANPDF_ENGINE` on a manual run and none was captured at release. Rasterize
cost is qualitative only ("order of a minute, deadline-bounded").

[v0.4.1](https://github.com/nextexcite/anPDF/releases/tag/v0.4.1) published
on 2026-11-23. The [tag workflow](https://github.com/nextexcite/anPDF/actions/runs/45150899807)
passed full automated qualification and publication.

Remaining gates are unchanged from the 0.3.0 list below, plus external MCP/HTTP
client conformance, formal OS/backend license sign-off, installed-binary
encrypt smoke (formula test + `containers/smoke.sh`), per-release
product-engine benchmark capture, and advisory-count tracking.

## Historical 0.3.0 evidence

The 0.3.0 baseline added a capability registry, PDFium as the default embedded-text
and PNG engine, qpdf per-artifact preservation reporting, and the synthetic benchmark
corpus/harness. PDFOxide remains a separate preview tag. v0.3.0 was published by the
tag release workflow (linux/amd64 + linux/arm64 images, macOS arm64 tarball); it is not
production-qualified.

PR #8's final pre-retarget CI at `0d55915` reported nine passing checks: native
Ubuntu/macOS, Ubuntu without default features, bundled PDFium Ubuntu/macOS,
restricted linux/amd64 OCI smoke, Forge output, privacy and review. This does not
establish a v0.3.0 macOS package, linux/arm64 PDFium image, or current vulnerability
scan result; tag-time artifact audits remain required. The native license inventory
now contains 155 components. Benchmark results are comparative development evidence,
not preservation, visual, security or production guarantees.

Local 0.3.0 rehearsal on Linux amd64 passed default and structural-only format,
Clippy, unit tests (22+3 and 17+3), builds and cross-surface suites (85 and 75
cases). An auditable release binary and Linux-equivalent flat tarball passed
discovery, completions, merge, compression and bundled PDFium text/600x400 PNG
smokes without a library override. The actual amd64 OCI image passed restricted
merge/text/render smokes and the pinned Syft/Grype audit: 233 packages, 81 Debian
packages, 150 Rust crates, no uninventoried crates or disallowed licenses, and no
fixable High/Critical matches under the current policy. Grype still reported 76
High-severity matches; a passing policy gate is not a vulnerability-free claim.
No tag, image or release asset was published by this rehearsal. macOS packaging
and linux/arm64 execution were not repeated locally for 0.3.0.

## Historical 0.2.0 evidence

Observed locally and on hosted GitHub Actions runners (`ubuntu-26.04` amd64,
`ubuntu-26.04-arm` arm64 for release OCI, and `macos-26` arm64). These checks establish a
development `0.2.0` vertical slice and multi-arch artifact publication; they do not establish
a production V0/`1.0.0` release.

| Check | Observed result |
|---|---|
| Canonical OpenAPI 3.0.3 | Independent openapi-spec-validator passed |
| Rust | Build, unit safety/schema/inspect/discovery tests, format and Clippy with warnings denied passed on Linux amd64 and macOS arm64 |
| Shared corpus | 55 cross-surface cases (inspect, merge, extract, split, remove-pages, compress, malformed/stream-bomb) through CLI/HTTP/MCP passed with complete domain envelope equality on `ubuntu-26.04` and `macos-26` |
| Preservation/order | Every corpus input hash preserved; independent Poppler text order passed |
| HTTP | Loopback binding, bearer, Host/Origin, discovery and JSON error checks passed |
| MCP | Initialization, discovery, calls, error wrapping and stdout framing passed |
| qpdf | Exact 12.4.2 official archive matched published checksum; actual structural/transform commands ran on Linux amd64 and macOS arm64 |
| Forge | Official multi-platform 0.2.0 package (`b8d74e2`), three native generators, deterministic regeneration, and TS compilation passed; fidelity gaps documented |
| Edge fixtures | Embedded CJK text survived extraction; identical rendered pixels; synthetic signing and tampering independently verified; both refused by product |
| Offline | Native inspection passed in an isolated Linux network namespace |
| Process/filesystem | Timeout, stdout/stderr capture overflow, object-count/deep-nesting/non-finite-MediaBox guards, sparse-file limit, symlink/alias rejection, atomic competing outputs, and cleanup tests passed (`RLIMIT_AS` on Linux; `proc_pidinfo` RSS watchdog on macOS) |
| Native artifact | Auditable release binary and actual Syft SPDX SBOM produced; `macos-arm64` release tarball packaged and smoke-tested on `macos-26` |
| GitHub workflows | Hosted `ci.yml` (`ubuntu-26.04`, `macos-26`, `forge-output`, `privacy`) and `oci.yml` (`ubuntu-26.04`, `ubuntu-26.04-arm`, `macos-26`) |
| OCI (`linux/amd64`, `linux/arm64`) | Built natively on `ubuntu-26.04` and `ubuntu-26.04-arm`, passed non-root/read-only/`--network=none` smoke test, Syft SPDX SBOM, Grype vulnerability gate, and `build/oci-audit.py` license/crate audit before GHCR push + unified index verification |

Synthetic recipes contain no personal data or external documents. CJK testing uses
an exact checksum/revision-pinned OFL-1.1 font; the ephemeral test certificate/key
is generated in memory and never stored in source. A synthetic marker fixture is
clearly distinguished from real signing fixtures. Generated PDF manifests include
checksums, source recipes, licensing and expected counts. Visual checks are test
evidence, not a rendering/signing product feature.

Remaining qualification residuals:

- The exact 0.5.0 release workflow closed the hosted native Linux arm64 and macOS full-lane gaps for this release. This does not generalize into a support claim for other runner/backend combinations.
- Forge's runtime boundary is explicit in `docs/FORGE.md`: checked-in generated artifacts are qualified build outputs, Node/npm are build-time-only, and the handwritten Rust adapters independently validate runtime requests/responses. The accepted residual is limited to the executable probe's documented upstream schema-fidelity gaps.
- Deterministic input/output filesystem races, MCP cancellation/EOF cleanup, raw HTTP disconnect cancellation, malformed object/stream cases and compression bombs passed in the exact release workflow. Complex widget appearances and incremental/expired-signature variants remain out of scope because form filling and signing remain unsupported.
- Deterministic pinned-backend structural goldens passed in the exact release workflow. Broader cross-version backend comparison remains outside the exact-qpdf release boundary; independent structural success still does not establish every PDF's visual correctness.
- Exact release OCI license/advisory artifacts are recorded above. Formal legal sign-off, especially copyleft/source-offer treatment of GPL/LGPL-family Debian metadata, remains an explicit residual risk requiring owner acceptance, not an engineering conclusion. The OCI audit records Debian license counts and fix-state/severity vulnerability counts separately from the pass/fail fixable High/Critical gate.
- Per-release product-engine benchmark capture beyond the workflow's comparative `backend-comparison` artifact remains optional development evidence, not a 0.5.0 qualification gate.

Check scripts remain in the repository. Raw execution logs and sensitive scan
particulars are not public-bound.
