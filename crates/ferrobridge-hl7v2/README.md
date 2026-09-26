<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# ferrobridge-hl7v2

The FerroBRIDGE HL7 v2 face: MLLP framing, MSH-18 decoding, positional
parsing over the generated v2 tables, the acknowledgment, and the
v2-to-FHIR ConceptMap interpreter.

Part of [FerroBRIDGE](https://ferrobridge.eu), a pure-Rust bridge between
openEHR and two interoperability targets: HL7 FHIR through the FHIRconnect
specification, and the OMOP Common Data Model through the OMOCL specification.

## What it does

- `mllp`: the MLLP Release 1 frame codec (`<SB> message <EB><CR>`) on the
  `tokio-util` codec traits, and a listener that answers each frame through a
  handler and drains on a shutdown signal. A frame with no start block, a
  start block inside it, an end block without its carriage return, or a size
  past the ceiling is refused, and the handler may answer it with `AR` before
  the connection closes.
- `decode`: the character set MSH-18 names (HL7 table 0211): ASCII, ISO 8859
  parts 1 to 9 and 15, and UTF-8, with a per-connection default for an empty
  MSH-18. A byte outside the declared set is refused, never replaced.
- `parse`: segments, fields, repetitions, components and subcomponents by the
  MSH-1 and MSH-2 delimiters, the escape sequences decoded, and the segments
  placed in the segment-group tree of the message structure from
  [`hl7v2-types`](https://docs.rs/hl7v2-types). A Z-segment, a segment out of
  place and a field past its segment's table are counted outcomes; a missing
  required segment or field is a refusal.
- `ack`: the original-mode acknowledgment, `AA`, `AE` with one `ERR` per
  refusal, or `AR`, echoing MSH-10 in MSA-2.
- `inbound`: the steps above for one message, settling the `AR` and `AE`
  answers a message owes before it is mapped.
- `map`: the interpreter of the `hl7.fhir.uv.v2mappings` ConceptMaps, loaded
  from a directory at run time (the corpus is never packaged). The message
  map names a resource per segment, the segment map maps the fields, a data
  type map maps the components, and a table map is answered by
  `ConceptMap/$translate` on a terminology server. The FHIR side is written
  through `fhirconnect::tree` over the `fhir-types` R4 element table, and the
  output is an R4 message `Bundle`. Every condition, target or value the run
  cannot carry is a typed, counted outcome.
- `map::supplement`: FerroBRIDGE's own ConceptMaps in the guide's shape,
  packaged with the crate under `supplements/`, which override a guide map
  or add one it lacks; `Corpus::with_shipped_supplements` loads them over the
  guide, and the run counts each one it uses as `supplemented`.

## Status

The crate version is a placeholder while the crate line settles; the tracker
carries the build order.

## Licence

Business Source License 1.1 (`LICENSE`): free for every non-production use and
for non-commercial production use; a commercial licence for other production
use; Apache License 2.0 four years after each version.
