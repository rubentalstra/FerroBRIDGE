---
name: license-apache
description: "Owner decision 2026-09-03: FerroBRIDGE stays under the Apache License 2.0 while FerroEHR and FerroTERM moved to BUSL 1.1 the same day; the reason is that the bridge is meant to be adopted widely"
metadata:
  node_type: memory
  type: project
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

The licence of FerroBRIDGE's own code and text is the **Apache License 2.0**,
decided by the owner on 2026-09-03 at the repository's creation. On that same
day FerroEHR and FerroTERM moved to the Business Source License 1.1. The bridge
is deliberately the exception.

**Why:** a bridge is only worth building if people put it in the path between
their systems, so the licence must not be a reason to refuse it. Apache 2.0
also carries the patent grant and the contribution terms (section 5) that MIT
lacks, which is what makes inbound-equals-outbound work without a contributor
licence agreement.

**How to apply:**

- Every first-party file carries `SPDX-FileCopyrightText: Ruben Talstra` and
  `SPDX-License-Identifier: Apache-2.0` in its header (shell and YAML use `#`,
  markdown uses an HTML comment, JSON has no comment syntax so it is skipped).
- Vendored third-party material keeps its upstream terms, recorded in a
  `PROVENANCE.md` beside it (`.claude/rules/vendored-inputs.md`). Never
  conflate the two.
- There is no contributor licence agreement and no copyright assignment.
  Contributors keep their copyright.
- A licence question is the owner's to decide. Present options with trade-offs,
  and expect the answer to be revisited. Do not copy a sibling project's
  licence change into this repository on the assumption it applies here.
