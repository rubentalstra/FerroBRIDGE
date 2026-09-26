// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The narrowing of the data type maps into one complex target: the
//! alternatives for one value and the halves of one value.

use std::collections::BTreeSet;

use crate::map::corpus::{Map, Row};
use crate::map::run::datum::Datum;
use crate::map::run::{Base, Mark, Pending, Recorded, Run, Scope};
use crate::map::{MapError, Outcome, RowRef};

impl<'a> Run<'a> {
    /// Narrows each set of halves of one value among `maps`
    /// ([`Map::half_of`]) to the one the row's place names, and returns the
    /// maps with the components the halves left out read.
    ///
    /// The guide maps an `EIP` by two rows from one source into one element
    /// with two labels (`SPM-2` into `identifier[1]` and `identifier[2]`), so
    /// the rows in map order take the halves in component order, each
    /// counted as [`Outcome::DatatypeHalf`]. When the row has no such
    /// siblings, or the value holds a component no half reads (an `EI`
    /// written where the row names an `EIP`), every half is kept and
    /// [`Run::datatypes`] refuses the row. No specification governs this:
    /// our own design.
    pub(super) fn halve(
        &mut self,
        scope: &Scope<'a>,
        row: &Row,
        reference: &RowRef,
        maps: Vec<&'a Map>,
        datum: Datum<'a>,
    ) -> (Vec<&'a Map>, BTreeSet<usize>) {
        let place = self.place(row, reference);
        let valued = datum.valued_parts();
        let mut kept = Vec::new();
        let mut also = BTreeSet::new();
        for mut set in group(maps, Map::half_of) {
            if set.len() < 2 {
                kept.extend(set);
                continue;
            }
            set.sort_by_key(|map| (map.components().first().copied(), map.id.clone()));
            let read: BTreeSet<usize> = set.iter().flat_map(|map| map.components()).collect();
            let chosen = place
                .filter(|(_, count)| *count == set.len())
                .filter(|_| valued.iter().all(|component| read.contains(component)))
                .and_then(|(index, _)| set.get(index).copied());
            let Some(chosen) = chosen else {
                kept.extend(set);
                continue;
            };
            for other in set.iter().filter(|other| other.id != chosen.id) {
                also.extend(other.components());
            }
            self.outcomes.push(Outcome::DatatypeHalf {
                at: scope.at.clone(),
                row: reference.clone(),
                map: chosen.id.clone(),
                halves: set.iter().map(|map| map.id.clone()).collect(),
            });
            kept.push(chosen);
        }
        kept.sort_by(|a, b| a.id.cmp(&b.id));
        (kept, also)
    }

    /// The place of `row` among the rows of its map from the same source
    /// whose targets differ from its own only in their labels, with their
    /// count.
    fn place(&self, row: &Row, reference: &RowRef) -> Option<(usize, usize)> {
        let map = self.corpus.get(&reference.map)?;
        let shape = unlabelled(&row.target_code);
        let siblings: Vec<&Row> = map
            .rows
            .iter()
            .filter(|other| {
                other.source == row.source
                    && other.assignment.is_none()
                    && other.mapped_via.is_none()
                    && unlabelled(&other.target_code) == shape
            })
            .collect();
        let index = siblings
            .iter()
            .position(|other| other.target_code == row.target_code)?;
        Some((index, siblings.len()))
    }

    /// Chooses one map from each set of alternatives among `maps`
    /// ([`Map::alternative_to`]) and keeps every other map, by id.
    ///
    /// The map of a set whose rows write from the most components of `datum`
    /// is chosen; among equally specific maps whose writes agree, the one
    /// with the fewest rows, the plain map. Equally specific maps whose
    /// writes differ are counted as [`Outcome::DatatypeAmbiguous`], and none
    /// of them runs. No specification governs this: our own design, since
    /// no row of the guide names the variant it runs.
    pub(super) fn select(
        &mut self,
        scope: &Scope<'a>,
        reference: &RowRef,
        maps: Vec<&'a Map>,
        datatype: &str,
        datum: Datum<'a>,
        leaf: &Base,
    ) -> Result<Vec<&'a Map>, MapError> {
        let mut chosen = Vec::new();
        for set in group(maps, Map::alternative_to) {
            match set.as_slice() {
                [one] => chosen.push(*one),
                several => {
                    if let Some(map) =
                        self.choose(scope, reference, several, datatype, datum, leaf)?
                    {
                        chosen.push(map);
                    }
                }
            }
        }
        chosen.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(chosen)
    }

    /// Chooses one of the alternative maps `set` for `datum`, as
    /// [`Run::select`] describes, running each on trial and rolling it back.
    fn choose(
        &mut self,
        scope: &Scope<'a>,
        reference: &RowRef,
        set: &[&'a Map],
        datatype: &str,
        datum: Datum<'a>,
        leaf: &Base,
    ) -> Result<Option<&'a Map>, MapError> {
        let mut trials = Vec::new();
        for map in set {
            let mark = self.mark();
            let covered =
                self.datatype_rows(scope, map, datatype, datum, leaf, &BTreeSet::new())?;
            let recorded = self.recorded_since(&mark);
            self.rollback(&mark);
            trials.push((*map, covered.wrote.len(), recorded));
        }
        let most = trials.iter().map(|(_, count, _)| *count).max().unwrap_or(0);
        trials.retain(|(_, count, _)| *count == most);
        let agree = trials
            .split_first()
            .is_some_and(|((_, _, first), rest)| rest.iter().all(|(_, _, other)| other == first));
        if agree {
            return Ok(trials.iter().map(|(map, _, _)| *map).min_by(|a, b| {
                a.rows
                    .len()
                    .cmp(&b.rows.len())
                    .then_with(|| a.id.cmp(&b.id))
            }));
        }
        self.outcomes.push(Outcome::DatatypeAmbiguous {
            at: scope.at.clone(),
            row: reference.clone(),
            candidates: trials.iter().map(|(map, _, _)| map.id.clone()).collect(),
        });
        Ok(None)
    }

    /// The writes recorded since `mark`, in every resource, for comparison.
    fn recorded_since(&self, mark: &Mark) -> Vec<Recorded> {
        let mut recorded = Vec::new();
        for (index, resource) in self.resources.iter().enumerate() {
            let start = mark.writes.get(index).copied().unwrap_or(0);
            for write in resource.writes.get(start..).unwrap_or_default() {
                let (value, translation) = match &write.value {
                    Pending::Ready(value) => (Some(value.clone()), None),
                    Pending::Translation { request, part } => (
                        None,
                        self.requests
                            .get(*request)
                            .map(|asked| (asked.url.clone(), asked.code.clone(), *part)),
                    ),
                    Pending::Sibling { .. } => (None, None),
                };
                recorded.push(Recorded {
                    resource: index,
                    type_name: resource.type_name.clone(),
                    slots: write.slots.clone(),
                    value,
                    translation,
                });
            }
        }
        recorded
    }

    /// The data type maps from `source_type` into the complex element `leaf`:
    /// those named for its type, else those named for its element path.
    ///
    /// The guide's map titles name a target by type or by element path
    /// (`mapping_guidelines.md`, the `CQ[ServiceRequest.duration]` title
    /// form), and a backbone element such as `MessageHeader.source` has no
    /// type name, so its maps carry the path, as
    /// `datatype-hd-endpoint-to-messageheader-source`.
    pub(super) fn complex_maps(
        &self,
        source_type: &str,
        target_type: &str,
        leaf: &Base,
    ) -> Vec<&'a Map> {
        let corpus = self.corpus;
        let named = corpus.qualifying("datatype", source_type, target_type);
        if !named.is_empty() {
            return named;
        }
        let Some(resource) = self.resources.get(leaf.resource) else {
            return Vec::new();
        };
        let mut path = resource.type_name.clone();
        for slot in &leaf.slots {
            path.push('-');
            path.push_str(&slot.name);
        }
        corpus.qualifying("datatype", source_type, &path)
    }
}

/// Groups `maps` into sets joined by `related`, each set by id.
pub(super) fn group<'m>(maps: Vec<&'m Map>, related: fn(&Map, &Map) -> bool) -> Vec<Vec<&'m Map>> {
    let mut sets: Vec<Vec<&'m Map>> = Vec::new();
    for map in maps {
        let (joined, mut apart): (Vec<_>, Vec<_>) = std::mem::take(&mut sets)
            .into_iter()
            .partition(|set| set.iter().any(|other| related(other, map)));
        let mut merged: Vec<&'m Map> = joined.into_iter().flatten().collect();
        merged.push(map);
        merged.sort_by(|a, b| a.id.cmp(&b.id));
        apart.push(merged);
        sets = apart;
    }
    sets
}

/// A target code with every `[..]` label removed, `identifier[2]` giving
/// `identifier`.
fn unlabelled(code: &str) -> String {
    let mut text = String::with_capacity(code.len());
    let mut depth = 0_usize;
    for character in code.chars() {
        match character {
            '[' => depth = depth.saturating_add(1),
            ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => text.push(character),
            _ => {}
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use fhir_types::codec::Value;

    use crate::map::Outcome;
    use crate::map::run::Run;
    use crate::map::run::tests::{
        GLUCOSE, corpus, datatype_map, ei_refusals, message, specimen, supplemented, text,
        values_at, writes_of,
    };
    use crate::parse::Parsed;

    /// A synthetic ORU^R01 whose one order carries `orc`.
    fn order(orc: &str) -> Parsed {
        message(&[
            "MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00021|P|2.5.1",
            "PID|1||PAT-0021^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
            orc,
            "OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN",
        ])
    }

    // NOTE: `ConceptMap-datatype-ei-*-to-identifier`: the four variants each write `EI.1` into
    // `value`, and `hd-endpoint` writes `name` from HD.1 only under a condition.
    #[test]
    fn the_ei_variants_are_alternatives_and_the_hd_maps_into_one_source_are_not() {
        let corpus = corpus();
        let map = |id: &str| corpus.get(id).expect("a vendored map");
        let organization = map("datatype-ei-organization-to-identifier");
        for other in [
            "datatype-ei-defaultassigner-to-identifier",
            "datatype-ei-extension-to-identifier",
            "datatype-ei-system-to-identifier",
        ] {
            assert!(organization.alternative_to(map(other)), "{other}");
        }
        assert!(
            !map("datatype-hd-endpoint-to-messageheader-source")
                .alternative_to(map("datatype-hd-name-to-messageheader-source"))
        );
    }

    // NOTE: `segment-orc-to-diagnosticreport` ORC-2 and ORC-3 into `identifier[1]` and `[2]`; a
    // bare EI takes the plain variant, `datatype-ei-defaultassigner-to-identifier`, once.
    #[test]
    fn a_bare_placer_and_filler_number_give_one_identifier_each() {
        let corpus = corpus();
        let parsed = order("ORC|RE|PLC-1|FIL-1");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DiagnosticReport", "identifier.value"),
            vec![text("PLC-1"), text("FIL-1")],
            "{:?}",
            writes_of(&run, "DiagnosticReport")
        );
        assert!(
            writes_of(&run, "DiagnosticReport")
                .iter()
                .all(|(path, _)| path != "identifier.system" && path != "identifier.assigner"),
            "{:?}",
            writes_of(&run, "DiagnosticReport")
        );
        assert_eq!(ei_refusals(&run), Vec::<&Outcome>::new());
    }

    // NOTE: `datatype-ei-organization-to-identifier` maps EI.2 into the assigner, the one variant
    // with a row that runs on a namespace ID, so it is the most specific.
    #[test]
    fn a_value_with_an_assigning_authority_takes_the_organization_variant() {
        let corpus = corpus();
        let parsed = order("ORC|RE|PLC-1^ORDERDESK|FIL-1");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DiagnosticReport", "identifier.value"),
            vec![text("PLC-1"), text("FIL-1")]
        );
        assert!(
            values_at(&run, "Organization", "identifier.value").contains(&text("ORDERDESK")),
            "{:?}",
            writes_of(&run, "Organization")
        );
        assert_eq!(ei_refusals(&run), Vec::<&Outcome>::new());
    }

    // NOTE: `segment-txa-to-documentreference` TXA-12 into `masterIdentifier`.
    #[test]
    fn a_unique_document_number_gives_one_master_identifier() {
        let corpus = corpus();
        let parsed = message(&[
            "MSH|^~\\&|DOCS|NORTHHOSP|EHR|SOUTHCLINIC|20260925100000+0200||MDM^T02^MDM_T02|MSG00023|P|2.5.1",
            "EVN||20260925095900+0200",
            "PID|1||PAT-0023^^^NORTHHOSP^MR||Doe^Sam^^^^^L||19751111|U",
            "PV1|1|O",
            "TXA|1|CN|TX|20260925095000+0200||||||||DOC-0023|||||AU",
        ]);
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DocumentReference", "masterIdentifier.value"),
            vec![text("DOC-0023")],
            "{:?}",
            writes_of(&run, "DocumentReference")
        );
        assert_eq!(ei_refusals(&run), Vec::<&Outcome>::new());
    }

    // NOTE: no specification governs this: our own design; two variants equally specific for a
    // value that write it differently are counted with both named, and neither writes.
    #[test]
    fn two_equally_specific_variants_that_differ_are_a_counted_ambiguity() {
        let rival = datatype_map(
            "datatype-ei-rival-to-identifier",
            "http://example.org/fhir/ConceptMap/datatype-ei-rival-to-identifier",
            &[("EI.1", "value", "string"), ("EI.2", "type.text", "string")],
        );
        let corpus = supplemented(&[rival]);
        let parsed = order("ORC|RE|PLC-1^ORDERDESK|FIL-1");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DiagnosticReport", "identifier.value"),
            vec![text("FIL-1")]
        );
        let ambiguous: Vec<(&str, &[String])> = run
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::DatatypeAmbiguous {
                    row, candidates, ..
                } => Some((row.source.as_str(), candidates.as_slice())),
                _ => None,
            })
            .collect();
        assert_eq!(
            ambiguous,
            vec![(
                "ORC-2",
                &[
                    String::from("datatype-ei-organization-to-identifier"),
                    String::from("datatype-ei-rival-to-identifier"),
                ][..]
            )]
        );
    }

    // NOTE: `StructureDefinition-TypeInfo.json` `mappedVia` is the "Url of the mapping artifact
    // for the item", so a row naming a data type map runs that map alone.
    #[test]
    fn a_row_naming_its_data_type_map_runs_that_map() {
        let segment: Value = serde_json::from_value(serde_json::json!({
            "resourceType": "ConceptMap",
            "id": "segment-orc-to-diagnosticreport",
            "url": "http://example.org/fhir/ConceptMap/segment-orc-to-diagnosticreport",
            "status": "active",
            "group": [{ "element": [{
                "code": "ORC-2",
                "extension": [{
                    "url": crate::map::corpus::TYPE_INFO,
                    "extension": [{ "url": "type", "valueCode": "EI" }]
                }],
                "target": [{
                    "code": "identifier[1]",
                    "equivalence": "equivalent",
                    "extension": [{
                        "url": crate::map::corpus::TYPE_INFO,
                        "extension": [
                            { "url": "type", "valueCode": "Identifier" },
                            {
                                "url": "mappedVia",
                                "valueUrl": "ConceptMap/datatype-ei-extension-to-identifier"
                            }
                        ]
                    }]
                }]
            }]}]
        }))
        .expect("a JSON value");
        let corpus = supplemented(&[segment]);
        let parsed = order("ORC|RE|PLC-1^ORDERDESK");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DiagnosticReport", "identifier.value"),
            vec![text("PLC-1")]
        );
        assert!(
            values_at(&run, "Organization", "identifier.value")
                .iter()
                .all(|value| value != &text("ORDERDESK")),
            "{:?}",
            writes_of(&run, "Organization")
        );
        assert!(
            run.outcomes.iter().any(|outcome| matches!(
                outcome,
                Outcome::UnmappedComponent { map, component: 2, .. }
                    if map == "datatype-ei-extension-to-identifier"
            )),
            "{:?}",
            run.outcomes
        );
    }

    /// The maps a run chose as halves, with the row source and target.
    fn halves(run: &Run<'_>) -> Vec<(String, String, String)> {
        run.outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::DatatypeHalf { row, map, .. } => {
                    Some((row.source.clone(), row.target.clone(), map.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// The conflicts and ambiguities a run counted.
    fn refusals<'r>(run: &'r Run<'_>) -> Vec<&'r Outcome> {
        run.outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    Outcome::DatatypeConflict { .. } | Outcome::DatatypeAmbiguous { .. }
                )
            })
            .collect()
    }

    // NOTE: `ConceptMap-datatype-eip-*-to-identifier`: the placer map writes `EIP.1` into `value`
    // and the filler map `EIP.2`, so the two fill one `Identifier` each and are no alternatives.
    #[test]
    fn the_eip_maps_are_halves_and_the_ei_variants_are_not() {
        let corpus = corpus();
        let map = |id: &str| corpus.get(id).expect("a vendored map");
        let placer = map("datatype-eip-placerassignedidentifier-to-identifier");
        let filler = map("datatype-eip-fillerassignedidentifier-to-identifier");
        assert!(placer.half_of(filler));
        assert!(!placer.alternative_to(filler));
        assert!(
            !map("datatype-ei-organization-to-identifier")
                .half_of(map("datatype-ei-system-to-identifier"))
        );
        assert!(
            !map("datatype-hd-endpoint-to-messageheader-source")
                .half_of(map("datatype-hd-name-to-messageheader-source"))
        );
    }

    // NOTE: `segment-spm-to-specimen` maps SPM-2 into `identifier[1]` and `identifier[2]`, and the
    // EIP components are the placer's (EIP.1) then the filler's (EIP.2), in that order.
    #[test]
    fn a_specimen_id_gives_the_placer_and_the_filler_identifier() {
        let corpus = corpus();
        let parsed = specimen("ORC|RE|PLC-1|FIL-1", GLUCOSE, "SPM|1|SPC-P^SPC-F");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "Specimen", "identifier.value"),
            vec![text("SPC-P"), text("SPC-F")],
            "{:?}",
            writes_of(&run, "Specimen")
        );
        assert_eq!(
            values_at(&run, "Specimen", "identifier.type.coding.code"),
            vec![text("PGN"), text("FGN")]
        );
        assert_eq!(
            halves(&run),
            vec![
                (
                    String::from("SPM-2"),
                    String::from("identifier[1]"),
                    String::from("datatype-eip-placerassignedidentifier-to-identifier")
                ),
                (
                    String::from("SPM-2"),
                    String::from("identifier[2]"),
                    String::from("datatype-eip-fillerassignedidentifier-to-identifier")
                ),
            ]
        );
        assert_eq!(refusals(&run), Vec::<&Outcome>::new());
    }

    #[test]
    fn a_filler_specimen_id_alone_gives_the_filler_identifier() {
        let corpus = corpus();
        let parsed = specimen("ORC|RE|PLC-1|FIL-1", GLUCOSE, "SPM|1|^SPC-F");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "Specimen", "identifier.value"),
            vec![text("SPC-F")],
            "{:?}",
            writes_of(&run, "Specimen")
        );
        assert_eq!(
            values_at(&run, "Specimen", "identifier.type.coding.code"),
            vec![text("FGN")]
        );
        assert!(
            run.outcomes.iter().all(
                |outcome| !matches!(outcome, Outcome::UnmappedComponent { map, .. }
                    if map.starts_with("datatype-eip-"))
            ),
            "{:?}",
            run.outcomes
        );
        assert_eq!(refusals(&run), Vec::<&Outcome>::new());
    }

    // NOTE: `segment-orc-to-diagnosticreport` maps ORC-4 into `identifier[3]` and `identifier[4]`,
    // so a placer group number alone gives the `PGN` identifier and no `FGN` one.
    #[test]
    fn a_placer_group_number_gives_the_placer_identifier() {
        let corpus = corpus();
        let parsed = order("ORC|RE|PLC-1|FIL-1|GRP-P");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DiagnosticReport", "identifier.value"),
            vec![text("PLC-1"), text("FIL-1"), text("GRP-P")],
            "{:?}",
            writes_of(&run, "DiagnosticReport")
        );
        let codes = values_at(&run, "DiagnosticReport", "identifier.type.coding.code");
        assert!(codes.contains(&text("PGN")), "{codes:?}");
        assert!(!codes.contains(&text("FGN")), "{codes:?}");
        assert_eq!(refusals(&run), Vec::<&Outcome>::new());
    }

    // NOTE: no specification governs this: our own design; an ORC-4 valuing EI components 3
    // and 4 is no EIP either half reads, so the row stays a counted conflict.
    #[test]
    fn an_entity_identifier_where_the_row_names_an_eip_stays_a_conflict() {
        let corpus = corpus();
        let parsed = order("ORC|RE|PLC-1|FIL-1|GRP-1^LAB^1.2.3^ISO");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert!(
            !values_at(&run, "DiagnosticReport", "identifier.value").contains(&text("GRP-1")),
            "{:?}",
            writes_of(&run, "DiagnosticReport")
        );
        assert_eq!(halves(&run), Vec::new());
        assert!(
            refusals(&run).iter().all(|outcome| matches!(
                outcome,
                Outcome::DatatypeConflict { row, .. } if row.source == "ORC-4"
            )) && !refusals(&run).is_empty(),
            "{:?}",
            run.outcomes
        );
    }
}
