---
name: file-length-rule
description: Owner rule (2026-09-26) that a hand-written Rust file is at most 1000 lines, split into module folders at 750; enforced by scripts/checks/file-length.sh with a ratchet allow-list
metadata:
  type: feedback
---

On 2026-09-26 the owner ruled that hand-written files had grown out of hand
(the v2 interpreter's `run.rs` at 4600 lines, the FHIRconnect traverser at
3834): a hand-written Rust file is at most 1000 lines, and one over 750 is
split into a module folder (`<name>/mod.rs` plus one child per concern)
before it grows. Generated files are outside the rule. Neither FerroTERM nor
FerroEHR enforces such a cap; this is FerroBRIDGE's own rule.

**Why:** a 4000-line file cannot be reviewed, and every worker that touched
one grew it further; the split is the only thing that keeps the code
readable by the next person.

**How to apply:** `scripts/checks/file-length.sh` runs in the CI guard tier;
`scripts/checks/file-length-allow.txt` lists each known breach with the
sub-issue of program #351 that splits it, and a listed file may not grow.
When a worker touches a listed file, the brief says to split it first, or
at least not to grow it. A split moves code only; a defect found on the way
is filed ([[legacy-standards-first-class]] for the v2 crate's history).
