# Releasing

step-loupe is not published to crates.io. A **release** is a git tag plus a
GitHub Release with the self-contained `step-loupe.html` attached, and the
hosted demo refreshed on GitHub Pages.

## Branch Strategy

- **`main`** — source, docs, and release tags. Development happens here directly.
- **`gh-pages`** — the deployed site (`index.html` bundle + `nist-ctc05.step`).
  Force-pushed by `scripts/deploy-gh-pages.sh`; **never edit it by hand.**

## Release Checklist

### 1. Finalize version

Remove the `-dev` suffix from `version` in `Cargo.toml` (`X.Y.Z-dev` → `X.Y.Z`).

### 2. Update CHANGELOG.md

- Move `[Unreleased]` items into a new `[X.Y.Z] - YYYY-MM-DD` section.
- Add/refresh the comparison links at the bottom.

### 3. Run checks

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
wasm-pack build --target web --release
```

### 4. Build the single-file bundle

```sh
python3 scripts/build_single.py   # -> step-loupe.html (self-contained)
```

### 5. Create the release commit and tag

```sh
git commit -am "release: vX.Y.Z"
git tag vX.Y.Z
```

### 6. Push commit and tag

```sh
git push origin main --tags
```

### 7. Create the GitHub Release (attach the bundle)

```sh
gh release create vX.Y.Z step-loupe.html --title "vX.Y.Z" --notes-file <(
  sed -n '/^## \[X\.Y\.Z\]/,/^## \[/{ /^## \[X\.Y\.Z\]/d; /^## \[/d; p; }' CHANGELOG.md
  echo "**Full Changelog**: https://github.com/elgar328/step-loupe/compare/vA.B.C...vX.Y.Z"
)
```

> Replace `vA.B.C` with the previous release tag. `step-loupe.html` becomes the
> downloadable, offline single-file viewer.

### 8. Deploy the hosted demo

```sh
./scripts/deploy-gh-pages.sh
```

First time only — enable Pages on the `gh-pages` branch:

```sh
gh api -X PUT repos/elgar328/step-loupe/pages \
  -f 'source[branch]=gh-pages' -f 'source[path]=/'
```

Demo URL: <https://elgar328.github.io/step-loupe/?file=nist-ctc05.step>
(example = NIST CTC 05, AP242 e1, public domain).

### 9. Start the next development cycle

Bump `version` in `Cargo.toml` to the next `X.Y.(Z+1)-dev`, then:

```sh
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
