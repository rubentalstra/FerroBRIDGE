<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Writing and loading mappings

Mappings are the product. FerroBRIDGE reads specification-conformant YAML and
never carries a hand-coded converter for one FHIR resource or one OMOP table. A
per-resource converter is refused in review, because it defeats the reason the
project exists. The loader described here is designed and not yet built, so
read this page as the contract it is being built to.

<!-- toc -->

## What a mapping set looks like

On the FHIR side you supply three kinds of file, and they compose in this
order:

1. A `model` file per archetype, mapping it to an unprofiled R4 resource.
2. An `extension` file where a profile or a template needs an adaptation.
3. A `context` file as the entry point, naming the profile, the template, the
   archetypes, the extensions, and the `start` mapping.

On the OMOP side you supply OMOCL files: a flat list of records keyed by CDM
target, each record's keys drawn from OMOCL's own column vocabulary (`value`,
`unit`, `concept_id`, `measurement_date`) with ordered `alternatives`.
FerroBRIDGE carries the table that projects each key onto CDM columns, because
OMOCL does not publish one. YAML anchors and aliases are supported, because the
published library uses them.

## Validation happens at load, not at runtime

Every FHIRconnect file is checked three ways. The two published draft-07 JSON
schemas are vendored and run, and the files they reject are recorded, because
those schemas refuse three mapping types the specification defines. Then
FerroBRIDGE's own strict schema runs, which admits exactly what the prose
defines. Then the constraints no schema can express are checked: `targetRoot`
alignment, `appendTo` and `slotArchetype` resolution, unique mapping names,
the presence of `criteria` where the grammar requires it, and every path
resolving against the template and the FHIR element table. Every OMOCL file is
validated against the schema FerroBRIDGE authors from the grammar tables.

Version agreement is part of loading. The mapping version, the grammar version,
the archetype revision, the template `sem_ver`, and the profile version have to
agree, and a mismatch is refused before any request is served.

## Paths need a template

Paths in both languages are RM and archetype paths, such as
`$archetype/data[at0001]/items[at0077]`. A path alone is not executable,
because the leaf RM type and the template-specific identifiers live in the Web
Template built from the template's operational template. FerroBRIDGE resolves
in that order: the operational template, then the Web Template, then the path,
then the composition. Supply the template to the CDR, and FerroBRIDGE reads it
from there.

## Composition fields FHIR does not carry

A composition needs a composer, a `context/start_time`, a `setting`, a
`language`, a `territory`, and a `category`. FHIR carries no counterpart for
most of those, so FHIRconnect defaults them and requires the `start` mapping to
slot in a reusable `COMPOSITION.<archetype>.<Resource>` mapping. FerroBRIDGE
follows those defaults and records every defaulted value in the composition's
`FEEDER_AUDIT`, so a later reader can tell which values came from the source
and which came from a default.

## Code translation

FHIRconnect's `conceptmap` and OMOCL's `conceptMap` both translate codes, and
they resolve differently. A FHIRconnect `conceptmap` reference goes to the
configured FHIR terminology server through `$translate`. An OMOCL `conceptMap`
is an inline table in the mapping file, and everything else on the OMOP side
resolves through the loaded OHDSI vocabulary in SQL.

## Escaping to code

`mappingCode` in FHIRconnect and `CustomMapping` in OMOCL name a function that
the mapping cannot express. FerroBRIDGE resolves those names against Rust
functions registered at build time. There is no runtime plugin loading, so a
mapping file cannot introduce new executable behaviour into a deployment.

## Where the mapping libraries come from

No mapping library is vendored yet. The plan on record is to vendor the
published FHIRconnect and OMOCL libraries verbatim through committed fetch
scripts, with provenance recorded beside them, and to exercise them in full:
every file loads and validates, or carries a recorded skip with its reason.
That corpus is how FerroBRIDGE finds out that a grammar feature it has not
implemented exists, which is why it lands with the first engine work rather
than after it.
