---
paths: ["scripts/vendor/*.sh", "**/vendor/**", "docs/specs/**"]
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Vendored inputs

External material enters this repository one way only: a committed fetch
script, vendored verbatim, stamped with provenance. Nothing is vendored yet,
because the research on issue #1 has not pinned any version. The rule stands
from the first vendored byte.

## The rule

Every external corpus (a specification's machine-readable artifacts, a mapping
schema, a test corpus, a grammar) is:

- **Fetched by a committed `scripts/vendor/*.sh` script.** Never hand-download
  into the tree, never hand-edit a vendored file, and never paste material in
  from a chat transcript. To refresh or extend a corpus, change the script,
  re-run it, and commit the result. A hand-edit of a vendored file is a defect
  to revert.
- **Vendored verbatim**, byte for byte as the publisher ships it. Reformatting,
  pretty-printing, or trimming a vendored file destroys the property that makes
  it checkable against its source.
- **Stamped with a `PROVENANCE.md`** in its own directory, recording the
  upstream source (the URL or registry), the exact version or commit pin, the
  fetch date, and the upstream licence, with the upstream `LICENSE` vendored
  alongside. A vendored tree with no provenance is unusable: nobody can tell
  what it is or whether it may be redistributed.
- **Exercised.** A vendored input is not done until something reads it: a
  codegen drift check, a schema-validation test, or a corpus test. An unread
  corpus is dead weight that rots without anyone noticing.
- **Marked in `.gitattributes`** as `linguist-vendored`, and `-text` where the
  bytes must not be line-ending normalized, so the tree matches the upstream
  archive exactly.

## Licensing

Vendored material keeps its upstream terms, and those terms are recorded in the
`PROVENANCE.md` rather than assumed. The project's own code and text are
under the Business Source License 1.1 (`CLAUDE.md` §Licence); a vendored tree
is not, and the two are never conflated. If a corpus's licence does not permit redistribution, it is
not vendored: the fetch script pulls it into an ignored directory at build time
and the repository ships none of it.

## Never commit clinical or patient data

No patient data, no identifiable health information, and no extract from a
production system belongs in this repository, in a fixture, or in a vendored
tree. Test fixtures are synthetic content invented for the test
(`testing.md`). A licence-gated code system or clinical corpus is a deployment
input, never a committed artifact, and `.gitignore` refuses its shapes so a
copy dropped into a working tree cannot be committed by accident. Widen
`.gitignore` in the same change when a genuinely new shape appears.
