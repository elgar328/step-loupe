#!/usr/bin/env python3
"""Bundle the multi-file viewer into a single self-contained step-loupe.html.

Run `wasm-pack build --target no-modules --release` first, then this script.
It puts into src/index.html, producing step-loupe.html at the project root:

- the wasm-bindgen glue (pkg/step_loupe.js), as base64;
- the wasm (pkg/step_loupe_bg.wasm), gzipped then base64'd, which the page
  inflates with DecompressionStream;
- every three.js module the import map names (from vendor/), as a data: URL in
  that map, so the page loads nothing from elsewhere and works offline;
- the version from Cargo.toml, in the view menu."""

import base64
import gzip
import json
import pathlib
import re
import sys
import tomllib
import urllib.parse

root = pathlib.Path(__file__).parent.parent
src = root / "src"
html = (src / "index.html").read_text()


def fail(msg):
    sys.exit(f"error: {msg}")


# ---- the reader: glue and wasm ----
glue_b64 = base64.b64encode((root / "pkg/step_loupe.js").read_bytes()).decode()
wasm_gz = gzip.compress((root / "pkg/step_loupe_bg.wasm").read_bytes(), 9, mtime=0)
wasm_b64 = base64.b64encode(wasm_gz).decode()

old = """const glueSrc = await (await fetch('./pkg/step_loupe.js')).text();
const wasmBytes = new Uint8Array(await (await fetch('./pkg/step_loupe_bg.wasm')).arrayBuffer());"""
new = f"""const GLUE_B64 = "{glue_b64}";
const WASM_GZ_B64 = "{wasm_b64}";
const b64bytes = (s) => Uint8Array.from(atob(s), c => c.charCodeAt(0));
const inflate = (bytes) => new Response(new Blob([bytes]).stream().pipeThrough(new DecompressionStream('gzip'))).arrayBuffer();
const glueSrc = new TextDecoder().decode(b64bytes(GLUE_B64));
const wasmBytes = new Uint8Array(await inflate(b64bytes(WASM_GZ_B64)));"""
if old not in html:
    fail("wasm load anchor not found in src/index.html")
html = html.replace(old, new)

# ---- three.js: each module in the import map, as a data: URL ----
m = re.search(r'<script type="importmap">(.*?)</script>', html, re.S)
if not m:
    fail("import map not found in src/index.html")
imports = json.loads(m.group(1))["imports"]
by_path = {(src / path).resolve(): name for name, path in imports.items()}

# Bytes a data: URL may carry as they are. Everything else is percent-encoded,
# newlines and tabs too: a URL parser drops those, which would let a // comment
# run into the code after it.
SAFE = " !$&'()*+,-./:;=?@_~[]{}<>|^`\"\\"


def data_url(name, path):
    code = path.read_text()

    # A relative import cannot resolve from a data: URL; name the module instead.
    def bare(rel):
        target = (path.parent / rel.group(2)).resolve()
        if target not in by_path:
            fail(f"{name} imports {rel.group(2)}, which the import map does not list")
        return f"{rel.group(1)}{by_path[target]}{rel.group(3)}"

    code = re.sub(r"""(from\s+['"])(\.{1,2}/[^'"]+)(['"])""", bare, code)
    # Any other relative import (`import './x.js'`, `import('./x.js')`) would
    # fail only when the page runs; stop here instead.
    left = re.search(r"""\bimport\s*\(?\s*['"]\.{1,2}/""", code)
    if left:
        fail(f"{name} has a relative import the build does not rewrite: {left.group(0)}")
    return "data:text/javascript;charset=utf-8," + urllib.parse.quote(code, safe=SAFE)


inlined = {name: data_url(name, (src / path).resolve()) for name, path in imports.items()}
# Kept free of "<", so nothing in it can close the <script>.
map_json = json.dumps({"imports": inlined}, indent=1).replace("<", "\\u003c")
html = html[: m.start(1)] + "\n" + map_json + "\n" + html[m.end(1) :]

# ---- the version, and the title ----
slot = '<span class="ver" id="version"></span>'
if slot not in html:
    fail("version anchor not found in src/index.html")
version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
out = html.replace(slot, f'<span class="ver" id="version">v{version}</span>').replace(
    "<title>step-loupe — STEP viewer</title>",
    "<title>step-loupe — STEP viewer (single file)</title>",
)
(root / "step-loupe.html").write_text(out)
print(f"wrote step-loupe.html ({len(out.encode()) / 1e6:.1f} MB)")
