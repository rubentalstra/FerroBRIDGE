---
name: owner-work-style
description: How the owner wants FerroBRIDGE work done (research-first, evidence-based, from first principles, confirm foundational decisions before scaffolding)
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

On FerroBRIDGE, the owner decides foundational architecture from **cited
research (academic papers included), not convention**, and is explicitly
skeptical of the legacy way others build this kind of software.

**Why:** the foundation is treated as the most important thing to get right
from the start. **How to apply:**

- For any foundational choice, do proper research and put the evidence in front
  of the owner BEFORE building. Present options with a recommendation; do not
  default to the conventional answer.
- Do NOT scaffold code or a workspace while the design is still open. On
  FerroTERM the owner stopped an early scaffold with "still in the discover
  phase". Build only after the direction is confirmed. FerroBRIDGE is in
  exactly that phase now (`CLAUDE.md` §Status).
- Take the owner's design intuitions seriously and test them against evidence.
  On FerroTERM the owner's graph-model call was correct and the research
  refined how to implement it. Reconcile rather than dismiss.
- Pure Rust, memory-safe, lightweight, single binary are standing constraints
  across the Ferro family.

Same defer-nothing, proper-rewrites-welcome spirit as on FerroEHR and
FerroTERM.
