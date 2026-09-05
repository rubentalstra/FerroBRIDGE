<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Provenance: the mermaid browser assets

These four files are third-party material, vendored verbatim by
`scripts/vendor/mdbook-mermaid-assets.sh` and never hand-edited
(`.claude/rules/vendored-inputs.md`). They are not covered by the project's own
licence; each keeps the upstream terms recorded below.

The book's `mermaid` preprocessor rewrites a ```` ```mermaid ```` fence into a
`<pre class="mermaid">` block and leaves the drawing to the mermaid library in
the reader's browser, so `book.toml` loads both scripts through `additional-js`.
`website/book/src/evaluate/architecture.md` carries the diagram that exercises
them, and `mdbook build` fails when a listed `additional-js` file is missing.

| File | Upstream | Pin | Terms |
|---|---|---|---|
| `mermaid.min.js` | <https://github.com/badboy/mdbook-mermaid> `src/bin/assets/mermaid.min.js` (the mermaid 11.6.0 browser bundle it ships) | commit `99e2393a739f3c36fde30c59317d4daa355eefaf` (tag `v0.17.1`) | MIT, in `LICENSE-mermaid` |
| `mermaid-init.js` | <https://github.com/badboy/mdbook-mermaid> `src/bin/assets/mermaid-init.js` | commit `99e2393a739f3c36fde30c59317d4daa355eefaf` (tag `v0.17.1`) | Mozilla Public License 2.0, in `LICENSE-mdbook-mermaid` |
| `LICENSE-mdbook-mermaid` | <https://github.com/badboy/mdbook-mermaid> `LICENSE` | commit `99e2393a739f3c36fde30c59317d4daa355eefaf` | Mozilla Public License 2.0 |
| `LICENSE-mermaid` | <https://github.com/mermaid-js/mermaid> `LICENSE` | commit `7b2083926dbe3b6280f42376a0d4195b2c72fb8e` (tag `mermaid@11.6.0`) | MIT |

Fetched on 2026-09-05. The fetch script verifies the SHA-256 of every file and
fails when an upstream byte changes, so a silent substitution cannot land.

The pinned commit is the same release the docs toolchain installs, so the
vendored bytes and the `mdbook-mermaid` binary that consumes them stay in step:
`mdbook-mermaid` 0.17.1 in `docs/VERSIONS.md` and in
`.github/actions/docs-toolchain/action.yml`.

## Refreshing

Change the pins at the top of `scripts/vendor/mdbook-mermaid-assets.sh`, run
it, and commit the result with this file updated. The checksums it carries are
the ones the new upstream reports; take them from the failure message the
script prints.
