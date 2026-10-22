# Releasing

step-loupe is not published to crates.io. A **release** is a git tag plus a
GitHub Release with the single-file `step-loupe.html` attached, and the
hosted demo refreshed on GitHub Pages.

## Branch Strategy

- **`main`** — source, docs, and release tags. Development happens here directly.
- **`gh-pages`** — the deployed site (`index.html` bundle + `nist-ctc05.step`).
  Force-pushed by `scripts/deploy-gh-pages.sh`; **never edit it by hand.**

## Release Checklist

### 1. Finalize version

Remove the `-dev` suffix from `version` in `Cargo.toml` (`X.Y.Z-dev` → `X.Y.Z`).
`Cargo.lock` follows on the next cargo command (step 3); commit both.

### 2. Update CHANGELOG.md

- Move `[Unreleased]` items into a new `[X.Y.Z] - YYYY-MM-DD` section.
- Add/refresh the comparison links at the bottom.

### 3. Run checks

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
wasm-pack build --target no-modules --release
```

### 4. Build the single-file bundle

```sh
python3 scripts/build_single.py   # -> step-loupe.html
```

### 5. Check in a browser

Open `step-loupe.html` from file:// and over http (`python3 -m http.server`),
in Safari and in a Chromium browser, and open a file with the button and by
dropping it. From file://, also open one with the network off. Over http, also open `?file=sample/nist-ctc05.step`, the same with
`&up=z`, and a `?from=` hand-over: in the page's console run
`localStorage.setItem('step-loupe:t', JSON.stringify({name: 'a.step', text: await (await fetch('sample/nist-ctc05.step')).text(), at: Date.now()}))`,
then open `?from=t`.

### 6. Create the release commit and tag

```sh
git commit -am "release: vX.Y.Z"
git tag vX.Y.Z
```

### 7. Push commit and tag

```sh
git push origin main --tags
```

Wait for CI to pass before creating the release.

### 8. Create the GitHub Release (attach the bundle)

```sh
gh release create vX.Y.Z step-loupe.html --title "vX.Y.Z" --notes-file <(
  sed -n '/^## \[X\.Y\.Z\]/,/^## \[/{ /^## \[X\.Y\.Z\]/d; /^## \[/d; p; }' CHANGELOG.md
  echo "**Full Changelog**: https://github.com/elgar328/step-loupe/compare/vA.B.C...vX.Y.Z"
)
```

> Replace `vA.B.C` with the previous release tag. `step-loupe.html` becomes the
> downloadable single-file viewer.

### 9. Deploy the hosted demo

```sh
./scripts/deploy-gh-pages.sh
```

Demo URL: <https://elgar328.github.io/step-loupe/?file=nist-ctc05.step>
(example = NIST CTC 05, AP242 e1, public domain). Open it and check that the
file loads.

### 10. Start the next development cycle

Bump `version` in `Cargo.toml` to the next `X.Y.(Z+1)-dev`, then:

```sh
cargo check   # so Cargo.lock follows
git commit -am "chore: start next development cycle (X.Y.Z-dev)"
git push
```

## Versioning (SemVer)

Follows [Semantic Versioning 2.0.0](https://semver.org/): **MAJOR** (incompatible
changes), **MINOR** (new features), **PATCH** (fixes). While the version is
`0.x.y`, minor bumps may include breaking changes.

## CHANGELOG Guidelines

Follow the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format.

- **Categories** (in this order): Added, Changed, Deprecated, Removed, Fixed, Security.
- Write entries from the user's perspective; each a concise, complete sentence.
- Most recent release first; always keep an `[Unreleased]` section at the top.
- Order the category sections as listed above; omit empty ones.
- Leave out figures that depend on the machine or the file (seconds,
  megabytes); say what got better.

## README screenshot

The screenshot lives in the `assets` release, apart from the versions; replace
it with `gh release upload assets screenshot.webp --clobber`.
