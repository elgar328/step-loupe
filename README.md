# step-loupe

**The example app for [step-io](https://github.com/elgar328/step-io).**

A single-file, in-browser **STEP (ISO 10303) viewer** that renders what step-io
reads — b-rep geometry, the assembly tree, PMI (features, dimensions, tolerances,
datums), units, and a provenance report — with three.js, inline in one
self-contained HTML file. `src/lib.rs` is the whole glue between step-io
(compiled to WebAssembly) and the page, so it doubles as a worked example of
step-io's reading API.

## Demo

**<https://elgar328.github.io/step-loupe/?file=nist-ctc05.step>**

Point `?file=<url>` at any STEP file (its host must allow CORS), or use the
**Open STEP file** button / drag-and-drop.

## Build

Requires Rust with [wasm-pack](https://rustwasm.github.io/wasm-pack/), plus Python 3.

```sh
wasm-pack build --target web --release
python3 scripts/build_single.py   # inlines the wasm glue → step-loupe.html
```

The result, `step-loupe.html`, is fully self-contained (three.js loads from a CDN).

## Layout

```
src/lib.rs            Rust → wasm glue (load_step)
src/index.html        viewer frontend (source; not the deployed page)
scripts/              build + deploy tooling
sample/               example STEP file
```

## Example file

`sample/nist-ctc05.step` is **NIST CTC 05** (AP242 e1), a public-domain part from
the NIST MBE PMI Validation & Conformance Testing set.

## Releasing

The hosted demo is deployed to the `gh-pages` branch and each release attaches a
downloadable, offline `step-loupe.html` — see [docs/RELEASING.md](docs/RELEASING.md).

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE)
at your option — the same as [step-io](https://github.com/elgar328/step-io).

The bundled example `sample/nist-ctc05.step` is a NIST public-domain part.
