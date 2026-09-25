<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Provenance: the HL7 v2 message corpora pass list

`pass-list.txt` records which vendored HL7 v2 messages the v2 face carries from
an MLLP frame to an R4 Bundle, one case id per line, and the message count on
its `total` line. A case id is the set's directory under
`crates/ferrobridge-hl7v2/vendor/` and the file's path under it, with each
space written `%20`. It holds no copy of any input. The sets read:

- `fhir-converter/`: the Microsoft FHIR-Converter samples (MIT).
- `reportstream/`: the CDC ReportStream data tests (CC0-1.0).
- `v2-to-fhir/`: the HL7 v2-to-FHIR benchmark messages (Apache-2.0).

Each carries its own `PROVENANCE.md` and upstream `LICENSE`, vendored by
`scripts/vendor/hl7v2-samples.sh`.

The list is written by
`corpus::conformance_the_vendored_hl7v2_corpora_hold_their_pass_list` in
`crates/ferrobridge-hl7v2/tests/it/corpus.rs`, under
`scripts/checks/conformance.sh --update`. A case passes when its message is
framed, decoded, parsed with no refusal, mapped through the v2-to-FHIR guide,
and the Bundle decodes as R4; an acknowledgment passes when it parses. The
counted outcomes and the differences from an expected Bundle are in the result
the test writes, `target/conformance/hl7v2.json`, and never decide a verdict.
Never edit the list by hand.
