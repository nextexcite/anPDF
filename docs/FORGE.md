# Forge qualification

Mandatory build-time adoption: official Cloudflare Forge 0.2.0, source revision
`b8d74e2cbe41e4afaf2acaec2903add0bc11b116`.

The exact upstream source at revision `b8d74e2cbe41e4afaf2acaec2903add0bc11b116`
is built by upstream CI with Rust 1.99.0 (`cargo build --release --locked --features wasm`,
pnpm workspace, and `packages/forge/scripts/archive.mjs`) into the multi-platform
`@cloudflare/forge` `0.2.0` preview package (`darwin-arm64`, `darwin-x64`, `linux-arm64`,
`linux-x64`, `win32-arm64`, `win32-x64`). Its SHA-256 is pinned in `build/forge-pin.json`
and fetched/verified by `build/get-forge.py`; the source revision remains independently
rebuildable from upstream Git. The npm registry `0.0.0` placeholder is not used.

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
python3 build/get-forge.py build/work/forge.tgz
python3 build/generate.py --archive build/work/forge.tgz
npm ci --ignore-scripts --prefix build/qualification
mkdir -p build/qualification/work
cp -R generated/ts generated/mcp generated/capnweb build/qualification/work/
build/qualification/node_modules/.bin/tsc -p build/qualification/tsconfig.json
node --import ./build/qualification/register-hooks.mjs build/qualification/probe.mjs
```

Generation requires Node 26 and the exact archive checksum; mismatches fail.
`cargo build` works offline from checked-in outputs and fails on contract, pin,
output hash or generated operation-coverage drift. It does not claim to rerun Forge.

For plain `v0.5.0`, Forge remains a build-time generator: the checked-in outputs
are qualified artifacts, while Node/npm and the pinned archive are not runtime
or redistributed product components. The accepted engineering-scope residual is
limited to the probed upstream schema-fidelity gaps above and to upstream
generator behavior outside the canonical contract; the handwritten Rust adapters
remain authoritative and independently validate every running request/response.
A redistributable Forge toolchain or broader generator conformance is explicitly
outside the 0.5.0 runtime qualification boundary.

Sources: [official pinned package/build documentation](https://github.com/cloudflare/forge/blob/b8d74e2cbe41e4afaf2acaec2903add0bc11b116/packages/forge/README.md),
[official helper source](https://github.com/cloudflare/forge/blob/b8d74e2cbe41e4afaf2acaec2903add0bc11b116/src/bin/forge/mcp/lib/src/tool.ts).
