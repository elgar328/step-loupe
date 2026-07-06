# step-loupe

A single-file, in-browser **STEP (ISO 10303) viewer** built on
[step-io](https://crates.io/crates/step-io). It renders b-rep geometry, the
assembly tree, PMI (features, dimensions, tolerances, datums) and a provenance
report — all inline in one self-contained HTML file (three.js from CDN).

## Demo

<https://elgar328.github.io/step-loupe/?file=nist-ctc05.step>

Open any STEP file with `?file=<url>`, or use the **Open STEP file** button /
drag-and-drop.

## Build

```sh
wasm-pack build --target web --release
python3 scripts/build_single.py   # inlines the wasm glue → step-loupe.html
```

## Layout

```
src/lib.rs            Rust → wasm glue (load_step)
src/index.html        viewer frontend (source; not the deployed page)
scripts/              build tooling
sample/               example STEP file
```

## Example file

`sample/nist-ctc05.step` is **NIST CTC 05** (AP242 e1), a public-domain part from
the NIST MBE PMI Validation & Conformance Testing set.
