<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# ferrobridge-hl7v2

The HL7 v2 face's crate, hand-written over two generated layers: the v2
message structures and segment tables of `hl7v2-types`, and the FHIR R4 model
and element table of `fhir-types`. The oracles are HL7 v2 (chapter 2 for the
encoding rules and the acknowledgment; the `HL7/v2ig` definitions the tables
are generated from), the MLLP Release 1 transport, HL7 table 0211 for the
character sets, the v2-to-FHIR implementation guide
(`hl7.fhir.uv.v2mappings` 1.0.0 and its `mapping_guidelines.md`, vendored
under `tools/fhir-codegen/vendor/hl7.fhir.uv.v2mappings/`), and FHIR R4.

## The interpreter is an adapter over the corpus and the engine's writer

The ConceptMaps are data. Nothing here maps a message, segment or table by
hand: the message map names a resource per segment, the segment map maps the
fields, a data type map maps the components, a table map goes through
`ferrobridge_term::client::Client::translate`, and every FHIR element is
written through `fhirconnect::tree::write::write` over
`fhir_types::r4::schema::SCHEMAS`. Never add a second FHIR writer, a table
evaluator, or a per-message code path. A gap in the corpus is a counted
outcome here and a ConceptMap in a supplement directory (#256), in the guide's
own shape.

## Selection by name is a recorded design, flagged in the code

The guide names neither the segment map a message row runs nor the data type
map a field row runs. The interpreter selects `segment-<seg>-to-<resource>`
and `datatype-<source>-to-<target>`, else the single qualified map
`<kind>-<source>-<qualifier>-to-<target>`, and counts `no-segment-map` or
`no-datatype-map` when there is none or several (`Corpus::find`). Keep that
rule in one place. A row into a complex element is the one exception: it
runs every qualified map (`Corpus::qualifying`), named for the element's type
or else its element path (`messageheader-source`), each into its own
children (`Run::datatypes`). A child another row of the same source targets
directly is left to that row (`Run::claimed`), unless that row only reaches
the bridge's endpoint fallback (`Run::falls_back`). Two maps writing one child
with different values roll the row back as `datatype-conflict`; with the same
values they agree and the first map's writes stand. Before that, maps that
are alternatives for one value (`Map::alternative_to`: one component mapped
with no condition into the same child or `$value`, as the four EI variants
into `Identifier`) are narrowed to one (`Run::select`): the one whose rows
write from the most components of the value, then among those that write
the same the one with the fewest rows; equally specific maps that write
differently count `datatype-ambiguous` and none runs. Maps that are halves
of one value (`Map::half_of`: each maps, with no condition, into the same
child from components the other never reads, as the EIP placer and filler
maps) are narrowed by the row's place (`Run::halve`): the rows of its map
from the same source into the same element modulo labels take the halves
in component order, counted `datatype-half`; a value valuing a component no
half reads keeps every half, so the row is a `datatype-conflict`. A row
whose `mappedVia` names a data type map runs that map alone. A `(Type)` row
whose one inner step is a `Reference` with no map runs the map from its
source into the referenced resource when all its rows write through that
step (`Run::reference_root`, `reference-root`), and a primitive at `$value`
of a complex element lands in its `value` child (`value-child`).

## Instances are identities, allocated when a value first reaches them

`[n]` labels are identities (`mapping_guidelines.md` §\[n\] Notation), so a
write records an instance key per step and `Run::build` turns keys into array
indices in the order values arrive, all or nothing per write. A resource
instance from the message map is keyed by its label and the occurrences of the
repeating nodes above its segment, and a later segment reuses the instance
whose key is the longest prefix of its own. Never write a positional index
from a label.

## Strict at every seam

A condition in a form outside the grammar in `map::condition`, a check that
names no operand (`defective-condition`), a target outside `map::notation`,
an assignment outside the concatenation grammar or joining two parts with no
`+` (`defective-assignment`), a `Narrative-Condition`, a value a FHIR primitive cannot hold (a time with no
offset is no `dateTime`), and a resource that does not decode as R4 are each
a typed `Outcome`, counted and never dropped. Before a value is written,
`map::constraint` checks it against its primitive's lexical form through the
`fhir-types` decoder; after the writes, it drops every element and resource
lacking an element the element table marks required, where a primitive
counts as present by its value or by its `_name` sibling holding an
extension (R4 JSON §Primitive Types). Both read the
constraints from `fhir-types` and name no resource or element, so the Bundle
decodes. A `message` Bundle without its `MessageHeader` breaks `bdl-12`,
so `Run::finish` refuses it (`MapError::NoMessageHeader`); `inbound` refuses
no message for its sender, since the guide's MSH-24 row writes a
data-absent-reason endpoint when MSH-3 and MSH-24 are empty. With
MSH-3 and MSH-24 empty, `Run::facility_endpoints` writes the source endpoint
from MSH-4, and the destination's from MSH-6, as a `facility-endpoint`
outcome, replacing the data-absent-reason the guide's MSH-24 and MSH-25
rows write there. A row runs on its empty source only when it assigns a
literal and its condition requires that source `NOT VALUED`
(`condition::requires_absent`). The guide's own rows write the typed
`urn:oid:`, `urn:uuid:`, `urn:dns:` and `urn:uri:` endpoints. An `HD` into a
`url` that no row of the guide fills goes through `convert::endpoint`, the
one place the derived `urn:ferrobridge:hl7v2-hd:` form is built, written as
a fallback that a row of the guide at the same element replaces
(`Run::write`). A refused or failed translation
fails the run with the upstream status (`MapError::Terminology`); an absent
terminology server is `no-terminology`, never a code passed through.

The condition, assignment and target grammars are `logos` lexers under
`chumsky` parsers, the pair `openehr-query` builds its AQL parser on. The
condition grammar reads the guide's comparison forms (`=`, `EQUALS`, `IS`,
`NOT EQUALS`, `IN`, `NOT IN`, the counts and lengths) over a field
(`PID-3.1`), a component (`HD.3`, or `HD-3` when the name before `-` is a
data type and no segment) or a segment, and an `IN`, `NOT IN`, `VALUED` or
`NOT VALUED` with no operand reads the row's own source
(`condition::parse_for`; `mapping_guidelines.md` §Conditions lists the two
`IN` forms, and the `VALUED` forms are our own reading); the assignment
grammar is literals
and operands joined by `+` (`"urn:oid:"+HD.2`), where an operand with no
value joins as the empty text. `scripts/guide-forms.sh` lists every
condition and assignment form the package writes, with counts. The v2 message
splitter is hand-written, because MSH-1 and MSH-2 declare the delimiters at
run time.

## The generated tables are read, never shadowed

A shape `hl7v2-types` lacks (the event-to-structure table, data type
components, `PartialEq` on the tree types) is a generator follow-up, never a
local table here. `parse::structure_for` refuses a structure the definitions
carry in variants, and the caller names the variant. An MSH-9.3 naming no
structure falls back to the message index, counted as `other-structure`, and
then to a structure v2.9.1 withdrew (`ORM_O01`) from `hl7v2_types::legacy` by
MSH-12 (#303), counted as `withdrawn-structure`; a legacy tree is never
written here. A legacy field's type code the guide has no data type map
for (`CM_MSG`, `CE_0051`) takes the base type the generated
`LegacyDataType` links it to before the map lookup, counted as
`base-typed`; a code the guide maps keeps its map (`TS` keeps
`datatype-ts-to-datetime`). The link is the generator's, never a table
here. A segment whose group path no row names
takes the rows of the one source whose groups differ from its own by one
group, not the group holding the segment (`run::regrouped`), counted as
`group-path`: the guide's `ORM_O01.ORDER_DETAIL.CHOICE.OBR` reaches the 2.3
`ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR`; a source that reaches two tree
paths, or a path two sources reach, stays `unmapped-segment`. A group the
row names that the tree names otherwise at its position pairs with that
tree group when the tree group holds every segment the guide's rows place
in the row's group (`run::pairing`), counted as `group-renamed` naming
both: the guide's `ORM_O01.PATIENT.VISIT.PV1` reaches the 2.3
`ORM_O01.PATIENT.PATIENT_VISIT.PV1`. The pairing with the fewest renames
counts, and two at the fewest stay unmapped. The rule holds for v2.9.1
trees too, since the guide's paths disagree with the v2.9.1 definitions as
well (`OML_O21`, and `MDM_T02`, whose observation group is `FIXME`).

## Logs

Libraries speak `tracing`, and no message content reaches a log: the listener
logs the peer (the `mllp_connection` span), the refusal kind and task
failures, never a byte of a frame. `mllp::serve_with` takes the idle and the
frame timeout; a frame that stalls is refused as `Malformed::Stalled`, so the
caller answers it as any malformed frame.

## Tests

`tests/it/` drives the public surface: decoding per character set, parsing
against the generated structures, the ACK codes on the wire against a plain
`TcpStream` sender, and the interpreter over the vendored package with the
table maps answered by a `wiremock` terminology stub that answers from the
guide's own table maps. The bundles and outcome lists are reviewed `insta`
snapshots. Every fixture is synthetic (`tests/it/fixtures.rs`).
