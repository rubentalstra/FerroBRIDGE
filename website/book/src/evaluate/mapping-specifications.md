<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# The two mapping specifications

FerroBRIDGE reads two mapping languages. Both describe how openEHR content
relates to something else, both are YAML, and both are published by the same
author. Knowing where they agree tells you what the bridge can share; knowing
where they differ tells you why it keeps two interpreters.

<!-- toc -->

## What they share

One header. `grammar`, `type`, `metadata.name` and `metadata.version`, `spec`,
and `spec.openEhrConfig` pinning the archetype and its revision. The FHIRconnect
header page states that the header "is standardized for both FHIRconnect and
OMOCL".

Below the header they also share:

- RM and archetype paths, with `../` navigation.
- Archetype-keyed model files.
- An include mechanism: `slotArchetype` in FHIRconnect, `Include` in OMOCL.
- An escape hatch to code outside the mapping: `mappingCode` in FHIRconnect,
  `CustomMapping` in OMOCL.
- A code-translation concept: `conceptmap` in FHIRconnect, `conceptMap` in
  OMOCL.

Nothing else is common.

## FHIRconnect

FHIRconnect v1.0.0 is bidirectional. A mapping is a tree of entries, each
pairing a FHIR path with an openEHR path through `with`, typed from the
instances. `followedBy` nests entries and concatenates their paths onto the
parent's. Conditions are AND-combined and apply to the input side only.
`manual` supplies literals, `reference` points at another resource, `hierarchy`
realigns a tree, and `unidirectional` marks an entry that runs in one direction
only.

There are three file types, and they compose in a fixed order:

| File type | What it carries |
|---|---|
| `model` | an archetype mapped to an unprofiled resource, the shared layer |
| `extension` | profile and template adaptation, executed after the model |
| `context` | the entry point: the profile, the template, the archetypes, the extensions, and `start` |

Two JSON schemas are published, `model-mapping.schema.json` and
`contextual-mapping.schema.json`, both draft-07. They are the validation oracle,
and FerroBRIDGE validates every mapping against them before it runs anything.
The specification is at
<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/index.html>.

## OMOCL

OMOCL v1.0.0 is one-directional, openEHR to OMOP, by construction. A mapping is
a flat list of records, each keyed by a CDM target such as `Measurement`,
`ConditionOccurrence`, or `Person`. Inside a record, the keys are CDM column
names, and each takes an ordered list of `alternatives`: an RM `path`, or a
literal `code` holding an OMOP `concept_id`. The value `0` is the CDM's "no
matching concept". Records also carry `optional`, inline `conceptMap` tables,
`Include`, and `CustomMapping`. The published files use YAML anchors and
aliases, so the loader has to resolve them.

OMOCL publishes no JSON schema. Its grammar comes as a railroad diagram and
syntax tables, and its lexer and parser are marked work in progress.
FerroBRIDGE authors a JSON schema from the grammar tables and the published
mapping library, validates every OMOCL file against it, and offers that schema
upstream. The specification is at <https://github.com/SevKohler/OMOCL>.

## What this means for the bridge

The shared header, the path model, the include mechanism, and the YAML loader
live in one foundation crate. Everything above them is per language, because a
tree of bidirectional FHIR path pairs and a flat list of CDM column
alternatives have no useful common form. That split is the first decision in
the [architecture](architecture.md).
