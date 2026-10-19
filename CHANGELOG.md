# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Draw the edges and the vertices as a few shared objects instead of one
  object each, so a large file turns smoothly. On a 110 MB assembly (31,000
  edges, 20,000 vertices) the view redraws at the display's 60 fps instead of
  11 fps, selecting a solid shows in 10 ms instead of 170 ms, and the page
  holds 63 MB of JavaScript heap instead of 269 MB.
- Read the file in a background thread (a Web Worker, one per file), so the
  page no longer freezes while a large file opens, and the memory the read
  took is returned once it is done. On the same assembly the longest stall
  drops from 2.1 s to 0.1 s, and the tab keeps about 400 MB less memory
  afterwards. A read that fails, even by a crash inside the reader, now shows
  its error, and opening another file meanwhile cancels the read.
- Read the file piece by piece instead of copying all of it in first, so a
  read needs about the file's size less memory: on the same assembly the
  reader peaks at 331 MB instead of 436 MB, and the tab keeps about 100 MB
  less afterwards. A file that cannot be read, such as one deleted or changed
  since it was picked, now shows an error instead of nothing.
- Send the scene tree with each node once: an edge shared by two faces, or a
  vertex shared by its edges, was copied under each of them. On the same
  assembly the reader peaks at 243 MB instead of 331 MB, the page holds 18 MB
  of JavaScript heap instead of 39 MB once the file is open, and it opens in
  2.3 s instead of 2.5 s. On a 63 MB file with many more faces, the reader
  peaks at 464 MB instead of 859 MB.
- The build now uses `wasm-pack build --target no-modules`.

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

[Unreleased]: https://github.com/elgar328/step-loupe/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/elgar328/step-loupe/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/elgar328/step-loupe/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/elgar328/step-loupe/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/elgar328/step-loupe/releases/tag/v0.1.0
