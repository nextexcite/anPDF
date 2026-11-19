# Forge qualification

Mandatory build-time adoption: official Cloudflare Forge 0.2.0, source revision
`b8d74e2cbe41e4afaf2acaec2903add0bc11b116`.

The exact upstream source was built using Rust 1.99.0 with
`cargo build --release --locked --features wasm`. Frozen upstream build dependencies
were installed with pnpm 11.25.0. Official `packages/forge/scripts/archive.mjs`
packed the native Linux x64 binary under Node 26.13.1. The resulting local artifact
SHA-256 is recorded in `build/forge-pin.json`; it is a GNU-ABI local qualification
artifact, not a verified cross-platform binary distribution or reproducible release.
The npm placeholder distribution was not used.

Official `ts`, `capnweb`, and `mcp` generation stages ran successfully twice on the
canonical contract. The wrapper added provenance comments and hashes; it did not
modify emitted semantics. All emitted TypeScript compiled with exact build-only
packages from `build/qualification/package-lock.json`. Operation coverage, request
encoding, success/error schema references and repeat-generation bytes were checked.
The Forge WASM runtime feature was compiled; a custom WASM plugin was not exercised.

Observed upstream fidelity gaps are checked by an executable probe:

- nested closed-object `additionalProperties:false` is lost;
- `uniqueItems`, numeric `minimum`, string min/max lengths and patterns are lost;
- HTTP-shaped MCP tools add a `body` wrapper and unsuitable default annotations;
- the upstream helper's listed output validator accepts invalid output.

Required fields, enums, operation IDs and array min/max counts survived the probes.
Generated types and operation coverage are retained as qualified build artifacts.
The Rust adapters use owned canonical schemas and independent JSON Schema
validation for every domain request and response. They are handwritten, not
represented as Forge-generated Rust. No replacement generator is used.

```sh
python3 build/generate.py --archive /path/to/the-pinned-forge.tgz
npm ci --ignore-scripts --prefix build/qualification
mkdir -p build/qualification/work
cp -R generated/ts generated/mcp generated/capnweb build/qualification/work/
build/qualification/node_modules/.bin/tsc -p build/qualification/tsconfig.json
node --import ./build/qualification/register-hooks.mjs build/qualification/probe.mjs
```

Generation requires Node 26 and the exact archive checksum; mismatches fail.
`cargo build` works offline from checked-in outputs and fails on contract, pin,
output hash or generated operation-coverage drift. It does not claim to rerun Forge.
The source-pinned tool must be rebuilt/requalified for additional platforms and a
reviewed redistributable artifact before release. General OpenAPI fidelity and
upstream generator regression coverage remain open gates.

Sources: [official pinned package/build documentation](https://github.com/cloudflare/forge/blob/b8d74e2cbe41e4afaf2acaec2903add0bc11b116/packages/forge/README.md),
[official helper source](https://github.com/cloudflare/forge/blob/b8d74e2cbe41e4afaf2acaec2903add0bc11b116/src/bin/forge/mcp/lib/src/tool.ts).
