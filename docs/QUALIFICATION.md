# Preliminary qualification evidence

Locally observed on Linux amd64. These checks establish a preliminary native
build for CI/image workflow testing; they do not establish a V0 release.

| Check | Observed result |
|---|---|
| Canonical OpenAPI 3.0.3 | Independent openapi-spec-validator passed |
| Rust | Build, five safety/schema tests, format and Clippy with warnings denied passed |
| Shared corpus | 35 cases through CLI/HTTP/MCP, complete domain envelope equality |
| Preservation/order | Every corpus input hash preserved; independent Poppler text order passed |
| HTTP | Loopback binding, bearer, Host/Origin, discovery and JSON error checks passed |
| MCP | Initialization, discovery, calls, error wrapping and stdout framing passed |
| qpdf | Exact 12.4.2 official archive matched published checksum; actual structural/transform commands ran |
| Forge | Official source build, official package stage, three native generators, repeated output and TS compilation passed; fidelity gaps documented |
| Edge fixtures | Embedded CJK text survived extraction; identical rendered pixels; synthetic signing and tampering independently verified; both refused by product |
| Offline | Native inspection passed in an isolated Linux network namespace |
| Process/filesystem | Timeout, capture overflow, sparse-file limit, symlink/alias rejection, atomic competing outputs and cleanup tests passed |
| Native artifact | Auditable release binary and actual Syft SPDX SBOM produced; 125 discovered packages |
| GitHub workflows | Actionlint passed; hosted executions not run |
| OCI | Not built: rootless setup requires the unavailable host newuidmap helper; no existing Docker/Podman runner |

Synthetic recipes contain no personal data or external documents. CJK testing uses
an exact checksum/revision-pinned OFL-1.1 font; the ephemeral test certificate/key
is generated in memory and never stored in source. A synthetic marker fixture is
clearly distinguished from real signing fixtures. Generated PDF manifests include
checksums, source recipes, licensing and expected counts. Visual checks are test
evidence, not a rendering/signing product feature.

Remaining release gates:

- Actual OCI build, non-root/read-only/network-none runtime test, complete image
  SBOM/license/provenance/privacy review and image vulnerability checks.
- Native macOS/Linux arm64 results and platform-specific resource/atomic semantics.
- Reproducible redistributable Forge artifact and wider generator/schema regression
  qualification; generated schema fidelity remains insufficient for validation.
- Extended malformed/object/stream bombs, namespace mutation/race stress, HTTP
  disconnect cancellation, MCP cancellation negotiation with external clients,
  complex widget appearances and incremental/expired-signature variants.
- Cross-version golden PDFs and additional visual/page semantics; independent
  structural success does not establish every PDF's visual correctness.
- Full dependency vulnerability review, target-specific licenses and final artifact
  privacy checks. The native Rust license manifest inventories 128 components and
  retains legal notices; OS/backend/build-tool artifacts require separate review.
- Explicit safe owner, identity/privacy gate and separate remote/publication approval.

Check scripts remain in the repository. Raw execution logs and sensitive scan
particulars are not public-bound. No remote, registry upload, push or hosted CI run
has been performed by this development session.
