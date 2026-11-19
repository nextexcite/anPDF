# Synthetic fixtures

Run `python3 fixtures/generate.py build/work/fixtures`.
Original deterministic byte fixtures are Apache-2.0. Recipes contain no timestamps,
external files, personal metadata, or keys. The signature marker is deliberately
not a valid cryptographic signature. It tests conservative presence rejection.
Unicode filenames do not establish Chinese glyph rendering; embedded CJK and real
signed/incremental/tampered fixtures require additional qualification.

Each generated fixture has a checksum, provenance, and expected page count in its
manifest. Inputs are copied into private test workspaces; generated PDFs are not
tracked. Form fixtures test duplicate field names, UTF-16 values and XFA detection,
not interactive appearance preservation. Transformation of forms is rejected.
