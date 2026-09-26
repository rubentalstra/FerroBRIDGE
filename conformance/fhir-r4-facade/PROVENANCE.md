<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Provenance: the FHIR R4 facade pass list

`pass-list.txt` records which resources of HL7's `hl7.fhir.r4.examples`
package (4.0.1) the facade creates and reads back through a mapping context,
one `<context>/<file>` id per line, and the case count on its `total` line. It
holds no copy of any input. The package is the one the `fhir-r4` corpus reads,
fetched at build time into `tools/ferrobridge-testkit/vendor/`. The contexts
are the facade suite's own (`ferrobridge_facade`, over the synthetic
diagnosis template of the testkit) and the KDS diagnosis project
(`kds_diagnose`, over the published `KDS_Diagnose` template); each example of
a resource type one of them maps is a case for it, and every other example is
counted by type in the result file's `set_aside`, never in the list.

A case passes when the create answers `201`, the read answers `200` with an
instance the R4 model decodes, no element the example carries comes back with
another value (`PutGet`), and a second create and read of that answer gives it
back unchanged. The identity, `meta` and the subject are the facade's own and
leave every comparison.

The list is written by
`facade::examples::conformance_the_fhir_r4_examples_hold_their_facade_pass_list`
in `app/ferrobridge-server/tests/it/facade/examples/mod.rs`, under
`scripts/checks/conformance.sh --update`. Never edit the list by hand.
