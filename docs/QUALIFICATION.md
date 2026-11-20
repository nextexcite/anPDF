# Preliminary qualification evidence

## 0.3.0

The current baseline adds a capability registry, PDFium as the default embedded-text
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

Remaining release gates:

- Full native `cargo test` + cross-surface corpus execution directly on `linux/arm64` (release builds currently verify `linux/arm64` via OCI build + restricted `containers/smoke.sh` + Syft/Grype audit) and formal OS/backend license sign-off.
- macOS virtual-address vs RSS limits (`RLIMIT_AS` is rejected by XNU; `proc_pidinfo` enforces a 768 MiB RSS limit during polling, alongside kernel `RLIMIT_CPU`, `RLIMIT_FSIZE`, and `RLIMIT_NOFILE`).
- Reproducible redistributable Forge artifact and wider generator/schema regression
  qualification; generated schema fidelity remains insufficient for validation.
- Extended malformed/object/stream bombs, namespace mutation/race stress, HTTP
  disconnect cancellation, MCP cancellation negotiation with external clients,
  complex widget appearances and incremental/expired-signature variants.
- Cross-version golden PDFs and additional visual/page semantics; independent
  structural success does not establish every PDF's visual correctness.
- Full dependency vulnerability review and target-specific OS/backend license review.
  The native Rust license manifest inventories 128 components and retains legal notices;
  OS/backend/build-tool artifacts require separate review.

Check scripts remain in the repository. Raw execution logs and sensitive scan
particulars are not public-bound.
