---
name: openehr-crates-are-the-model
description: Never restate openEHR RM, AM, BASE or FLAT knowledge in this repository; read it from the openehr-* crates (write a value with the FLAT |raw suffix, look attributes up in openehr_rm::v1_2::model); owner flag 2026-09-25, audit and refactor #241
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

On 2026-09-25 the owner: "i see that we are doing some double work regarding
AM or RM or BASE. we have crates for that you know right?" An audit found the
FLAT suffix grammar restated per class in the engine (six attributes lost
silently), a `CARRIED` attribute table beside `openehr_rm::v1_2::model`, the
LINK, PARTICIPATION and FEEDER_AUDIT families spelled by hand, and one live
drift (`rm_version = "1.1.0"` on a `v1_2` EHR_STATUS). #241 removed what the
crates carry; the sibling requests S1 to S6 on #241 cover the rest.

**Why:** the crates are generated from the BMM and the Simplified Formats
text; a local copy drifts silently and the bridge then writes a wrong record.

**How to apply:** a value goes into a composition as one `|raw` part carrying
`openehr_its::json::to_canonical_value` of the `openehr_rm` type, never as
hand-spelled suffixes; a class or attribute fact comes from
`openehr_rm::v1_2::model::{attribute, descendants}`; a path is parsed by
`openehr_rm::v1_2::paths`; FLAT keys are built from
`openehr_sdt::flat::path::{Segment, Suffix, FlatKey}`; a version string comes
from `openehr_rm::Generation`. When the crate lacks a public reader, keep the
local code minimal under `// TODO(#241)` naming the sibling request, and never
widen it. Before a new module touches openEHR data, grep the crate for the
item first.
