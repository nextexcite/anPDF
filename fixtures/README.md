# Synthetic fixtures

For a broader synthetic engine-comparison corpus and external provider harness,
see [the benchmark guide](../docs/BENCHMARK.md). It is separate from product
qualification and does not enable unsupported product operations.

Run `python3 fixtures/generate.py build/work/fixtures`.
Original deterministic byte fixtures are Apache-2.0. Recipes contain no timestamps,
external files, personal metadata, or keys. The signature marker is deliberately
not a valid cryptographic signature. It tests conservative presence rejection.
Unicode filenames alone do not establish Chinese glyph rendering. Optional CJK
and real signed/tampered qualification is described below; incremental/expired
signature variants remain unqualified.

Each generated fixture has a checksum, provenance, and expected page count in its
manifest. Inputs are copied into private test workspaces; generated PDFs are not
tracked. Form fixtures test duplicate field names, UTF-16 values and XFA detection,
not interactive appearance preservation. Transformation of forms is rejected.

Optional real signing/CJK qualification is implemented in `edge-cases.py`.
Install `build/test-requirements.txt`, fetch the exact OFL font with
`build/get-test-font.py`, and run the recipe in an ignored build/work directory.
It verifies a generated self-signed synthetic certificate and tampered bytes with
network fetching disabled. Keys exist only in memory. These fixtures are not
tracked, and the optional signed PDFs are intentionally nondeterministic.
CJK source/subset renders were independently inspected and compared pixel-for-pixel.
