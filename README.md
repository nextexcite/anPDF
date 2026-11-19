# anPDF

A local, headless PDF toolkit written in Rust. Independent Apache-2.0 candidate;
preliminary `0.1.0-alpha.1`, not a qualified release.

Inspect and structurally validate PDFs, merge in input order, and extract an
ascending, unique, one-based page subset. CLI, loopback HTTP, and MCP stdio share
one contract-validated dispatcher. Inputs are preserved; output creation is
exclusive unless a distinct output is explicitly replaceable.

Requires Rust 1.100.0 to build and qpdf **12.4.2** to process PDFs. Linux amd64 is
locally qualified; other runner/platform entries are experimental. Node is used
only to qualify the pinned build-time Forge outputs, never by the product.

```sh
cargo build --locked
python3 build/get-qpdf.py build/work/qpdf
python3 fixtures/generate.py build/work/fixtures
export ANPDF_QPDF="$PWD/build/work/qpdf/bin/qpdf"
./target/debug/anpdf operations
./target/debug/anpdf schema inspectPdf
./target/debug/anpdf inspect --root build/work/fixtures --request-json '{"input":{"path":"three.pdf"}}'
./target/debug/anpdf merge --root build/work/fixtures --request-json '{"inputs":[{"path":"three.pdf"},{"path":"one.pdf"}],"output":{"path":"merged.pdf"}}'
./target/debug/anpdf extract --root build/work/fixtures --request-json '{"input":{"path":"three.pdf"},"pages":[1,3],"output":{"path":"subset.pdf"}}'
```

`validate` aliases `inspect`. Requests also accept `--request-file request.json`
or JSON on stdin. JSON is always the default. Success writes one JSON value to
stdout; failures write one canonical error to stderr and a stable nonzero exit.
`--dry-run` validates the request and filesystem destination policy, without
processing PDF content or writing an output. Unknown flags/properties fail.
Paths inside requests are relative to `--root`; URLs, traversal, symlinks,
special files, and output/input aliases are denied. Use privately owned workspace
directories without group/other write access.

HTTP requires a local credential file containing at least 32 ASCII graphical
characters. Create it outside the repository and restrict its permissions.

```sh
./target/debug/anpdf serve --root build/work/fixtures --token-file /run/secrets/anpdf-bearer --port 8787
# In a separate shell, provide the credential through a private curl config.
curl --config /run/secrets/anpdf-curl.conf -H 'Content-Type: application/json' --data '{"input":{"path":"three.pdf"}}' http://127.0.0.1:8787/v1/inspect
./target/debug/anpdf mcp --root build/work/fixtures
```

The curl config supplies `header = "Authorization: Bearer ..."` using the actual
private credential. No credential is included in this repository. HTTP binds
only to 127.0.0.1, authenticates discovery as well as processing, and rejects
Origin headers, unexpected Host values, and query strings. No non-loopback or
upload service is offered. See [protocol examples](docs/PROTOCOLS.md).

Transforms reject encrypted, signed, form/XFA-bearing, and preservation-unknown
inputs. Inspection never claims cryptographic validity, visual correctness,
PDF/A compliance, or malware safety. Compression, page reordering/removal, form
filling, visual signing, certificate signing and multi-output split are planned
but unsupported. OCR and full text replacement are excluded.

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
ANPDF_QPDF="$PWD/build/work/qpdf/bin/qpdf" python3 tests/integration.py
```

[Qualification](docs/QUALIFICATION.md) records actual results and open gates.
[Forge](docs/FORGE.md) explains generation and fidelity gaps.
[Security](SECURITY.md) describes filesystem/process limits.
[OCI and CI](docs/BUILD.md) covers the non-publishing preliminary workflows.
[Architecture](docs/ADR-002.md) records the Rust runtime decision.
