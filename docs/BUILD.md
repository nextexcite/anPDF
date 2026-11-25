# Preliminary builds and workflows

Routine CI runs on `ubuntu-26.04` (Linux amd64); full qualification also
runs on `macos-26` (Apple Silicon arm64).
Release builds (`.github/workflows/oci.yml`) additionally build OCI natively on `ubuntu-26.04`
(`linux/amd64`) and `ubuntu-26.04-arm` (`linux/arm64`). `.github/workflows/oci.yml` triggers on `v*`
tag pushes except PDFOxide preview tags, and on `workflow_dispatch`, reading the authoritative version from `Cargo.toml` (and asserting
tag alignment on tag pushes), so untrusted PRs never receive package or release write permissions.

The `0.5.0` release line includes page reordering, grouped command help, opt-in layered compression, and the
capability registry, default PDFium content operations, qpdf preservation reporting,
size-targeted compression with automatic page rasterization, AES-256 encrypt/decrypt,
and the synthetic comparison harness in `docs/BENCHMARK.md`. Status and the
prioritized queue live in `docs/HANDOFF.md`.

The Homebrew tap tracks plain `vMAJOR.MINOR.PATCH` tags. Download suffixed
prereleases from GitHub Releases or use their versioned OCI tags directly.

Versioning convention: `< 0.5.0` tags use clean milestone versions (`0.3.0`, `v0.3.0`); SemVer prerelease identifiers (`-alpha.N`, `-rc.N`) start at `>= 0.5.0`.

## Default content engine

Standard native, Homebrew, and OCI releases use PDFium for `extractText` and
`renderPage`; qpdf remains the structural engine. PDFOxide ships separately under
its own preview tag. `cargo build --locked` includes PDFium support;
`cargo build --locked --no-default-features` builds the structural-only variant.
Source builds need `python3 build/get-pdfium.py build/work/pdfium` and either
`ANPDF_PDFIUM_LIB` set to its printed path or the library installed beside the binary.

Library precedence is `--pdfium-lib` > `ANPDF_PDFIUM_LIB` >
`<canonical-executable-dir>/../lib/anpdf/libpdfium.{so,dylib}` >
`<canonical-executable-dir>/libpdfium.{so,dylib}`. Missing libraries leave content
operations unavailable. The native tarball contains `anpdf`, `libpdfium.dylib`,
and `PDFIUM_LICENSES/` (upstream LICENSE plus licenses directory). Homebrew installs
the dylib in `lib/anpdf/` and notices under its documentation directory. The formula
in the `nextexcite/homebrew-anpdf` tap targets the latest published tarball with its
real SHA-256 — the release workflow notifies the tap, which updates url/checksum
itself (formula revision bumps force reinstall when bundled content changes).
OCI installs `/usr/local/lib/anpdf/libpdfium.so`, sets `ANPDF_PDFIUM_LIB`, and
carries `/usr/share/licenses/pdfium/`. Binary acquisition is SHA-256 pinned for
Linux x64/arm64 and macOS arm64; the dynamic library is not statically linked.

## Published development artifacts

Each release run of `.github/workflows/oci.yml` builds, scans, smoke-tests, and publishes:

1. **Multi-arch `linux/amd64` + `linux/arm64` OCI index (GHCR):** `ghcr.io/<owner>/anpdf` with tags
   `<version>-<sha>`, `<version>`, and `latest`, minimal BuildKit provenance, per-arch
   Syft SPDX image SBOMs (`anpdf-linux-{amd64,arm64}.spdx.json`), Grype vulnerability reports
   (`anpdf-linux-{amd64,arm64}.grype.json`), license/crate policy audits
   (`oci-audit-report-linux-{amd64,arm64}.json`), and recorded digest (`oci-digest.txt`). Each
   architecture passes the non-root, read-only, `--network=none` smoke test AND `build/oci-audit.py`
   (disallowing AGPL, checking uninventoried Rust crates, and blocking fixable High/Critical CVEs)
   before push by digest; `image-index` then creates and smoke-tests the unified OCI index (`DOCKER_BUILD_RECORD_UPLOAD=false`
   keeps build-record bundles off the release).
2. **`macos/arm64` native binary (GitHub Release + Homebrew tap [nextexcite/homebrew-anpdf](https://github.com/nextexcite/homebrew-anpdf)):**
   `anpdf-<version>-macos-arm64.tar.gz` and `SHA256SUMS-macos-arm64.txt`, built with
   pinned Rust `1.100.0` and `cargo-auditable 0.7.7`, smoke-tested against pinned qpdf `12.4.2`,
   and attached to the `v<version>` GitHub release alongside the OCI SBOMs, scan reports, and digests.

## PDFOxide preview releases

After the preview workflow is merged, tag the intended commit on `main` as
`v<Cargo.toml version>-pdfoxide-preview.<number>` (for example, `v0.2.0-pdfoxide-preview.1`)
and push that tag. Only `.github/workflows/pdfoxide-preview.yml` publishes for these tags.
The workflow builds Linux amd64 with `--features pdfoxide`, smoke-tests extraction and rendering
under the same restricted runtime settings as the main build, and audits the image before publishing.

The preview uses `ghcr.io/<owner>/anpdf-pdfoxide:<tag>` with no `latest` tag and creates
a separate GitHub prerelease. It includes `anpdf-<tag>-linux-amd64.image.tar.gz`, an SPDX
SBOM, a Grype report, a license/crate audit, and checksums. The image carries the feature
dependency inventory and notices. No main image tags, native artifacts, or Homebrew
formula are updated by the preview workflow.

## OCI recipe and local reproduction

OCI builder and Debian rootfs base images are pinned by index digest in `containers/Dockerfile`.
Both tags were resolved against the official Docker registry. Debian package
resolution uses the signed `20261118T000000Z` snapshot (`trixie`, `trixie-updates`, and
`trixie-security`). Exact qpdf 12.4.2 source is SHA-256 pinned and built with OpenSSL,
avoiding the extra crypto libraries bundled with upstream development/test binary archives.
Node, Forge, Rust and compilers are absent from the runtime stage. Rust dependency metadata
is embedded with pinned `cargo-auditable 0.7.7` for actual binary SBOM discovery. Runtime
images carry license notices and retain distribution package copyright records.

The runtime is still a complete **Debian trixie-slim** rootfs, not distroless.
An intermediate `rootfs` stage performs the snapshot-backed security upgrade and
runtime package installation. The final `FROM scratch` stage copies that complete
filesystem once, eliminating replaced package bytes from lower layers; it does
not remove Debian's Essential packages, package manager, dpkg database, or
`/etc/os-release`. PATH, backend configuration, numeric non-root user, working
directory, entrypoint, command and labels are explicitly restored. BuildKit may
emit a 32-byte empty gzip layer for `WORKDIR`; all filesystem data is in one layer.

Debian slim already excludes man/info/locale data and most documentation with a
dpkg exception for `/usr/share/doc/*/copyright`. Cleanup retains copyright,
license, copying and notice files and all product/dependency notices; it removes
only remaining non-license documentation, man/info/locale data, apt caches,
logs and old debconf backups. No runtime packages are removed: qpdf still needs
libjpeg, libcrypto and libstdc++, and `ldd` alone does not prove that a Debian
package or dynamically loaded facility is safe to remove.

Local linux/amd64 size rehearsal, using the same source and gzip OCI exporter:

| Variant | `docker image ls` disk usage | Compressed OCI layer bytes |
|---|---:|---:|
| Original layered recipe | 217 MB | 55,276,407 |
| Flattened rootfs only | 153 MB | 40,000,772 |
| Flattened rootfs plus cleanup | 152 MB | 39,784,700 |

The final compressed layers are **28.0% smaller** than the original recipe.
Disk usage is Docker's local storage accounting, not registry transfer size;
compressed sizes sum the gzip layer descriptors in each OCI image manifest.
The rehearsal preserved all 81 Debian packages and their versions/Essential flags,
355 license/copyright files byte-for-byte, and six notice symlinks. These counts
describe that amd64 run, not a cross-platform guarantee. Arm64 requires hosted
qualification; changing base pins or package versions requires remeasurement.

```sh
docker build -f containers/Dockerfile -t anpdf:local .
sh containers/smoke.sh anpdf:local
```

The smoke test uses numeric non-root UID `65532`, read-only rootfs, `--network=none`,
`no-new-privileges`, dropped capabilities, bounded tmpfs and resource limits, with separate
read-only input and writable temporary output mounts.

## Build caches and release provenance

`ci.yml` and `oci.yml` use SHA-pinned `actions/cache` for Cargo's public registry
index, archives and sources, and separate Rust `target` caches for native CI,
PDFium CI, structural-only CI and native release builds. Registry keys include
OS, architecture, `Cargo.lock` and `rust-toolchain.toml`; target keys additionally
include source inputs, with fallback restricted to the same dependency/toolchain
and feature lane. Cargo still runs with `--locked`; build, test and smoke steps
are not skipped on cache hits. Rustup toolchain installation still runs normally.

Pinned qpdf, PDFium, test-font, Gitleaks and Syft/Grype downloads use exact keys
containing the downloader script hash (including its version/checksum pins), plus
OS/architecture for binaries. There are no broad fallback keys for these tools.
On a miss the existing checksum-verifying downloader runs; on a hit its extracted
files are reused, not freshly downloaded/reverified. qpdf and PDFium path files
are always regenerated from the downloader's canonical
path output (`--path-only` on cache hits, validating executable/library presence),
not reconstructed from the shell's working directory. Forge's cached archive is
rechecked against `forge-pin.json` by `generate.py` every time. Native release
jobs also cache only the `cargo-auditable` executable, keyed by its explicit
version and toolchain; Node qualification uses setup-node's locked npm cache.
The Grype vulnerability database and scan/SBOM results are **not** cached.

OCI builds may **read and write GitHub Actions BuildKit layer caches**, including
intermediate build stages (`mode=max`), scoped separately as `oci-amd64` and
`oci-arm64`. The manual CI amd64 smoke build seeds `oci-amd64` when run on the default branch;
release local builds read/write the corresponding architecture scope, and the
subsequent OCI export and digest push read it. GitHub's ref-based cache access
rules still apply: PR caches cannot overwrite default-branch caches; tag-scoped
caches generally help reruns of that tag, not later tags. In particular, arm64
and native-release-only caches may start cold on a new tag. Missing caches cause
normal builds/downloads; layer-cache export errors do not fail an otherwise
verified build.

Caches are trusted build inputs and acceleration only, **not evidence of a clean,
cache-free rebuild** or fresh verification of every upstream download. Pinned
base images, Debian snapshot and source/download checksums remain unchanged;
checksum checks still run on fetched inputs. SBOM generation, fresh vulnerability
scanning, `oci-audit.py`, license gates and restricted runtime smokes still run
for releases, including the published-digest smoke. Minimal BuildKit provenance
must be interpreted with this cache-use policy. Cache paths exclude Cargo
credentials, registry-login configuration, tokens and other secrets; checkouts
retain `persist-credentials: false`. No secret-bearing build arguments or layers
may be introduced. To require a cache-free release, separately review disabling
cache restores/imports and requesting a no-cache build; these workflows do not
claim that property.

## Release checklist

For the next release (`v<Cargo.toml version>`), release preparation must not push tags, dispatch publication workflows,
or publish images/assets. Review `CHANGELOG.md`, the version in Cargo.toml/Cargo.lock,
the Dockerfile `APP_VERSION`, baseline documentation and regenerated contract outputs.
Keep Forge **0.2.0** and third-party dependency pins unchanged.

For release review, use the checks below. The release workflow runs the version
check, full CI, and shipping-artifact checks automatically; a separate CI dispatch
is not required. Review any evidence the workflow does not collect before
authorizing publication.

1. Dry-run the `prepare` job's version/tag validation with `GITHUB_REF_TYPE=tag`
   and `GITHUB_REF_NAME=v<Cargo.toml version>`; reject mismatched tags. Do not execute publish jobs.
2. Run format, Clippy, tests and builds with default features and
   `--no-default-features`; run the qpdf/PDFium cross-surface suite using a bundled
   library layout without `ANPDF_PDFIUM_LIB`. Regenerate licenses without drift,
   and run Forge qualification, privacy and gitleaks checks.
3. Build an auditable release binary, stage the native tarball with its PDFium
   library and notices, and smoke-test that staged binary. On Apple Silicon,
   install the local tarball with the tap formula pointed at its local URL and
   checksum, run `brew test`, then run `build/smoke-protection.sh` against the
   installed binary and a generated `one.pdf`. Use qpdf 12.4.2; refresh Homebrew
   metadata if it offers an older dependency. A Linux simulation is not macOS
   qualification.
4. Build the local OCI image, run `containers/smoke.sh`, export an OCI archive,
   and run pinned Syft, Grype and `build/oci-audit.py` exactly as in `oci.yml`.
   Review failures; do not waive CVE/license gates to publish.
5. After a separately authorized tag build publishes the actual macOS tarball,
   verify its SHA-256 against `SHA256SUMS-macos-arm64.txt`. In a follow-up commit,
   the [homebrew-anpdf](https://github.com/nextexcite/homebrew-anpdf) tap workflow updates its formula's url/checksum from the published `SHA256SUMS-macos-arm64.txt` on schedule; verify it committed, and run `brew test anpdf` on macOS against that artifact.

## CI matrix and pinned GitHub Actions

Pull requests and pushes to `main` run one default Rust build and the bundled
PDFium routine corpus (`tests/integration.py --routine`) on `ubuntu-26.04`,
plus `forge-output` and `privacy`. They do not build macOS or OCI artifacts.
The routine corpus keeps cross-surface checks, including CJK text and rendering,
and runs one real image-downsampling and one page-rasterization case via the CLI.

The release workflow calls **Rust and contract qualification** on the same commit
after validating the version and tag. Qualification must pass before the release
builds start, including before any GHCR push. It runs the full bundled PDFium
corpus on Linux and macOS, including the compression matrix across all three
interfaces, plus the Ubuntu `--no-default-features` corpus, reference backend
benchmark, restricted linux/amd64 OCI smoke, Forge, and privacy checks. Dispatch
**Rust and contract qualification** directly to run those checks without
publishing. The release workflow still builds and verifies each shipping platform.

Poppler and upstream qpdf binary archives are test tools, not runtime image dependencies.
OpenAPI validation and optional signed/CJK fixtures can use pinned Python packages from
`build/test-requirements.txt`.

Existing action releases were checked through each official upstream latest release
and tag API on 2026-11-19; the added cache action was verified against its upstream
tag ref with `git ls-remote`. Full commit SHAs are pinned with version comments:

| Action | Verified release |
|---|---|
| [actions/checkout](https://github.com/actions/checkout/releases/tag/v7.0.1) | v7.0.1 |
| [actions/setup-node](https://github.com/actions/setup-node/releases/tag/v7.1.0) | v7.1.0 |
| [actions/cache](https://github.com/actions/cache/releases/tag/v5.1.0) | v5.1.0 (tag SHA verified with `git ls-remote`) |
| [actions/upload-artifact](https://github.com/actions/upload-artifact/releases/tag/v7.0.2) | v7.0.2 |
| [actions/download-artifact](https://github.com/actions/download-artifact/releases/tag/v8.0.2) | v8.0.2 |
| [docker/setup-buildx-action](https://github.com/docker/setup-buildx-action/releases/tag/v4.4.1) | v4.4.1 |
| [docker/login-action](https://github.com/docker/login-action/releases/tag/v4.8.0) | v4.8.0 |
| [docker/build-push-action](https://github.com/docker/build-push-action/releases/tag/v7.4.0) | v7.4.0 |

Only minimum read permissions are requested at workflow top level; `packages: write` and
`contents: write` are scoped strictly to the jobs that publish in `oci.yml`. Checkout
credentials are not retained. Generated-code checks use a locked Node build dependency
tree. Gitleaks and Syft qualification binaries are checksum pinned.
