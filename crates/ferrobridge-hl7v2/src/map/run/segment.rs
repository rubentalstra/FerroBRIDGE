// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The row loop of a segment map: the conditions, the rows that map an
//! absence, and the data type each field row maps its value by.

use std::collections::BTreeSet;

use crate::map::condition::{self, Operand, Unevaluable};
use crate::map::corpus::{Condition, Corpus, Map, Row};
use crate::map::run::datum::Datum;
use crate::map::run::{Base, Run, Scope};
use crate::map::{MapError, Outcome, RowRef};
use crate::parse::{Location, Segment};

impl<'a> Run<'a> {
    /// Evaluates a row's condition, recording why a row does not apply.
    ///
    /// # Errors
    ///
    /// Returns [`MapError::Stopped`] when a `NOT VALUED ERROR` check holds.
    pub(super) fn gate(
        &mut self,
        scope: &Scope<'_>,
        row: &Row,
        reference: &RowRef,
    ) -> Result<bool, MapError> {
        match &row.condition {
            Condition::Always => Ok(true),
            Condition::Narrative => {
                self.outcomes.push(Outcome::NarrativeCondition {
                    at: scope.at.clone(),
                    row: reference.clone(),
                });
                Ok(false)
            }
            Condition::Unsupported { text, .. } => {
                self.outcomes.push(Outcome::UnsupportedCondition {
                    at: scope.at.clone(),
                    row: reference.clone(),
                    text: text.clone(),
                });
                Ok(false)
            }
            Condition::Defective { text } => {
                self.outcomes.push(Outcome::DefectiveCondition {
                    at: scope.at.clone(),
                    row: reference.clone(),
                    text: text.clone(),
                });
                Ok(false)
            }
            Condition::Computable(expr) => {
                match condition::evaluate(expr, &mut |operand| scope.probe(operand)) {
                    Ok(holds) => Ok(holds),
                    Err(Unevaluable::Operand(operand)) => {
                        self.outcomes.push(Outcome::UnevaluableCondition {
                            at: scope.at.clone(),
                            row: reference.clone(),
                            operand,
                        });
                        Ok(false)
                    }
                    Err(Unevaluable::Stop(operand)) => Err(MapError::Stopped {
                        at: Box::new(scope.at.clone()),
                        row: Box::new(reference.clone()),
                        operand,
                    }),
                }
            }
        }
    }

    /// Runs a segment map over the scope's segment into `base`.
    pub(super) fn segment(
        &mut self,
        scope: &Scope<'a>,
        map: &'a Map,
        base: &Base,
        named: &mut BTreeSet<usize>,
    ) -> Result<(), MapError> {
        let Some(segment) = scope.segment() else {
            return Ok(());
        };
        self.supplemented(map, &scope.at);
        for row in &map.rows {
            let reference = row_ref(map, row);
            let (id, path) = split_source(&row.source, '-');
            if id != segment.id() {
                self.outcomes.push(Outcome::UnsupportedShape {
                    at: scope.at.clone(),
                    row: reference,
                });
                continue;
            }
            let Some((&position, components)) = path.split_first() else {
                if self.gate(scope, row, &reference)? {
                    self.apply(scope, row, &reference, Datum::Segment, 0, base)?;
                }
                continue;
            };
            named.insert(position);
            let own = Operand::Field {
                segment: String::from(id),
                path: path.clone(),
            };
            let Some(field) = segment.field(position).filter(|field| field.is_valued()) else {
                let mut empty = scope.clone();
                empty.at = scope.at.clone().with_field(position);
                self.absence(&empty, row, &reference, &own, base)?;
                continue;
            };
            // NOTE: no specification governs this: our own design; a computable condition
            // is evaluated per repetition, reading the repetition mapped, and any other
            // gate is met once per row, so a narrative row counts once.
            let computable = matches!(row.condition, Condition::Computable(_));
            let mut once = scope.clone();
            once.at = scope.at.clone().with_field(position);
            if !computable && !self.gate(&once, row, &reference)? {
                continue;
            }
            let legacy = self.parsed.structure().withdrawn_as_of.is_some() && components.is_empty();
            if (legacy || row.source_type.is_none())
                && let Some((code, base)) =
                    version_specific(self.corpus, scope.definition, position)
            {
                self.outcomes.push(Outcome::BaseTyped {
                    at: once.at.clone(),
                    row: reference.clone(),
                    field: format!("{id}-{position}"),
                    code: code.code,
                    base: base.code,
                    version: code.version,
                });
            }
            let (source_type, replaced) = field_type(
                row.source_type.clone(),
                definition_type(self.corpus, scope.definition, position),
                legacy,
            );
            if let (Some(row_type), Some(version_type)) = (replaced, source_type.clone()) {
                self.outcomes.push(Outcome::VersionTyped {
                    at: once.at.clone(),
                    row: reference.clone(),
                    field: format!("{id}-{position}"),
                    row_type,
                    version_type,
                    version: self.parsed.structure().version,
                });
            }
            let typed = Row {
                source_type: resolve_varies(source_type, segment),
                ..row.clone()
            };
            for (repetition, value) in field.repetitions().iter().enumerate() {
                let datum = Datum::Repetition(value).at(components);
                let mut inner = scope.clone();
                inner.at = Location {
                    repetition: Some(repetition.saturating_add(1)),
                    ..once.at.clone()
                };
                inner.repetition = Some((position, repetition));
                if !datum.valued() {
                    self.absence(&inner, row, &reference, &own, base)?;
                    continue;
                }
                if computable && !self.gate(&inner, row, &reference)? {
                    continue;
                }
                self.apply(&inner, &typed, &reference, datum, repetition, base)?;
            }
        }
        Ok(())
    }

    /// Applies a row to its empty source when the row maps that absence: it
    /// assigns a value, and its condition requires `own`, its source, not
    /// valued ([`condition::requires_absent`]) and holds.
    ///
    /// Any other row on an empty source maps nothing and is skipped
    /// (`mapping_guidelines.md` §General Format/Approach: a condition decides
    /// whether the v2 element is mapped).
    ///
    /// # Errors
    ///
    /// Returns [`MapError::Stopped`] when a `NOT VALUED ERROR` check holds.
    pub(super) fn absence(
        &mut self,
        scope: &Scope<'a>,
        row: &Row,
        reference: &RowRef,
        own: &Operand,
        base: &Base,
    ) -> Result<(), MapError> {
        let maps_absence = row.assignment.is_some()
            && matches!(&row.condition, Condition::Computable(expr)
                if condition::requires_absent(expr, own));
        if maps_absence && self.gate(scope, row, reference)? {
            self.apply(scope, row, reference, Datum::Absent, 0, base)?;
        }
        Ok(())
    }

    /// Counts the valued fields of the scope's segment no applied row names.
    pub(super) fn unmapped_fields(&mut self, scope: &Scope<'a>, named: &BTreeSet<usize>) {
        let Some(segment) = scope.segment() else {
            return;
        };
        for (offset, field) in segment.fields().iter().enumerate() {
            let position = offset.saturating_add(1);
            let header = segment.id() == "MSH" && position <= 2;
            if field.is_valued() && !header && !named.contains(&position) {
                self.outcomes.push(Outcome::UnmappedField {
                    at: scope.at.clone().with_field(position),
                });
            }
        }
    }
}

/// Splits a source code into its id and positions: `PID-11.9` into `PID` and
/// `[11, 9]`, `CX.1` into `CX` and `[1]`, `MSG` into `MSG` and `[]`.
pub(super) fn split_source(source: &str, separator: char) -> (&str, Vec<usize>) {
    match source.split_once(separator) {
        Some((id, rest)) => (
            id,
            rest.split('.')
                .map_while(|part| part.parse().ok())
                .collect(),
        ),
        None => (source, Vec::new()),
    }
}

/// The data type the placed segment's definition gives a field, when its
/// `TypeInfo` names none: the definition of the version the parser selected
/// the structure from, so a legacy field keeps its own version's type.
///
/// The guide's maps come first: a legacy code the guide has a data type map
/// for keeps that map (`TS` keeps `datatype-ts-to-datetime`), and one it has
/// none for gives the base type the generated tables link it to
/// ([`version_specific`]): `CM_MSG` gives `MSG` and `CE_0051` gives `CE`.
fn definition_type(
    corpus: &Corpus,
    definition: &'static hl7v2_types::model::Segment,
    position: usize,
) -> Option<String> {
    if let Some((_, base)) = version_specific(corpus, definition, position) {
        return Some(String::from(base.code));
    }
    definition
        .fields
        .iter()
        .find(|field| usize::from(field.position) == position)?
        .data_type
        .map(|data_type| String::from(data_type.code()))
}

/// The legacy data type code of a field that stands for a base type, with
/// that base, as the generated tables link them, when the guide has no data
/// type map from the code as written.
fn version_specific(
    corpus: &Corpus,
    definition: &'static hl7v2_types::model::Segment,
    position: usize,
) -> Option<(
    &'static hl7v2_types::model::LegacyDataType,
    hl7v2_types::model::LegacyBase,
)> {
    let field = definition
        .fields
        .iter()
        .find(|field| usize::from(field.position) == position)?;
    match field.data_type {
        Some(hl7v2_types::model::DataTypeRef::Legacy(code))
            if !corpus.maps_from("datatype", code.code) =>
        {
            Some((code, code.base?))
        }
        _ => None,
    }
}

/// The data type a field row maps its value by, with the type the row names
/// when the field's own definition replaces it.
///
/// The row's `TypeInfo` type comes first. For a field of a legacy structure
/// (`legacy`), the placed definition's type decides where it differs from
/// the row's, compared without case, since the guide's rows name the types
/// of the v2.9.1 definitions. No specification governs this: our own design,
/// `mapping_guidelines.md` names no v2 version for its type columns.
fn field_type(
    named: Option<String>,
    own: Option<String>,
    legacy: bool,
) -> (Option<String>, Option<String>) {
    match (named, own) {
        (Some(named), Some(own)) if legacy && !named.eq_ignore_ascii_case(&own) => {
            (Some(own), Some(named))
        }
        (named, own) => (named.or(own), None),
    }
}

/// The type a `varies` field holds: OBX-5 is of the type OBX-2 names.
///
/// NOTE: HL7 v2.5.1 chapter 7 §7.4.2.2: OBX-2 "contains the format of the
/// observation value in OBX", the one `varies` field the mapped segments carry.
fn resolve_varies(source_type: Option<String>, segment: &Segment) -> Option<String> {
    match source_type.as_deref() {
        Some("varies" | "Varies") if segment.id() == "OBX" => segment.text(2).map(String::from),
        _ => source_type,
    }
}

/// The reference of a row, for an outcome.
pub(super) fn row_ref(map: &Map, row: &Row) -> RowRef {
    RowRef {
        map: map.id.clone(),
        source: row.source.clone(),
        target: row.target_code.clone(),
    }
}

#[cfg(test)]
mod tests {
    use fhir_types::codec::Value;

    use super::{definition_type, field_type};
    use crate::decode::Charset;
    use crate::map::Outcome;
    use crate::map::run::Run;
    use crate::map::run::tests::{corpus, header_writes, legacy_order, parsed};
    use crate::map::run::walk::collect;
    use crate::parse;

    // NOTE: `segment-msh-to-messageheader`: the MSH-24 rows gated on
    // `IF MSH-24 NOT VALUED AND MSH-3 NOT VALUED` assign the data-absent-reason.
    #[test]
    fn rows_gated_on_their_own_empty_source_write_their_assignment() {
        let corpus = corpus();
        let parsed = parsed("||EHR|SOUTHCLINIC");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let writes = header_writes(&run);
        let reason = String::from("http://hl7.org/fhir/R4/extension-data-absent-reason.html");
        assert!(
            writes.contains(&(
                String::from("source.endpoint.extension.url"),
                Value::String(reason)
            )),
            "{writes:?}"
        );
        assert!(
            writes.contains(&(
                String::from("source.endpoint.extension.valueCode"),
                Value::String(String::from("unknown"))
            )),
            "{writes:?}"
        );
    }

    #[test]
    fn a_valued_source_takes_no_absence_row() {
        let corpus = corpus();
        let parsed = parsed("LAB|NORTHLAB|EHR|SOUTHCLINIC");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let writes = header_writes(&run);
        assert!(
            writes.iter().all(|(path, _)| !path.contains("extension")),
            "{writes:?}"
        );
    }

    // NOTE: the HL7 v2.3 tables type OBR-4 `CE` where the guide's row names `CWE`, so the
    // placed 2.3 definition chooses the guide's `CE` map, counted as `version-typed`.
    #[test]
    fn a_legacy_field_is_typed_by_its_own_versions_definition() {
        let text = "MSH|^~\\&|NORTHLAB|NORTHHOSP|EHR|SOUTHCLINIC|20260925143000+0200||ORM^O01|MSG00012|P|2.3\r\
             PID|1||PAT-0012^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M\r\
             ORC|NW|PLC-2\r\
             OBR|1|PLC-2||2345-7^Glucose^LN\r";
        let lexed = parse::lex::lex(text, Charset::Ascii).expect("it lexes");
        let structure = parse::structure::structure_for(&lexed.message).expect("a structure");
        assert_eq!((structure.id, structure.version), ("ORM_O01", "2.3"));
        let parsed = parse::grouping::group(lexed, structure);
        let mut visits = Vec::new();
        collect(
            parsed.items(),
            parsed.structure_name(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            parsed.structure().nodes,
            &mut visits,
        );
        let obr = visits
            .iter()
            .find(|visit| visit.code == "ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR")
            .expect("the OBR is placed");
        let legacy = definition_type(&corpus(), obr.definition, 4).expect("OBR-4 is typed at 2.3");
        assert_eq!(legacy, "CE");
        assert_eq!(
            definition_type(&corpus(), &hl7v2_types::segment::obr::OBR, 4).as_deref(),
            Some("CWE")
        );
        let corpus = corpus();
        let chosen = corpus
            .find("datatype", &legacy, "CodeableConcept")
            .expect("one map");
        assert_eq!(chosen.id, "datatype-ce-to-codeableconcept");
        assert_eq!(
            field_type(Some(String::from("CWE")), Some(legacy.clone()), true),
            (Some(legacy), Some(String::from("CWE")))
        );
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let typed: Vec<(&str, &str, &str, &str)> = run
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::VersionTyped {
                    field,
                    row_type,
                    version_type,
                    version,
                    ..
                } => Some((
                    field.as_str(),
                    row_type.as_str(),
                    version_type.as_str(),
                    *version,
                )),
                _ => None,
            })
            .collect();
        assert!(typed.contains(&("PID-8", "CWE", "IS", "2.3")), "{typed:?}");
    }

    #[test]
    fn a_version_specific_field_type_resolves_to_its_base_for_the_map_lookup() {
        let msh = &hl7v2_types::legacy::v2_3::segment::msh::MSH;
        let dg1 = &hl7v2_types::legacy::v2_3::segment::dg1::DG1;
        let corpus = corpus();
        assert_eq!(definition_type(&corpus, msh, 9).as_deref(), Some("MSG"));
        assert_eq!(definition_type(&corpus, dg1, 3).as_deref(), Some("CE"));
        assert_eq!(definition_type(&corpus, msh, 4).as_deref(), Some("HD"));
        let parsed = legacy_order();
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let based: Vec<(&str, &str, &str, &str)> = run
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::BaseTyped {
                    field,
                    code,
                    base,
                    version,
                    ..
                } => Some((field.as_str(), *code, *base, *version)),
                _ => None,
            })
            .collect();
        for expected in [
            ("MSH-9", "CM_MSG", "MSG", "2.3"),
            ("DG1-3", "CE_0051", "CE", "2.3"),
        ] {
            assert!(based.contains(&expected), "{expected:?} in {based:?}");
        }
        assert!(
            based.iter().all(|(_, code, _, _)| *code != "TS"),
            "{based:?}"
        );
        let unmapped_types: Vec<&Outcome> = run
            .outcomes
            .iter()
            .filter(|outcome| {
                matches!(outcome, Outcome::NoDatatypeMap { source_type, .. }
                    if ["CM_MSG", "CE_0051", "TS"].contains(&source_type.as_str()))
            })
            .collect();
        assert!(unmapped_types.is_empty(), "{unmapped_types:?}");
    }

    #[test]
    fn a_legacy_code_the_guide_maps_keeps_the_guides_map() {
        let corpus = corpus();
        let msh = &hl7v2_types::legacy::v2_3::segment::msh::MSH;
        assert!(corpus.maps_from("datatype", "TS"));
        assert!(!corpus.maps_from("datatype", "CM_MSG"));
        assert_eq!(definition_type(&corpus, msh, 7).as_deref(), Some("TS"));
        let chosen = corpus
            .find("datatype", "TS", "dateTime")
            .expect("the guide's TS map");
        assert_eq!(chosen.id, "datatype-ts-to-datetime");
    }

    // NOTE: the guide's ORC-4 rows name `EIP` and the v2.9.1 definitions `EI`; a
    // v2.9.1 structure keeps the row's type and counts no substitution.
    #[test]
    fn a_current_field_keeps_the_type_its_row_names() {
        let text = "MSH|^~\\&|NORTHLAB|NORTHHOSP|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00013|P|2.5.1\r\
             PID|1||PAT-0013^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M\r\
             ORC|RE|PLC-3|FIL-3|GRP-3^NORTHLAB\r\
             OBR|1|PLC-3|FIL-3|2345-7^Glucose^LN\r";
        let lexed = parse::lex::lex(text, Charset::Ascii).expect("it lexes");
        let structure = parse::structure::structure_for(&lexed.message).expect("a structure");
        assert_eq!(structure.withdrawn_as_of, None);
        let parsed = parse::grouping::group(lexed, structure);
        assert_eq!(
            definition_type(&corpus(), &hl7v2_types::segment::orc::ORC, 4).as_deref(),
            Some("EI")
        );
        assert_eq!(
            field_type(Some(String::from("EIP")), Some(String::from("EI")), false),
            (Some(String::from("EIP")), None)
        );
        let corpus = corpus();
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let substituted = run
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Outcome::VersionTyped { .. }))
            .count();
        assert_eq!(substituted, 0, "{:?}", run.outcomes);
    }
}
