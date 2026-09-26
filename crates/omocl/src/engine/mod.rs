// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OMOCL interpreter: one composition in, one record graph out.
//!
//! [`run`] walks a composition with a [`Program`] compiled for its template,
//! asks the vocabulary each distinct question once, projects every record
//! instance into its `omop_cdm::graph` row under a natural key, runs the
//! `CustomMapping` converters over the rows each scope instance produced, and
//! records what it refused and what it left unread in the graph's report; the
//! writer counts what it writes. A record that cannot be written is refused on
//! its own and the rest of the composition still runs. A failure of the
//! vocabulary itself, a program of another template or a missing domain
//! concept refuses the run, because a partial graph would leave rows the
//! report cannot account for.
//!
//! `person_id`, `visit_occurrence_id`, `*_type_concept_id`, `*_source_value`
//! and `*_source_concept_id` appear in no OMOCL file. The engine fills the
//! source columns from the value read, the type concept from
//! [`Seams::type_concept`] (the provenance a deployment knows, as the CDM field
//! definitions describe it), and the person and the visit as references the
//! writer resolves. No specification governs that division: our own design.

pub mod concept;
pub mod custom;
mod datum;
mod finish;
mod navigate;
mod row;
mod walk;

use core::fmt;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use omop_cdm::graph::RecordGraph;
use omop_cdm::graph::key::EmptyIdentifier;
use omop_cdm::graph::key::MappingName;
use omop_cdm::graph::key::Source;
use omop_cdm::graph::key::VisitKey;
use omop_cdm::graph::link::Link;
use omop_cdm::graph::link::LinkEnd;
use omop_cdm::graph::report::Refusal;
use omop_cdm::graph::report::UnmappedField;
use omop_cdm::graph::row::GraphError;
use omop_cdm::graph::row::Row;
use omop_cdm::value::CdmDate;
use omop_cdm::vocabulary::ResolveError;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::index::WebTemplateIndex;

use crate::engine::concept::ConceptSource;
use crate::engine::concept::LookupError;
use crate::engine::concept::VocabularyAliases;
use crate::engine::finish::Answer;
use crate::engine::finish::Answers;
use crate::engine::finish::Finisher;
use crate::engine::finish::Question;
use crate::engine::walk::Walk;
use crate::model::ast::ConceptId;
use crate::model::ast::Target;
use crate::resolve::program::BoundRecord;
use crate::resolve::program::Program;

/// What a run is handed from outside the mapping.
#[derive(Debug)]
pub struct Seams<'a, S> {
    /// The vocabulary.
    pub concepts: &'a S,
    /// The `*_type_concept_id` every row carries.
    pub type_concept: ConceptId,
    /// The reading of an openEHR terminology id as a `vocabulary_id`.
    pub aliases: &'a VocabularyAliases,
}

/// Why one record was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum RefusalKind {
    /// A column the mapping requires had no alternative present.
    MissingColumn,
    /// A column the CDM requires received no value.
    RequiredColumn,
    /// A path matched several nodes where the record does not iterate.
    Multiple,
    /// A `conceptMap` read an at-code its table does not list.
    UnlistedAtCode,
    /// A resolved concept belongs to a domain its column does not take.
    DomainMismatch,
    /// The vocabulary holds more than one concept for a source code.
    AmbiguousConcept,
    /// A source code maps to more than one standard concept.
    SeveralStandardConcepts,
    /// A value is of an openEHR type the column cannot take.
    UnsupportedValue,
    /// A value does not fit its CDM column type.
    InvalidValue,
    /// A `multiplication` overflowed.
    Overflow,
    /// A `normal_range` is in another unit than the value.
    RangeUnit,
}

/// One refused record: which, where, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRefusal {
    /// The `metadata.name` of the mapping.
    pub mapping: String,
    /// The index of the entry within `mappings`.
    pub entry: usize,
    /// The CDM table the record would have written.
    pub table: &'static str,
    /// The OMOCL key of the column, when one column caused the refusal.
    pub column: Option<&'static str>,
    /// The instance path the refusal is about.
    pub path: String,
    /// The kind of refusal.
    pub kind: RefusalKind,
    /// The refusal rendered for a person.
    pub message: String,
}

impl fmt::Display for RecordRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}#{} {} at {}",
            self.mapping, self.entry, self.table, self.path
        )?;
        if let Some(column) = self.column {
            write!(f, ", `{column}`")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// A produced value with the typed refusals of producing it.
///
/// The graph's report carries the same refusals rendered; this list keeps
/// their kinds for a caller that branches on them.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome<T> {
    value: T,
    refusals: Vec<RecordRefusal>,
}

impl<T> Outcome<T> {
    /// Returns the produced value.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Returns the refused records, in the order the run met them.
    #[must_use]
    pub fn refusals(&self) -> &[RecordRefusal] {
        &self.refusals
    }

    /// Returns how many records were refused for `kind`.
    #[must_use]
    pub fn refused(&self, kind: RefusalKind) -> usize {
        self.refusals
            .iter()
            .filter(|refusal| refusal.kind == kind)
            .count()
    }

    /// Returns the value and the refusals, consuming the outcome.
    #[must_use]
    pub fn into_parts(self) -> (T, Vec<RecordRefusal>) {
        (self.value, self.refusals)
    }
}

/// Why a run produced no graph.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EngineError {
    /// The composition was built against another template than the program.
    #[error("the program is compiled for `{program}` and the composition names `{composition}`")]
    TemplateMismatch {
        /// The program's template.
        program: String,
        /// The composition's template.
        composition: String,
    },
    /// The index is of another template than the program.
    #[error("the program is compiled for `{program}` and the index is of `{index}`")]
    IndexMismatch {
        /// The program's template.
        program: String,
        /// The index's template.
        index: String,
    },
    /// The vocabulary failed a lookup.
    #[error(transparent)]
    Lookup(#[from] LookupError),
    /// The vocabulary failed to resolve a source code.
    #[error("the vocabulary failed")]
    Vocabulary {
        /// The resolver's error.
        #[source]
        source: ResolveError,
    },
    /// The vocabulary holds no concept for a domain a link names.
    #[error("the vocabulary's DOMAIN table holds no `{domain}`")]
    MissingDomainConcept {
        /// The domain.
        domain: &'static str,
    },
    /// The graph refused a link.
    #[error("the record graph refused the run's output")]
    Graph {
        /// The graph's refusal.
        #[source]
        source: GraphError,
    },
    /// A mapping name was empty.
    #[error(transparent)]
    Identifier(#[from] EmptyIdentifier),
}

/// Runs `program` over one composition.
///
/// `source` names the composition, and `visit` the visit its rows belong to
/// when the visit derivation assigned one.
///
/// # Errors
///
/// Returns [`EngineError::TemplateMismatch`] or
/// [`EngineError::IndexMismatch`] when the composition or the index is of
/// another template, [`EngineError::Lookup`] when the vocabulary fails,
/// [`EngineError::MissingDomainConcept`] when a link names a domain the
/// vocabulary does not hold, and [`EngineError::Graph`] when the graph refuses
/// a link. A record the mapping cannot write is no error: it is refused and
/// reported.
pub async fn run<S: ConceptSource>(
    program: &Program,
    composition: &CanonicalComposition,
    index: &WebTemplateIndex,
    source: &Source,
    visit: Option<&VisitKey>,
    seams: &Seams<'_, S>,
) -> Result<Outcome<RecordGraph>, EngineError> {
    if composition.template_id() != program.template_id() {
        return Err(EngineError::TemplateMismatch {
            program: program.template_id().to_owned(),
            composition: composition.template_id().to_owned(),
        });
    }
    if index.template_id() != program.template_id() {
        return Err(EngineError::IndexMismatch {
            program: program.template_id().to_owned(),
            index: index.template_id().to_owned(),
        });
    }
    let walk = Walk::run(program, composition.value());
    let fallback = context_date(composition);
    let answers = ask(&walk, seams, fallback.as_ref()).await?;
    let finisher = Finisher {
        answers: &answers,
        aliases: seams.aliases,
        source,
        visit,
        type_concept: seams.type_concept.get(),
        fallback_date: fallback.as_ref(),
    };

    let mut graph = RecordGraph::new(source.clone());
    let mut refusals = Vec::new();
    for failed in &walk.failed {
        refusals.push(refusal(
            &failed.mapping,
            failed.record,
            failed.refused.clone(),
        ));
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for draft in &walk.drafts {
        let rows = match finisher.finish(draft) {
            Ok(rows) => rows,
            Err(refused) => {
                refusals.push(refusal(&draft.mapping, draft.record, refused));
                continue;
            }
        };
        for row in rows {
            graph
                .push_row(row)
                .map_err(|source| EngineError::Graph { source })?;
            groups.push(draft.groups.clone());
        }
    }
    for refused in &refusals {
        graph.report_mut().refuse(with_column(
            Refusal::new(refused.to_string())
                .with_mapping(MappingName::new(refused.mapping.clone())?)
                .with_table(refused.table)
                .with_element(refused.path.clone()),
            refused.column,
        ));
    }
    for link in links(&walk, graph.rows(), &groups, seams).await? {
        graph
            .push_link(link)
            .map_err(|source| EngineError::Graph { source })?;
    }
    for field in unmapped(&walk, composition)? {
        graph.report_mut().unmapped(field);
    }
    Ok(Outcome {
        value: graph,
        refusals,
    })
}

/// Returns the date of the composition's `context/start_time`.
fn context_date(composition: &CanonicalComposition) -> Option<CdmDate> {
    let segment = openehr_rm::v1_2::paths::PathSegment {
        attribute: "context".to_owned(),
        predicate: openehr_rm::v1_2::paths::Predicate::default(),
        descendant: false,
    };
    let found = openehr_rm::v1_2::paths::select_children(composition.value(), &segment);
    let context = found.first()?;
    // NOTE: a context with no complete start date is legitimately no fallback
    // date; a record that needs one is refused for its own lack.
    match datum::read(context).ok().flatten() {
        Some(datum::Datum::Moment(moment)) => Some(moment.date),
        _ => None,
    }
}

/// Asks the vocabulary every distinct question the drafts raise, once.
async fn ask<S: ConceptSource>(
    walk: &Walk<'_, '_>,
    seams: &Seams<'_, S>,
    fallback: Option<&CdmDate>,
) -> Result<Answers, EngineError> {
    let mut asked: BTreeSet<(Question, CdmDate)> = BTreeSet::new();
    for draft in &walk.drafts {
        let Some(date) = finish::record_date(draft, fallback) else {
            continue;
        };
        for question in finish::questions(draft, seams.aliases) {
            asked.insert((question, date.clone()));
        }
    }
    let mut answers = Answers::default();
    for (question, date) in asked {
        match question {
            Question::Concept(key) => {
                let answer = match seams.concepts.resolve(&key, &date).await {
                    Ok(resolution) => Answer::Resolved(Box::new(resolution)),
                    Err(ResolveError::Ambiguous { concept_ids, .. }) => {
                        Answer::Ambiguous(concept_ids)
                    }
                    Err(source) => return Err(EngineError::Vocabulary { source }),
                };
                answers.concepts.insert((key, date), answer);
            }
            Question::Operator(operator) => {
                let concept = seams.concepts.operator(operator, &date).await?;
                answers.operators.insert((operator, date), concept);
            }
        }
    }
    Ok(answers)
}

/// Runs every converter over the rows of its scope instance.
async fn links<S: ConceptSource>(
    walk: &Walk<'_, '_>,
    rows: &[Row],
    groups: &[Vec<usize>],
    seams: &Seams<'_, S>,
) -> Result<Vec<Link>, EngineError> {
    let mut relations = Vec::new();
    for task in &walk.tasks {
        let members: Vec<&Row> = rows
            .iter()
            .zip(groups)
            .filter(|&(_, chain)| chain.contains(&task.group))
            .map(|(row, _)| row)
            .collect();
        relations.append(&mut task.converter.relate(&members));
    }
    let mut domains: BTreeMap<&'static str, i32> = BTreeMap::new();
    let mut links = Vec::new();
    for relation in relations {
        let mut ends = Vec::new();
        for fact in [relation.first, relation.second] {
            let domain = table_domain(fact.table)
                .ok_or(EngineError::MissingDomainConcept { domain: fact.table })?;
            let concept = if let Some(&concept) = domains.get(domain) {
                concept
            } else {
                let concept = seams
                    .concepts
                    .domain_concept(domain)
                    .await?
                    .ok_or(EngineError::MissingDomainConcept { domain })?
                    .get();
                domains.insert(domain, concept);
                concept
            };
            ends.push(
                LinkEnd::new(fact.table, fact.key, concept)
                    .map_err(|source| EngineError::Graph { source })?,
            );
        }
        let mut ends = ends.into_iter();
        if let (Some(first), Some(second)) = (ends.next(), ends.next()) {
            links.push(Link::new(first, second));
        }
    }
    Ok(links)
}

/// Returns every valued element no chosen alternative read.
///
/// An element is reported under the mapping of the deepest scope instance
/// that holds it; an element outside every mapped archetype has no mapping
/// the report can name it under. No specification governs this: our own
/// design.
fn unmapped(
    walk: &Walk<'_, '_>,
    composition: &CanonicalComposition,
) -> Result<Vec<UnmappedField>, EngineError> {
    let mut fields = Vec::new();
    for element in navigate::valued_elements(composition.value()) {
        if walk.consumed.contains(&element) {
            continue;
        }
        let owner = walk
            .scopes
            .iter()
            .filter(|(root, _)| element.starts_with(root.as_str()))
            .max_by_key(|(root, _)| root.len());
        if let Some((_, mapping)) = owner {
            fields.push(UnmappedField::new(
                MappingName::new(mapping.clone())?,
                element,
            ));
        }
    }
    Ok(fields)
}

/// Returns `refusal` naming `column`, when one caused it.
fn with_column(refusal: Refusal, column: Option<&'static str>) -> Refusal {
    match column {
        Some(column) => refusal.with_column(column),
        None => refusal,
    }
}

/// Returns the domain the rows of a CDM table belong to.
fn table_domain(table: &str) -> Option<&'static str> {
    Target::ALL
        .iter()
        .find(|target| target.table() == table)
        .and_then(|target| finish::target_domain(*target))
}

/// Renders one refusal.
fn refusal(mapping: &str, record: &BoundRecord, refused: walk::Refused) -> RecordRefusal {
    RecordRefusal {
        mapping: mapping.to_owned(),
        entry: record.entry(),
        table: record.target().table(),
        column: refused.column,
        path: refused.path,
        kind: refused.kind,
        message: refused.message,
    }
}
