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

## Tests

The corpus under `docs/specs/fhirconnect-mapping-lib/` is evidence of what real
mappings write, never an oracle. A path form found there is covered by a test;
a form it uses that the FHIR specification does not define is adjudicated
against the specification and recorded, never accepted because the corpus
writes it. Fixtures are synthetic content invented for the test.
