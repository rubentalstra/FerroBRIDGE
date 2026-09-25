// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The last pass of a run: every drafted record projected into its graph row
//! with the vocabulary's answers in hand.
//!
//! The projection of each key is `crate::model::projection`'s table. What a
//! part takes from a value follows the CDM v5.4 field definitions
//! (`OMOP_CDMv5.4_Field_Level.csv`): a source code with no standard concept
//! writes concept `0` and keeps its source value, units given but not mapped
//! write `unit_concept_id = 0` and absent units leave it NULL, an exact value
//! leaves `operator_concept_id` NULL, and a `*_source_concept_id` carries the
//! concept the source code names when the vocabulary holds one. A resolved
//! concept whose domain is not the one its column takes refuses the record:
//! the CDM routes a record by the domain of its standard concept
//! (<https://ohdsi.github.io/CommonDataModel/dataModelConventions.html>) and
//! OMOCL fixes the table in `type`, so a disagreement is never resolved by
//! moving the record.

use std::collections::BTreeMap;

use omop_cdm::generated::concept::Concept;
use omop_cdm::graph::ArchetypeRootPath;
use omop_cdm::graph::Discriminator;
use omop_cdm::graph::MappingName;
use omop_cdm::graph::OccurrencePath;
use omop_cdm::graph::RecordKey;
use omop_cdm::graph::Row;
use omop_cdm::graph::Source;
use omop_cdm::graph::VisitKey;
use omop_cdm::value::CdmDate;
use omop_cdm::vocabulary::ConceptCode;
use omop_cdm::vocabulary::Resolution;
use omop_cdm::vocabulary::SourceKey;
use omop_cdm::vocabulary::VocabularyId;
use openehr_rm::v1_2::data_types::quantity::dv_quantity::DvQuantity;
use rust_decimal::Decimal;

use crate::engine::RefusalKind;
use crate::engine::concept::Operator;
use crate::engine::concept::UNIT_VOCABULARY;
use crate::engine::concept::VocabularyAliases;
use crate::engine::datum;
use crate::engine::datum::Datum;
use crate::engine::row;
use crate::engine::row::Cell;
use crate::engine::row::Cells;
use crate::engine::row::Filled;
use crate::engine::row::RowError;
use crate::engine::walk::Chosen;
use crate::engine::walk::Draft;
use crate::engine::walk::Refused;
use crate::model::ast::ConceptId;
use crate::model::ast::Target;
use crate::model::projection::ColumnProjection;
use crate::model::projection::Fill;
use crate::model::projection::Part;

/// One vocabulary question a draft asks.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Question {
    /// A source code.
    Concept(SourceKey),
    /// An operator concept.
    Operator(Operator),
}

/// What the vocabulary answered for one source code.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Answer {
    /// The resolver's answer.
    Resolved(Box<Resolution>),
    /// More than one valid concept carries the code.
    Ambiguous(Vec<omop_cdm::vocabulary::ConceptId>),
}

/// Every answer of one run.
#[derive(Debug, Default)]
pub(crate) struct Answers {
    pub(crate) concepts: BTreeMap<(SourceKey, CdmDate), Answer>,
    pub(crate) operators: BTreeMap<(Operator, CdmDate), Option<ConceptId>>,
}

/// Returns the lookup key of a code, `None` when it cannot be one.
///
/// A code wider than `CONCEPT.vocabulary_id` or `CONCEPT.concept_code` names
/// no concept of the vocabulary, which is an absent concept: the column writes
/// concept `0`. No specification governs this reading: our own design.
fn key_of(vocabulary: &str, code: &str) -> Option<SourceKey> {
    // NOTE: a code wider than its CDM column is legitimately no vocabulary key,
    // so the refused construction reads as an unmapped code.
    let vocabulary = VocabularyId::new(vocabulary).ok()?;
    let code = ConceptCode::new(code).ok()?;
    Some(SourceKey::new(vocabulary, code))
}

/// Returns the key the coded value of a column is looked up by.
fn coded_key(datum: &Datum, aliases: &VocabularyAliases) -> Option<SourceKey> {
    match *datum {
        Datum::Coded { ref code, .. } => key_of(
            aliases.vocabulary_of(&code.terminology_id.value),
            &code.code_string,
        ),
        _ => None,
    }
}

/// Returns the key the units of a quantity are looked up by.
fn unit_key(datum: &Datum) -> Option<SourceKey> {
    match *datum {
        Datum::Quantity { ref quantity, .. } => key_of(UNIT_VOCABULARY, &quantity.units),
        _ => None,
    }
}

/// Returns the operator a quantity's `magnitude_status` names.
fn operator_of(datum: &Datum) -> Option<Operator> {
    match *datum {
        Datum::Quantity { ref quantity, .. } => quantity
            .magnitude_status
            .as_deref()
            .and_then(Operator::from_status),
        _ => None,
    }
}

/// Whether a key projects `part`.
fn projects(chosen: &Chosen, part: Part) -> bool {
    chosen
        .projection
        .columns
        .iter()
        .any(|column| column.part == part)
}

/// Returns every question `draft` asks the vocabulary.
pub(crate) fn questions(draft: &Draft<'_>, aliases: &VocabularyAliases) -> Vec<Question> {
    let mut asked = Vec::new();
    for chosen in &draft.columns {
        if projects(chosen, Part::Concept) || projects(chosen, Part::SourceConcept) {
            asked.extend(coded_key(&chosen.datum, aliases).map(Question::Concept));
        }
        if projects(chosen, Part::Units) {
            asked.extend(unit_key(&chosen.datum).map(Question::Concept));
        }
        if projects(chosen, Part::Operator) {
            asked.extend(operator_of(&chosen.datum).map(Question::Operator));
        }
    }
    asked
}

/// Returns the date a draft's concepts are resolved on.
///
/// A concept must be valid on the record date, which is the first date key
/// the record writes, or the composition's `context/start_time` for a record
/// that writes none. No specification governs the fallback: our own design.
pub(crate) fn record_date(draft: &Draft<'_>, fallback: Option<&CdmDate>) -> Option<CdmDate> {
    draft
        .columns
        .iter()
        .filter(|chosen| projects(chosen, Part::Date) || projects(chosen, Part::Datetime))
        .find_map(|chosen| match chosen.datum {
            Datum::Moment(ref moment) => Some(moment.date.clone()),
            _ => None,
        })
        .or_else(|| fallback.cloned())
}

/// The domain a target's primary concept belongs to, by the CDM's domain
/// names (<https://ohdsi.github.io/CommonDataModel/dataModelConventions.html>).
pub(crate) const fn target_domain(target: Target) -> Option<&'static str> {
    match target {
        Target::Measurement => Some("Measurement"),
        Target::Observation => Some("Observation"),
        Target::DrugExposure => Some("Drug"),
        Target::ConditionOccurrence => Some("Condition"),
        Target::ProcedureOccurrence => Some("Procedure"),
        Target::DeviceExposure => Some("Device"),
        Target::Specimen => Some("Specimen"),
        Target::Visit => Some("Visit"),
        Target::Death | Target::Person => None,
    }
}

/// The keys whose concept is the record's primary concept.
const PRIMARY_KEYS: &[&str] = &["concept_id", "visit_concept"];

/// Projects drafted records into rows.
#[derive(Debug)]
pub(crate) struct Finisher<'a> {
    pub(crate) answers: &'a Answers,
    pub(crate) aliases: &'a VocabularyAliases,
    pub(crate) source: &'a Source,
    pub(crate) visit: Option<&'a VisitKey>,
    pub(crate) type_concept: i32,
    pub(crate) fallback_date: Option<&'a CdmDate>,
}

/// The state of projecting one record.
#[derive(Debug)]
struct Projection<'d> {
    target: Target,
    date: Option<CdmDate>,
    chosen: &'d Chosen,
    cells: Cells,
    /// The standard concept this row writes as its primary concept, when its
    /// source code maps to several.
    primary: Option<&'d Concept>,
}

/// Returns the key of the `branch`th row of a draft.
///
/// The occurrence is the record's instance path below its archetype root, and
/// the discriminator names the mapping, the entry and the branch, because one
/// node may feed several rows of one table. No specification governs this:
/// our own design.
fn key_of_draft(draft: &Draft<'_>, source: &Source, branch: u16) -> Result<RecordKey, Refused> {
    let below = draft
        .path
        .strip_prefix(draft.root_path.as_str())
        .filter(|below| !below.is_empty())
        .unwrap_or("/");
    let invalid = |message: String| {
        Refused::new(RefusalKind::InvalidValue, None, draft.path.clone(), message)
    };
    let entry = u16::try_from(draft.record.entry).map_err(|_wide| {
        invalid(format!(
            "the entry index {} is too large",
            draft.record.entry
        ))
    })?;
    let empty = |error: omop_cdm::graph::EmptyIdentifier| invalid(error.to_string());
    Ok(RecordKey::new(
        source.ehr_id().clone(),
        source.versioned_object_uid().clone(),
        ArchetypeRootPath::new(draft.root_path.clone()).map_err(empty)?,
        OccurrencePath::new(below).map_err(empty)?,
        Discriminator::new(
            MappingName::new(draft.mapping.clone()).map_err(empty)?,
            entry,
            branch,
        ),
    ))
}

impl Finisher<'_> {
    /// Projects one draft into its rows: one, or one per standard concept its
    /// primary source code maps to.
    pub(crate) fn finish(&self, draft: &Draft<'_>) -> Result<Vec<Row>, Refused> {
        let target = draft.record.target;
        let date = record_date(draft, self.fallback_date);
        let branches = self.branches(draft, date.as_ref());
        let mut rows = Vec::new();
        for (index, primary) in branches.iter().enumerate() {
            let branch = u16::try_from(index).map_err(|_wide| {
                Refused::new(
                    RefusalKind::InvalidValue,
                    None,
                    draft.path.clone(),
                    "a source code maps to more standard concepts than a key numbers",
                )
            })?;
            let mut cells = Cells::default();
            for chosen in &draft.columns {
                let mut projection = Projection {
                    target,
                    date: date.clone(),
                    chosen,
                    cells,
                    primary: primary.as_ref(),
                };
                self.column(&mut projection)?;
                cells = projection.cells;
            }
            let key = key_of_draft(draft, self.source, branch)?;
            let filled = Filled {
                person: self.source.ehr_id(),
                visit: self.visit,
                type_concept: self.type_concept,
            };
            let row = row::build(target.table(), key, &cells, &filled).map_err(|error| {
                let (kind, column) = match error {
                    RowError::Graph(omop_cdm::graph::GraphError::Missing { column, .. }) => {
                        (RefusalKind::RequiredColumn, Some(column))
                    }
                    RowError::Invalid { column, .. } => (RefusalKind::InvalidValue, Some(column)),
                    RowError::Graph(_) => (RefusalKind::InvalidValue, None),
                };
                Refused::new(kind, column, draft.path.clone(), error.to_string())
            })?;
            rows.push(row);
        }
        Ok(rows)
    }

    /// Returns the standard concept each row of a draft writes as its primary
    /// concept, `[None]` for a draft that writes one row.
    ///
    /// The CDM conventions allow "one `SOURCE_CONCEPT_ID` to map to multiple
    /// Standard `CONCEPT_ID`s within the same domain", "in which case multiple
    /// `CONDITION_OCCURRENCE` records will be generated for the one source value
    /// record"
    /// (<https://ohdsi.github.io/CommonDataModel/dataModelConventions.html>).
    fn branches(&self, draft: &Draft<'_>, date: Option<&CdmDate>) -> Vec<Option<Concept>> {
        let primary = draft.columns.iter().find(|chosen| {
            PRIMARY_KEYS.contains(&chosen.projection.key) && projects(chosen, Part::Concept)
        });
        let Some(chosen) = primary else {
            return vec![None];
        };
        let Some(key) = coded_key(&chosen.datum, self.aliases) else {
            return vec![None];
        };
        // NOTE: CDM conventions, "Mapping": a source concept mapped to several
        // standard concepts in one domain writes one record per concept.
        let answer = date.and_then(|date| self.answers.concepts.get(&(key, date.clone())));
        match answer {
            Some(Answer::Resolved(resolution)) if resolution.standard_concepts().len() > 1 => {
                resolution
                    .standard_concepts()
                    .iter()
                    .cloned()
                    .map(Some)
                    .collect()
            }
            _ => vec![None],
        }
    }

    /// Projects one key into the columns it writes.
    fn column(&self, projection: &mut Projection<'_>) -> Result<(), Refused> {
        let chosen = projection.chosen;
        let kind = match chosen.projection.fill {
            Fill::Every => None,
            Fill::OneByValueKind => Some(match chosen.datum {
                Datum::Quantity { .. } | Datum::Number(_) => Part::Number,
                Datum::Coded { .. } | Datum::Mapped { .. } | Datum::Literal(_) => Part::Concept,
                _ => Part::Text,
            }),
        };
        for column in chosen.projection.columns {
            let by_kind = matches!(column.part, Part::Number | Part::Concept | Part::Text);
            if kind.is_some_and(|kind| by_kind && column.part != kind) {
                continue;
            }
            if let Some(cell) = self.cell(projection, column)? {
                projection.cells.set(column.column, cell);
            }
        }
        Ok(())
    }

    /// Returns the value one column takes from the chosen value.
    fn cell(
        &self,
        projection: &Projection<'_>,
        column: &ColumnProjection,
    ) -> Result<Option<Cell>, Refused> {
        let chosen = projection.chosen;
        let datum = &chosen.datum;
        let unsupported = || {
            Refused::new(
                RefusalKind::UnsupportedValue,
                Some(chosen.projection.key),
                chosen.path.clone(),
                format!("`{}` cannot take the value read", column.column),
            )
        };
        let cell = match column.part {
            Part::Concept => Some(self.concept(projection, column)?),
            Part::Units => match *datum {
                Datum::Literal(code) => Some(Cell::Int(code.get())),
                Datum::Quantity { ref quantity, .. } if quantity.units.is_empty() => None,
                Datum::Quantity { .. } => Some(self.concept(projection, column)?),
                _ => return Err(unsupported()),
            },
            Part::SourceConcept => self.source_concept(projection)?,
            Part::SourceValue => source_value(datum, projects(chosen, Part::Units)).map(Cell::Text),
            Part::Date => match *datum {
                Datum::Moment(ref moment) => Some(Cell::Date(moment.date.clone())),
                _ => return Err(unsupported()),
            },
            Part::Datetime => match *datum {
                Datum::Moment(ref moment) => moment.datetime.clone().map(Cell::Datetime),
                _ => return Err(unsupported()),
            },
            Part::Year => match *datum {
                Datum::Moment(ref moment) => moment
                    .date
                    .as_str()
                    .get(..4)
                    .and_then(|year| year.parse::<i32>().ok())
                    .map(Cell::Int),
                _ => return Err(unsupported()),
            },
            Part::Number => Some(Cell::Number(match *datum {
                Datum::Quantity { ref magnitude, .. } | Datum::Number(ref magnitude) => {
                    magnitude.value
                }
                Datum::Literal(code) => Decimal::from(code.get()),
                _ => return Err(unsupported()),
            })),
            Part::Text => Some(Cell::Text(text_of(datum).ok_or_else(unsupported)?)),
            Part::RangeLow | Part::RangeHigh => match *datum {
                Datum::Quantity { ref quantity, .. } => range(quantity, column, chosen)?,
                _ => return Err(unsupported()),
            },
            Part::Operator => self.operator(projection)?,
            // TODO(#243): a provider or visit detail row has no natural key the
            // graph's references can name yet, so a present value refuses.
            Part::Reference => {
                return Err(Refused::new(
                    RefusalKind::UnsupportedValue,
                    Some(chosen.projection.key),
                    chosen.path.clone(),
                    format!(
                        "`{}` references a row no natural key of the graph names",
                        column.column
                    ),
                ));
            }
        };
        Ok(cell)
    }

    /// Returns the answer to one source-code question.
    fn answer(&self, projection: &Projection<'_>, key: &SourceKey) -> Result<&Answer, Refused> {
        let chosen = projection.chosen;
        let refuse = |message: String| {
            Refused::new(
                RefusalKind::InvalidValue,
                Some(chosen.projection.key),
                chosen.path.clone(),
                message,
            )
        };
        let date = projection.date.clone().ok_or_else(|| {
            refuse("the record carries no date to resolve its concepts on".to_owned())
        })?;
        self.answers
            .concepts
            .get(&(key.clone(), date))
            .ok_or_else(|| refuse(format!("{key} was not resolved")))
    }

    /// Returns the standard concept a column takes, with its domain checked.
    fn concept(
        &self,
        projection: &Projection<'_>,
        column: &ColumnProjection,
    ) -> Result<Cell, Refused> {
        let chosen = projection.chosen;
        let refuse = |kind: RefusalKind, message: String| {
            Refused::new(
                kind,
                Some(chosen.projection.key),
                chosen.path.clone(),
                message,
            )
        };
        let key = match chosen.datum {
            Datum::Literal(code) | Datum::Mapped { concept: code, .. } => {
                return Ok(Cell::Int(code.get()));
            }
            Datum::Coded { .. } => coded_key(&chosen.datum, self.aliases),
            Datum::Quantity { .. } if column.part == Part::Units => unit_key(&chosen.datum),
            Datum::Text(_) | Datum::Flag(_) | Datum::Identifier(_) => None,
            _ => {
                return Err(refuse(
                    RefusalKind::UnsupportedValue,
                    format!("`{}` takes a coded value", column.column),
                ));
            }
        };
        let Some(key) = key else {
            return Ok(Cell::Int(ConceptId::NO_MATCHING_CONCEPT.get()));
        };
        let resolution = match *self.answer(projection, &key)? {
            Answer::Ambiguous(ref ids) => {
                return Err(refuse(
                    RefusalKind::AmbiguousConcept,
                    format!("{key} names {} valid concepts", ids.len()),
                ));
            }
            Answer::Resolved(ref resolution) => resolution,
        };
        let fanned = PRIMARY_KEYS.contains(&chosen.projection.key) && column.part == Part::Concept;
        if let Some(concept) = projection.primary.filter(|_| fanned) {
            check_domain(projection, column, concept)?;
            return Ok(Cell::Int(concept.concept_id));
        }
        let concept = match resolution.standard_concepts() {
            [] => return Ok(Cell::Int(ConceptId::NO_MATCHING_CONCEPT.get())),
            [only] => only,
            several => {
                // NOTE: no specification governs this: our own design; only the
                // primary concept fans out into rows, so another column refuses.
                return Err(refuse(
                    RefusalKind::SeveralStandardConcepts,
                    format!("{key} maps to {} standard concepts", several.len()),
                ));
            }
        };
        check_domain(projection, column, concept)?;
        Ok(Cell::Int(concept.concept_id))
    }

    /// Returns the source concept of a coded value or of a quantity's units.
    fn source_concept(&self, projection: &Projection<'_>) -> Result<Option<Cell>, Refused> {
        let chosen = projection.chosen;
        let key = if projects(chosen, Part::Units) {
            unit_key(&chosen.datum)
        } else {
            coded_key(&chosen.datum, self.aliases)
        };
        let Some(key) = key else {
            return Ok(None);
        };
        match *self.answer(projection, &key)? {
            Answer::Ambiguous(ref ids) => Err(Refused::new(
                RefusalKind::AmbiguousConcept,
                Some(chosen.projection.key),
                chosen.path.clone(),
                format!("{key} names {} valid concepts", ids.len()),
            )),
            Answer::Resolved(ref resolution) => Ok(resolution
                .source_concept()
                .map(|concept| Cell::Int(concept.concept_id))),
        }
    }

    /// Returns the operator concept of a quantity's `magnitude_status`.
    fn operator(&self, projection: &Projection<'_>) -> Result<Option<Cell>, Refused> {
        let chosen = projection.chosen;
        let refuse = |kind: RefusalKind, message: &str| {
            Refused::new(
                kind,
                Some(chosen.projection.key),
                chosen.path.clone(),
                message,
            )
        };
        let status = match chosen.datum {
            Datum::Literal(code) => return Ok(Some(Cell::Int(code.get()))),
            Datum::Quantity { ref quantity, .. } => quantity.magnitude_status.clone(),
            _ => {
                return Err(refuse(
                    RefusalKind::UnsupportedValue,
                    "an operator is read from a quantity",
                ));
            }
        };
        let Some(status) = status.filter(|status| status != "=") else {
            return Ok(None);
        };
        let concept = match (Operator::from_status(&status), projection.date.clone()) {
            (Some(operator), Some(date)) => self
                .answers
                .operators
                .get(&(operator, date))
                .copied()
                .flatten(),
            (Some(_), None) => {
                return Err(refuse(
                    RefusalKind::InvalidValue,
                    "the record carries no date to resolve its operator on",
                ));
            }
            (None, _) => None,
        };
        Ok(Some(Cell::Int(
            concept.unwrap_or(ConceptId::NO_MATCHING_CONCEPT).get(),
        )))
    }
}

/// Returns the source value of a chosen value, verbatim.
///
/// The key that projects a quantity's units takes the units; every other key
/// takes the code, the text or the number as the source writes it.
fn source_value(datum: &Datum, units: bool) -> Option<String> {
    match *datum {
        Datum::Quantity { ref quantity, .. } if units => {
            (!quantity.units.is_empty()).then(|| quantity.units.clone())
        }
        Datum::Literal(_) => None,
        Datum::Coded { ref code, .. } => Some(code.code_string.clone()),
        Datum::Mapped { ref code, .. } => Some(code.clone()),
        _ => text_of(datum),
    }
}

/// Returns a value as text, for a text column.
fn text_of(datum: &Datum) -> Option<String> {
    match *datum {
        Datum::Text(ref text) | Datum::Identifier(ref text) | Datum::Coded { ref text, .. } => {
            Some(text.clone())
        }
        Datum::Mapped { ref code, .. } => Some(code.clone()),
        Datum::Quantity { ref magnitude, .. } | Datum::Number(ref magnitude) => {
            Some(magnitude.text.clone())
        }
        Datum::Moment(ref moment) => Some(moment.written.clone()),
        Datum::Flag(flag) => Some(flag.to_string()),
        Datum::Literal(_) => None,
    }
}

/// Returns one bound of a quantity's `normal_range`.
///
/// The CDM states that "ranges have the same unit as the `VALUE_AS_NUMBER`"
/// (`measurement.range_low`), so a bound in another unit refuses the record.
fn range(
    quantity: &DvQuantity,
    column: &ColumnProjection,
    chosen: &Chosen,
) -> Result<Option<Cell>, Refused> {
    let Some(ref interval) = quantity.normal_range else {
        return Ok(None);
    };
    let bound = if column.part == Part::RangeLow {
        interval.lower.as_ref()
    } else {
        interval.upper.as_ref()
    };
    let Some(limit) = bound else {
        return Ok(None);
    };
    let refuse = |kind: RefusalKind, message: String| {
        Refused::new(
            kind,
            Some(chosen.projection.key),
            chosen.path.clone(),
            message,
        )
    };
    if limit.units != quantity.units {
        return Err(refuse(
            RefusalKind::RangeUnit,
            format!("`{}` is in another unit than the value", column.column),
        ));
    }
    let limit = datum::number(limit.magnitude, "DV_QUANTITY")
        .map_err(|error| refuse(RefusalKind::InvalidValue, error.to_string()))?;
    Ok(Some(Cell::Number(limit.value)))
}

/// Refuses a concept whose domain its column does not take.
fn check_domain(
    projection: &Projection<'_>,
    column: &ColumnProjection,
    concept: &Concept,
) -> Result<(), Refused> {
    let chosen = projection.chosen;
    let declared = omop_cdm::meta::table(projection.target.table())
        .and_then(|table| table.column(column.column))
        .and_then(|meta| meta.fk_domain);
    let primary = (PRIMARY_KEYS.contains(&chosen.projection.key) && column.part == Part::Concept)
        .then(|| target_domain(projection.target))
        .flatten();
    let Some(expected) = declared.or(primary) else {
        return Ok(());
    };
    if concept.domain_id.as_str() == expected {
        return Ok(());
    }
    Err(Refused::new(
        RefusalKind::DomainMismatch,
        Some(chosen.projection.key),
        chosen.path.clone(),
        format!(
            "concept {} is in the `{}` domain; `{}.{}` takes the `{expected}` domain",
            concept.concept_id,
            concept.domain_id.as_str(),
            projection.target.table(),
            column.column
        ),
    ))
}
