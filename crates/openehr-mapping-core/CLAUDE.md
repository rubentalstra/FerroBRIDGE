# openehr-mapping-core

The half of the mapping foundation both languages share, hand-written and
consumed by `fhirconnect` and `omocl` (`docs/architecture.md` §7, §8). Nothing
here interprets a mapping: the crate hands each language a header, a positioned
value tree, a registry lookup, a diagnostic and a resolved openEHR path, and
the language crate applies its own grammar to them.

- **The header is the shared one.** The FHIRconnect header page states it "is
  standardized for both FHIRconnect and OMOCL", so `grammar`, `type`,
  `metadata.name`, `metadata.version` and `spec.openEhrConfig.archetype` are
  modelled here once. Everything a language adds under `spec` (`fhirConfig`,
  `extends`, `system`, `version`, `revision`) stays an opaque positioned node;
  parsing it here would fork the language model.
- **Two strictness rules differ by language, each with its ground.** `type` is
  required for FHIRconnect, because both published schemas list it in
  `required`, and optional for OMOCL, which publishes no schema. The `grammar`
  language name is compared case-insensitively, because the specification's own
  prose and its schema pattern disagree about the spelling of FHIRconnect; the
  `type` value is compared case-exactly, because a JSON Schema `enum` fixes it.
- **The corpora are evidence, never oracles.** `tests/it/corpus.rs` pins the
  exact set of vendored files this crate refuses and why. A file that starts
  failing, or stops failing, is a finding to adjudicate against the
  specification, never a reason to widen the loader.
- **The parser is `serde-saphyr`.** Duplicate keys are errors, merge keys are
  expanded, and an unrecognised YAML tag is refused. An aliased node carries
  the position of the alias, which is where a reader of the file looks first.
- **Paths go through `openehr-rm`.** The BASE path grammar is never
  re-implemented here. This crate adds only what the mapping languages add to
  it: the leading `../` run and the `$name` head variable, both resolved
  against an anchor before a path is emitted.

- **FLAT keys go through `openehr-sdt`.** A `NodeValue` carries its
  segments and suffixes as `openehr_sdt::flat::path::{Segment, Suffix}` and
  its key is a `FlatKey`, so this crate prints no `/`, `:i` or `|` of its own.
  An index past `MAX_INSTANCE_INDEX` is `PathError::InstanceIndex` before the
  builder runs. A `ctx/` value is `NodeValue::context`; a whole value is the
  `raw` suffix (Simplified Formats master04 §Raw canonical JSON).

The Web Template index, relative path derivation, and composition build and
read (the other half of this crate's role in `docs/architecture.md` §7) are
#75.
