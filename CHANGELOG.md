# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- A file opens framed to the points it draws rather than to their bounding
  box, so the model fills the view, with a small margin at the edges.
- A new, dark look: one theme across the sidebar and the view, the app's
  name at the top, and a welcome card to drop a file on or try the sample.
  PMI lettering and other wireframes are drawn in amber, apart from the
  edges, and vertices show only when picked.
- While something is highlighted, the rest of the model fades back, so a
  small part stands out.
- Clicking a row in the tree selects it without opening or closing it; its
  arrow does that.

### Fixed

- Lines are as thick on a high-density (Retina) screen as on any other; they
  used to be half as thick there.

## [0.1.4] - 2026-10-21

### Added

- Show surface bodies (`SHELL_BASED_SURFACE_MODEL`) and wireframes
  (`GEOMETRIC_SET`, `GEOMETRIC_CURVE_SET`). Geometry that no part holds is
  listed under **Unplaced geometry** (was **Unplaced solids**).
- The report lists the parser's warnings, and each normalization note names
  its entity.
- An edge shows its own line style, falling back to its curve's.

### Changed

- Bump `step-io` to 0.3.0.
- Large files open faster, use much less memory and turn smoothly: the file
  is read piece by piece in a background thread, and edges and vertices are
  drawn as a few shared objects. The page no longer freezes while a file
  opens, and opening another file cancels the read in progress.
- Faces on degenerate tori and offset surfaces, and hyperbola and parabola
  curves, show their own kind instead of `Other`.
- Text written with Part 21 escapes (`\X2\…\X0\` and the like) shows the
  characters they stand for.
- A face shows its `orientation` instead of `same sense`.
- The header shows the schema only as precisely as the file gives it, such
  as `AP214` instead of `AP214 ed? (?)`.
- The report shows each warning once.
- The build uses `wasm-pack build --target no-modules` (was `--target web`).

### Fixed

- Curves no longer cut corners: a polyline or B-spline could skip a stretch
  between two samples.
- Edges and faces built on a parabola now show, and edges on parabolas and
  hyperbolas follow the curve instead of a dashed straight line.
- Components of SolidWorks assemblies, and of others that order placements
  the same way, are placed correctly.
- A part defined by a `PRODUCT_DEFINITION_WITH_ASSOCIATED_DOCUMENTS` no
  longer shows twice in the tree.
- A file with broken B-spline values, or a circle trimmed by a huge angle,
  opens instead of crashing or hanging.
- A read that fails, even by a crash in the reader, shows its error instead
  of nothing.
- A UTF-8 file with one broken character no longer has all its text read as
  ISO 8859-1.

## [0.1.3] - 2026-10-06

### Added

- Switch the view's up axis between Y and Z from the **Y↑** button in the
  view's corner, or open with `?up=z`; nacre-playground opens its parts Z-up.

## [0.1.2] - 2026-10-05

### Added

- Open a STEP file handed over by
  [nacre-playground](https://github.com/elgar328/nacre-playground): its
  **Open in step-loupe** button shows the part here without a download.

### Changed

- Bump `step-io` to 0.2.5.

## [0.1.1] - 2026-07-09

### Added

- Read solids with internal voids (`BREP_WITH_VOIDS`): a void solid now appears
  in the tree with its outer shell, and each internal cavity is shown as a
  `Void` group of its faces (requires step-io 0.2.3).

### Changed

- Bump `step-io` to 0.2.3.

## [0.1.0] - 2026-07-07

### Added

- Single-file, in-browser STEP (ISO 10303) viewer built on
  [step-io](https://github.com/elgar328/step-io): b-rep geometry, the assembly tree,
  PMI (features, dimensions, tolerances, datums), and a provenance report,
  rendered with three.js.
- Load a model via the `?file=<url>` query parameter, the **Open STEP file**
  button, or drag-and-drop; responsive layout for narrow (mobile) screens.

[Unreleased]: https://github.com/elgar328/step-loupe/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/elgar328/step-loupe/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/elgar328/step-loupe/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/elgar328/step-loupe/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/elgar328/step-loupe/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/elgar328/step-loupe/releases/tag/v0.1.0
