---
name: license-busl
description: "The project's own code and text are under the Business Source License 1.1 on FerroEHR's and FerroTERM's terms (owner decision 2026-09-04, #12), replacing the 2026-09-03 Apache 2.0 choice; Apache 2.0 is the Change License four years after each version"
metadata:
  node_type: memory
  type: project
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

The owner decided on 2026-09-04 (#12, merged in #13) that FerroBRIDGE's own
code and text are under the **Business Source License 1.1**, on the terms
FerroEHR and FerroTERM use. This replaced the Apache License 2.0 chosen on
2026-09-03 when the repository was created; that earlier choice is history,
not a second licence.

The terms, as `LICENSE` and `NOTICE` state them:

- Free to read, build, modify, and redistribute.
- Free for every non-production use and for non-commercial production use.
- A commercial licence from the Licensor for any other production use, always
  for a hosted, managed, or embedded service and for for-fee distribution.
- The Change License is Apache License 2.0, four years after each version.
- Contribution is inbound equals outbound under the same licence. There is no
  contributor licence agreement and no copyright assignment.

**How to apply:**

- Every first-party file carries `SPDX-FileCopyrightText: Ruben Talstra` and
  `SPDX-License-Identifier: BUSL-1.1` in its header. The pin-matrix guard
  (#15) fails on a stale licence claim anywhere in the tree.
- `LICENSE` is the one file that names Apache 2.0 as a licence of its own,
  where it is the Change License. Nowhere else describes the project as
  Apache-licensed.
- `deny.toml` (#20) lists BUSL-1.1 for the workspace's own crates only; the
  dependency allowlist is a separate decision.
- Vendored specifications and third-party material keep their upstream terms,
  recorded in a `PROVENANCE.md` beside each vendored tree
  (`.claude/rules/vendored-inputs.md`).
- The licence is decided per repository by the owner and never assumed from a
  sibling (see `sibling-projects.md`).
