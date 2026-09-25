# fhirconnect

The FHIRconnect half of the bridge, hand-written over the shared foundation in
`openehr-mapping-core` and the generated FHIR model in `fhir-types`. The
FHIRconnect specification and the HL7 FHIR specification for the version in
play are the oracles; openFHIR and the retired CDR connector are prior art.

## The document model is `fhir_types::codec::Value`, never `serde_json::Value`

FHIR forbids reading a decimal through a binary float
(<https://hl7.org/fhir/R4/datatypes.html#decimal>), so every tree this crate
reads or writes is the lexical `Value` of `fhir-types`, which keeps a number in
the text the document carried. A `serde_json::Value` conversion belongs at the
HTTP edge and nowhere inside the model. Never introduce a second JSON model,
and never route a clinical value through `f64`.

## The element table is the authority, and it is generated

What an element is, whether it repeats, which types a choice admits and where a
content reference lands all come from `fhir_types::schema::Schemas`, the element
table the generator emits from the vendored HL7 packages. That includes the two
members the JSON representation gives a primitive's sibling object, `id` and
`extension`, which the table carries as its own `Element` entry
(<https://hl7.org/fhir/R4/element.html>). Nothing here restates a fact about
FHIR that the table already carries. A gap in the table is a generator change
plus a regeneration, never a hand-written constant in this crate
(`.claude/rules/codegen.md`).

`tree::element::Table` is what the path model asks of a version's table, so
R4B and R5 plug in through the same trait. Only R4 is wired today.

## Writable or read-only is decided when the expression is parsed

A step that selects among the values a document already holds cannot be written
through: `where()`, `first()`, `last()`, an index filter and `resolve()` all
make the whole expression read-only, and a write through one is a typed
refusal naming the offending step. `extension(url)` stays writable, because the
url is the identity of the extension and so names one element to create
(<https://hl7.org/fhir/R4/extensibility.html>).

The point is a defect the retired CDR connector had: it let filtering paths
through and its reverse direction then wrote nothing, silently.

## A refusal names the element, never only the expression

Every refusal from `tree::element`, `tree::read` and `tree::write` carries the
element path the table holds (`Condition.onset[x]`), so a diagnostic points at
the definition. A shape the element table contradicts is refused rather than
read leniently, and a write applies to a copy so a refusal leaves no half-built
element behind.

## The program is immutable; the engine never parses a path

`resolve::compile` is the one place a path is parsed, an anchor is bound, an
extension is applied and a version selector is checked. What it returns is a
`Program` with private fields, no `&mut self` method and an `Arc` around it, so
nothing downstream can change what a context compiled to. The interpreter reads
a resolved FHIR target, a resolved Web Template node and a structured
occurrence axis; it never sees a path string and never reads a mapping file. A
mapping that cannot be resolved is a refusal at load, never a failure on the
request that first touches it.

Every diagnostic the compiler raises is collected. A run reports every
disagreement it found, because a mapping author fixing one refusal at a time is
how a load loop turns into an afternoon.

## A mapping with no `type` key converts through a derived pair

The `type` key is deprecated because the type is "derivable from the
instances" (`data-mappings.adoc` §Deprecated), so the compiler derives it once
and the program carries it as `Mapping::derived`; the engine never looks at a
type code or a class name to decide a cell. `resolve::derive::PAIRS` is the one
table: per openEHR class, the FHIR type codes a cell of the engine carries,
in the order the class's page of the data-type chapter lists them. That order
is our own design, and it is the whole tie-break: an element's own type wins
when the class pairs with it, a text primitive that is no pair converts as a
`string`, and a choice element with no type filter is written as the first
pair of the class the choice admits (going in, the alternative the document
carries decides). An `Extension` against a data value converts through its
`value[x]`. An untyped mapping onto a structural (`PATHABLE`) node anchors its
children, and one with no child is a warning, never a silent nothing. Every
pair in `PAIRS` must name a `FhirKind`; `engine::fhir` tests that.

## Direction enters the engine at three points only

A condition is evaluated on the input side, a `unidirectional` marker skips a
mapping in the other direction, and the composition defaults apply going into
openEHR. Everything else is written once, which is why a data-type cell is a
lens: `get` and `put` live beside each other and are property-tested against
each other both ways, so the two directions cannot drift apart. Two
independently written direction functions are the duplication this shape
exists to prevent.

A skipped element is a typed outcome carried to the caller, never a log line,
and the declared set of losses is closed: the round-trip tests assert it
exactly. An element the program cannot map refuses the unit.

The concept-type methods that create or resolve a document are the one place
each direction has a branch of its own, because the specification names a
different act per side: `hierarchy.split` runs `split.fhir` going out of
openEHR and `split.openehr` going in, and `reference` resolves a resource
going in and creates one going out. `link` and `participationsFunction` keep
the two branches beside each other over the same family, so they cannot
drift.

## What a run calls out to is a seam

The engine makes no call of its own. `engine::seam::Seams` carries the
`mappingCode` registry, the `ReferenceSource` a `reference` mapping fetches
from, and the `IdentitySink` that names every resource a run creates;
`Seams::default()` registers nothing, resolves nothing and derives ids by a
digest over the `IdentityRequest`, so the same composition yields the same
ids. A reference that resolves to nothing is a `Warning::Skipped`, never a
silent nothing; a reference chain that reaches itself refuses. Created
resources travel as `Outcome::created`: the operations lane answers them as
Bundle entries and the facade carries them as contained resources with `#id`
references. The operations lane resolves a reference against the request
Bundle only.

## Reference-model attributes beside the template nodes

`FEEDER_AUDIT`, `LINK` and `PARTICIPATION` are no Web Template node, so the
engine writes them as the `_feeder_audit`, `_link:i` and
`_other_participation:i` FLAT families of the node they belong to
(`engine::family`); `participationsFunction` writes
`ENTRY.other_participations` and `EVENT_CONTEXT.participations`
(`_participation:i` under `context`), the same `PARTICIPATION` list.
`ENTRY.provider` is a tail of the entry node whose value the cell writes as
the `_provider` family (`rm::Carried::Family`). A run whose `Defaults` carry
an `engine::origin::Origin` records the originating system, the source
resource and every defaulted field in the composition's
`FEEDER_AUDIT`, in the same order as the `Warning::Defaulted` entries. A
`link` target must be an `ehr:` URI, because `LINK.target` is a `DV_EHR_URI`;
the linked composition of a `link` whose FHIR side is no reference is
refused, since one run produces one composition.

## A tail below a node is carried by the FLAT parts of its class

The resolver records the RM class of the last attribute a tail names
(`OpenehrTarget::leaf_class`). The engine reads the tail from the node's
canonical JSON and writes it by merging into the value the place holds, and
`engine::rm::carried` is the table of tails the FLAT parts of each class
write. A tail outside it would be merged and then dropped on the way to the
wire, so the compiler refuses it at load (`fc-uncarried-tail`, naming the
mapping, the class and the tail), for a `with.openehr` tail and for a `manual`
path alike; `EngineError::UnsupportedTail` in both directions is only the
backstop for a program the compiler never saw. A tail that ends on a data
value runs the cell of the tail's class, not the node's. The FLAT spellings
are Simplified Formats, vendored at
`docs/specs/its-rest/docs/simplified_formats/`.

## Where a mapping writes is one rule, applied to whichever side is the output

The axes the parent mapping bound keep their instance; every repeating element
below them takes a fresh instance per input occurrence, because FHIRconnect
appends to a `0..n` path it does not iterate. An output with no repeating
element below the parent holds one value, so a later mapping overwrites an
earlier one and a `0..n` input leaves only its last occurrence. That one rule
is what makes the specification's three recurrence examples come out as the
specification draws them, in both directions, and the counter-examples come out
wrong in the documented way rather than in some other way. Going into openEHR
the repeating nodes down to the start archetype are bound to their first
instance before any mapping runs (`Program::root_axes`), because `$archetype`
is the root as `$resource` is: one resource, one archetype instance. A FHIR
axis is named by the document path the walk took, so two elements that share a
definition count their instances apart.

A `hierarchy.split` adds pins to that rule and nothing else: each group of
occurrences (one per occurrence, or one per distinct `unique` tuple) runs the
whole mapping set with the split node's openEHR axis pinned to the group's
instance, and going into openEHR with the FHIR `with` element pinned too. A
pinned axis counts as bound, so every mapping under the node writes into the
group's element and every mapping outside it runs as it would unsplit.

## Tests

The corpus under `docs/specs/fhirconnect-mapping-lib/` is evidence of what real
mappings write, never an oracle. A path form found there is covered by a test;
a form it uses that the FHIR specification does not define is adjudicated
against the specification and recorded, never accepted because the corpus
writes it. Fixtures are synthetic content invented for the test.
