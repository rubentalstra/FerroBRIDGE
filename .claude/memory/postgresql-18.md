---
name: postgresql-18
description: Every PostgreSQL the project tests against or documents is the latest release, PostgreSQL 18.6 today, never 16; owner ruling 2026-09-12
metadata:
  type: project
---

<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

Owner ruling 2026-09-12: the CDM database in tests, the container harness, the
quickstart `compose.yaml` and the documentation use the latest PostgreSQL
release, 18.6 at the time of the ruling, pinned by tag and digest in
`docs/VERSIONS.md`. The "PostgreSQL 16" the second research pass wrote into
the architecture and into #73 and #91 had no decision behind it and is
corrected.

**How to apply:** one pin row in `docs/VERSIONS.md`, one constant in the
testkit that `versions.sh` checks, and every issue or page naming a PostgreSQL
version names that row. Bump it when a new release ships, as a pin change.
