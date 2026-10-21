# step-loupe

**A STEP (ISO 10303) viewer in a single HTML file.**

Open it [on the web](https://elgar328.github.io/step-loupe/), or download
`step-loupe.html` from the [latest release](https://github.com/elgar328/step-loupe/releases/latest)
and open it wherever you keep it. Then drop a STEP file onto the page.
Nothing to install, no server to run, and your files never leave your device.

step-loupe shows the geometry, the assembly tree, PMI (dimensions, tolerances,
datums) and product data, plus a report of what reading the file kept,
repaired or dropped.

Built on [step-io](https://github.com/elgar328/step-io).

[![step-loupe showing NIST CTC 05 on desktop and mobile](https://github.com/elgar328/step-loupe/releases/download/assets/screenshot.webp)](https://elgar328.github.io/step-loupe/?file=nist-ctc05.step)

## Usage

Try it on a sample file (NIST CTC 05, public domain):
**<https://elgar328.github.io/step-loupe/?file=nist-ctc05.step>**

- **Open a file:** drop it onto the page, or use the **Open STEP file** button.
- **Open from a URL:** add `?file=<url>` to the address (the file's host must
  allow CORS).
- **Up axis:** the view opens Y-up. For a Z-up model, choose **Z axis up**
  from the **Y↑** button, or add `&up=z` to a `?file=` address.

## Build

Requires Rust with [wasm-pack](https://rustwasm.github.io/wasm-pack/) and
Python 3.

```sh
wasm-pack build --target no-modules --release
python3 scripts/build_single.py   # → step-loupe.html
```

The result is a single `step-loupe.html`; three.js loads from a CDN, so it
needs a network connection.

## Analytics

The web version uses [Umami](https://umami.is), cookieless analytics that
leaves out the `?file=` address, so the files you open are never recorded.
The downloaded `step-loupe.html` carries no analytics.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE)
at your option.
