# Preliminary builds and workflows

All workflows are local source until separately authorized for a safe destination.
There is no configured registry, login step, publication job or floating latest tag.
The OCI workflow is manual-only and builds local images/exports; `push:false`.
Its version-plus-full-commit tag is intended to be used immutably. Future registry
publication requires enforcing immutable tags and recording the resulting digest.

OCI builder/runtime images are pinned by index digest in `containers/Dockerfile`.
Both tags were resolved against the official Docker registry. Debian package
resolution uses the signed 20261118T000000Z snapshot. Exact qpdf 12.4.2 source is
SHA-256 pinned and built with OpenSSL, avoiding the extra crypto libraries bundled
with upstream development/test binary archives. Node, Forge, Rust and compilers
are absent from the intended runtime stage. Rust dependency metadata is embedded
with pinned cargo-auditable 0.7.7 for actual binary SBOM discovery. Runtime images
carry license notices and retain distribution package copyright records.

```sh
docker build -f containers/Dockerfile -t anpdf:0.1.0-alpha.1-local .
sh containers/smoke.sh anpdf:0.1.0-alpha.1-local
```

The smoke test uses numeric non-root UID 65532, read-only rootfs, no network,
no-new-privileges, no capabilities, bounded tmpfs and resource limits. Separate
read-only input and writable temporary output mounts are used. The recipe and
script are prepared; no local OCI build or execution is claimed.

The native CI matrix has one required Linux amd64 job. Linux arm64 and native
macOS arm64/amd64 jobs are explicitly experimental until their first actual
qualification. Do not treat a workflow passing with those jobs failing as a
multi-platform release. Poppler and upstream qpdf binary archives are test tools,
not runtime image dependencies. OpenAPI validation and optional signed/CJK fixtures
can use pinned Python packages from `build/test-requirements.txt`.

Current action releases were checked through each official upstream latest release
and tag API on 2026-11-19. Full tag-object commit SHAs are pinned with version comments:

| Action | Verified release |
|---|---|
| [actions/checkout](https://github.com/actions/checkout/releases/tag/v7.0.1) | v7.0.1 |
| [actions/setup-node](https://github.com/actions/setup-node/releases/tag/v7.1.0) | v7.1.0 |
| [docker/setup-buildx-action](https://github.com/docker/setup-buildx-action/releases/tag/v4.4.1) | v4.4.1 |
| [docker/build-push-action](https://github.com/docker/build-push-action/releases/tag/v7.4.0) | v7.4.0 |

Only minimum read permissions are requested, checkout credentials are not retained,
and no credentials are needed on untrusted PRs. Generated-code checks use a locked
Node build dependency tree. Gitleaks and Syft qualification binaries are checksum
pinned. Actionlint 1.7.12 validates workflow syntax and expressions locally.
The workflows have not run on hosted runners; action/runner integration is a
preliminary remote check, not established by local linting.

The OCI workflow produces an OCI archive, minimal provenance and an SPDX image
SBOM with checksums, without pushing a registry image. Artifact upload remains
disabled until the separate privacy/publication gate. Before any release, inspect
all actual image layers, OS licenses, generated provenance/labels and final artifact
SBOM, and qualify both image architectures. No token should be supplied in chat.
