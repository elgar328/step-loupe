#!/bin/bash
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# Deploys the pre-built single-file bundle to the gh-pages branch as a clean,
# orphan-style single commit (build it first with scripts/build_single.py).
if [[ ! -f step-loupe.html ]]; then
  echo "Missing step-loupe.html (run 'python3 scripts/build_single.py' first)"
  exit 1
fi

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

origin_url="$(git remote get-url origin)"
git -C "$tmpdir" init -b gh-pages
git -C "$tmpdir" remote add origin "$origin_url"

cp step-loupe.html "$tmpdir/index.html"
cp sample/nist-ctc05.step "$tmpdir/nist-ctc05.step"
touch "$tmpdir/.nojekyll"

git -C "$tmpdir" add .
git -C "$tmpdir" commit -m "deploy step-loupe" --allow-empty
git -C "$tmpdir" push -f origin gh-pages

echo "Deployed to gh-pages"
