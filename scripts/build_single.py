#!/usr/bin/env python3
"""Bundle the multi-file viewer into a single self-contained step-loupe.html.

Run `wasm-pack build --target no-modules --release` first, then this script.
It inlines pkg/step_loupe.js (glue) and pkg/step_loupe_bg.wasm as base64 into
src/index.html, producing step-loupe.html at the project root. three.js stays on
the CDN (import map)."""

import base64
import pathlib
import sys

root = pathlib.Path(__file__).parent.parent
html = (root / "src/index.html").read_text()
glue_b64 = base64.b64encode((root / "pkg/step_loupe.js").read_bytes()).decode()
wasm_b64 = base64.b64encode((root / "pkg/step_loupe_bg.wasm").read_bytes()).decode()

old = """const glueSrc = await (await fetch('./pkg/step_loupe.js')).text();
const wasmBytes = new Uint8Array(await (await fetch('./pkg/step_loupe_bg.wasm')).arrayBuffer());"""
new = f"""const GLUE_B64 = "{glue_b64}";
const WASM_B64 = "{wasm_b64}";
const b64bytes = (s) => Uint8Array.from(atob(s), c => c.charCodeAt(0));
const glueSrc = new TextDecoder().decode(b64bytes(GLUE_B64));
const wasmBytes = b64bytes(WASM_B64);"""

if old not in html:
    sys.exit("error: wasm load anchor not found in src/index.html")

out = html.replace(old, new).replace(
    "<title>step-loupe — STEP viewer</title>",
    "<title>step-loupe — STEP viewer (single file)</title>",
)
(root / "step-loupe.html").write_text(out)
print(f"wrote step-loupe.html ({len(out) / 1e6:.1f} MB)")
