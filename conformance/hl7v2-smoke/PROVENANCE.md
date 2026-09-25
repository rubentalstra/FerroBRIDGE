<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Provenance: the HL7 v2 smoke corpora pass list

`pass-list.txt` records which HL7 v2 messages of the build-time sets the v2
face carries from an MLLP frame to an R4 Bundle, one case id per line, and the
message count on its `total` line. A case id is the set's directory under
`crates/ferrobridge-hl7v2/vendor/` and the file's path under it, with each
space written `%20`; an AIRA case adds `#<n>`, the message's place in its file.
It holds no copy of any input. The sets read, fetched at build time and never
committed:

- `nist/`: every `Message.txt` of the NIST LRI, LOI and syndromic
  surveillance test bundles.
- `aira-mqe/`: the first 25 messages of each AIRA MQE example file.

Both carry a committed `PROVENANCE.md`, written by
`scripts/vendor/hl7v2-samples.sh --build-time --stamp`.

The list is written by
`corpus::conformance_the_fetched_hl7v2_corpora_hold_their_pass_list` in
`crates/ferrobridge-hl7v2/tests/it/corpus.rs`, under
`scripts/checks/conformance.sh --update`, which fetches the sets first. The
verdict rule is the `hl7v2` corpus's. Never edit the list by hand.
