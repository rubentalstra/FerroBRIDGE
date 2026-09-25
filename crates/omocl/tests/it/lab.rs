// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the resolve and engine suites share: the synthetic laboratory
//! template, the synthetic composition built against it, mapping sets over
//! the corpus plus inline synthetic files, and a map-backed vocabulary.
//!
//! Every code, concept id and value here is invented for the tests.

use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;
use std::sync::Mutex;

use omocl::engine::Seams;
use omocl::engine::concept::ConceptSource;
use omocl::engine::concept::LookupError;
use omocl::engine::concept::Operator;
use omocl::engine::concept::VocabularyAliases;
use omocl::model::ast::ConceptId;
use omocl::model::load::MappingSet;
use omocl::model::semantic::FirstPartyConverters;
use omocl::resolve::program::Program;
use omop_cdm::generated::concept::Concept;
use omop_cdm::graph::EhrId;
use omop_cdm::graph::Source;
use omop_cdm::graph::VersionUid;
use omop_cdm::graph::VersionedObjectUid;
use omop_cdm::graph::VisitKey;
use omop_cdm::graph::VisitSource;
use omop_cdm::value::CdmDate;
use omop_cdm::vocabulary::ConceptCode;
use omop_cdm::vocabulary::Resolution;
use omop_cdm::vocabulary::ResolveError;
use omop_cdm::vocabulary::SourceKey;
use omop_cdm::vocabulary::VocabularyId;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::template::TemplateSource;
use serde_json::Value;

use crate::support::corpus_path;
use crate::support::render;

/// The corpus files the laboratory result needs: the result, the two
/// clusters it includes.
pub(crate) const LABORATORY: &[&str] = &[
    "medical_data/observation/Laboratory_test_result_v1.yml",
    "medical_data/cluster/Laboratory_test_analyte_v1.yml",
    "medical_data/cluster/Specimen_v1.yml",
];

/// The FLAT prefix of the result's first event.
pub(crate) const EVENT: &str =
    "synthetic_laboratory_report/synthetic_laboratory_result:0/any_event:0";

/// The type concept the tests hand in.
pub(crate) const TYPE_CONCEPT: i32 = 7_001;

/// Builds the index over the synthetic laboratory template.
pub(crate) fn template() -> Result<WebTemplateIndex, Box<dyn Error>> {
    let source = TemplateSource::opt14(ferrobridge_testkit::fixtures::LABORATORY_REPORT_OPT)?;
    Ok(WebTemplateIndex::build(&source)?)
}

/// Returns the FLAT fixture with `set` written over it and `drop` removed.
pub(crate) fn flat(
    set: &[(&str, Value)],
    drop: &[&str],
) -> Result<serde_json::Map<String, Value>, Box<dyn Error>> {
    let mut flat: serde_json::Map<String, Value> =
        serde_json::from_str(ferrobridge_testkit::fixtures::LABORATORY_REPORT_FLAT)?;
    for key in drop {
        flat.retain(|written, _| !written.starts_with(key));
    }
    for (key, value) in set {
        flat.insert((*key).to_owned(), value.clone());
    }
    Ok(flat)
}

/// Builds the synthetic composition from a FLAT document.
pub(crate) fn composition(
    index: &WebTemplateIndex,
    flat: &serde_json::Map<String, Value>,
) -> Result<CanonicalComposition, Box<dyn Error>> {
    Ok(index.build_from_flat(flat, "2026-09-25T00:00:00Z")?)
}

/// Returns the canonical JSON of a composition changed by `edit`.
///
/// Some RM attributes (`normal_range`, `magnitude_status`, a date-only
/// `DV_DATE_TIME`) have no FLAT key the fixture writes, so a test sets them
/// on the canonical tree directly.
pub(crate) fn edited(
    composition: &CanonicalComposition,
    edit: impl FnOnce(&mut Value),
) -> CanonicalComposition {
    let mut value = composition.value().clone();
    edit(&mut value);
    CanonicalComposition::new(value, composition.template_id(), composition.generation())
}

/// The JSON pointer of the first analyte of the first event.
pub(crate) const FIRST_ANALYTE_NODE: &str = "/content/0/data/events/0/data/items/0";

/// Returns the `DV_QUANTITY` of the first analyte of the first event.
pub(crate) fn first_quantity(composition: &mut Value) -> Option<&mut Value> {
    composition.pointer_mut(&format!("{FIRST_ANALYTE_NODE}/items/1/value"))
}

/// Loads the named corpus files and the inline synthetic files into one set.
pub(crate) fn set(corpus: &[&str], inline: &[(&str, &str)]) -> Result<MappingSet, Box<dyn Error>> {
    let mut set = MappingSet::new();
    for relative in corpus {
        let path = corpus_path(relative);
        let document = openehr_mapping_core::loader::load_file(&path)?;
        let file = omocl::model::load::load_file(&path, &FirstPartyConverters)
            .map_err(|diagnostics| render(&diagnostics))?;
        set.insert(document, file)
            .map_err(|error| error.to_string())?;
    }
    for (name, source) in inline {
        let document = openehr_mapping_core::loader::load_str(*name, source)?;
        let file = omocl::model::load::load_str(*name, source, &FirstPartyConverters)
            .map_err(|diagnostics| render(&diagnostics))?;
        set.insert(document, file)
            .map_err(|error| error.to_string())?;
    }
    Ok(set)
}

/// Compiles a set against the laboratory template.
pub(crate) fn compiled(
    set: &MappingSet,
    index: &WebTemplateIndex,
) -> Result<Arc<Program>, Box<dyn Error>> {
    omocl::resolve::compile(set, index, &FirstPartyConverters).map_err(|errors| {
        errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<String>>()
            .join("\n")
            .into()
    })
}

/// The composition every run reads.
pub(crate) fn source() -> Result<Source, Box<dyn Error>> {
    Ok(Source::new(
        EhrId::new("ehr-synthetic-1")?,
        VersionedObjectUid::new("composition-synthetic-1")?,
        VersionUid::new("composition-synthetic-1::ferrobridge.test::1")?,
    ))
}

/// The visit every run's rows belong to.
pub(crate) fn visit() -> Result<VisitKey, Box<dyn Error>> {
    Ok(VisitKey::new(
        EhrId::new("ehr-synthetic-1")?,
        VisitSource::new("visit-synthetic-1")?,
    ))
}

/// Returns one synthetic concept row.
pub(crate) fn concept(
    id: i32,
    domain: &str,
    vocabulary: &str,
    code: &str,
    standard: bool,
) -> Result<Concept, Box<dyn Error>> {
    Ok(Concept {
        concept_id: id,
        concept_name: format!("Synthetic concept {id}").parse()?,
        domain_id: domain.parse()?,
        vocabulary_id: vocabulary.parse()?,
        concept_class_id: "FB-CLASS".parse()?,
        standard_concept: if standard { Some("S".parse()?) } else { None },
        concept_code: code.parse()?,
        valid_start_date: CdmDate::new("1970-01-01")?,
        valid_end_date: CdmDate::new("2099-12-31")?,
        invalid_reason: None,
    })
}

/// What the stub answers for one source code.
#[derive(Debug, Clone)]
pub(crate) enum Stubbed {
    /// A resolution.
    Resolved(Box<Resolution>),
    /// The resolver's ambiguity refusal, with the concepts that carry the key.
    Ambiguous(Vec<i32>),
}

/// Returns a standard answer.
pub(crate) fn standard(concept: Concept) -> Stubbed {
    Stubbed::Resolved(Box::new(Resolution::Standard { concept }))
}

/// Returns a mapped answer.
pub(crate) fn mapped(source: Concept, standard: Vec<Concept>) -> Stubbed {
    Stubbed::Resolved(Box::new(Resolution::Mapped { source, standard }))
}

/// Returns the key of a code.
pub(crate) fn key(vocabulary: &str, code: &str) -> Result<SourceKey, Box<dyn Error>> {
    Ok(SourceKey::new(
        VocabularyId::new(vocabulary)?,
        ConceptCode::new(code)?,
    ))
}

/// A vocabulary answered from maps, counting every question.
#[derive(Debug, Default)]
pub(crate) struct StubSource {
    pub(crate) codes: BTreeMap<(String, String), Stubbed>,
    pub(crate) operators: BTreeMap<Operator, i32>,
    pub(crate) domains: BTreeMap<String, i32>,
    pub(crate) fail_operators: bool,
    pub(crate) asked: Mutex<Vec<String>>,
}

impl StubSource {
    /// Returns the stub the laboratory snapshot runs against.
    pub(crate) fn laboratory() -> Result<Self, Box<dyn Error>> {
        let mut stub = Self::default();
        stub.code(
            "LOINC",
            "SYN-LAB-1",
            standard(concept(1_001, "Measurement", "LOINC", "SYN-LAB-1", true)?),
        );
        stub.code(
            "LOINC",
            "SYN-LAB-2",
            mapped(
                concept(2_001, "Measurement", "LOINC", "SYN-LAB-2", false)?,
                vec![concept(1_002, "Measurement", "FB-STANDARD", "STD-2", true)?],
            ),
        );
        stub.code(
            "UCUM",
            "mmol/L",
            standard(concept(3_001, "Unit", "UCUM", "mmol/L", true)?),
        );
        stub.code(
            "UCUM",
            "g/L",
            Stubbed::Resolved(Box::new(Resolution::Unmapped {
                key: key("UCUM", "g/L")?,
                source: None,
            })),
        );
        stub.code(
            "SNOMED",
            "SYN-SPEC-1",
            standard(concept(4_001, "Specimen", "SNOMED", "SYN-SPEC-1", true)?),
        );
        stub.operators.insert(Operator::Less, 5_001);
        stub.domains.insert("Measurement".to_owned(), 9_011);
        stub.domains.insert("Specimen".to_owned(), 9_016);
        Ok(stub)
    }

    /// Answers `answer` for one source code.
    pub(crate) fn code(&mut self, vocabulary: &str, code: &str, answer: Stubbed) {
        self.codes
            .insert((vocabulary.to_owned(), code.to_owned()), answer);
    }

    /// Returns every question asked, in order.
    pub(crate) fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .map(|asked| asked.clone())
            .unwrap_or_default()
    }

    fn record(&self, question: String) {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push(question);
        }
    }
}

impl ConceptSource for StubSource {
    fn resolve(
        &self,
        key: &SourceKey,
        date: &CdmDate,
    ) -> impl Future<Output = Result<Resolution, ResolveError>> + Send {
        self.record(format!("{key} on {date}"));
        let found = self.codes.get(&(
            key.vocabulary().as_str().to_owned(),
            key.code().as_str().to_owned(),
        ));
        let answer = match found {
            Some(Stubbed::Resolved(resolution)) => Ok(resolution.as_ref().clone()),
            Some(Stubbed::Ambiguous(ids)) => Err(ResolveError::Ambiguous {
                key: Box::new(key.clone()),
                date: date.clone(),
                concept_ids: ids
                    .iter()
                    .copied()
                    .map(omop_cdm::vocabulary::ConceptId::new)
                    .collect(),
            }),
            None => Ok(Resolution::Unmapped {
                key: key.clone(),
                source: None,
            }),
        };
        std::future::ready(answer)
    }

    fn operator(
        &self,
        operator: Operator,
        date: &CdmDate,
    ) -> impl Future<Output = Result<Option<ConceptId>, LookupError>> + Send {
        self.record(format!("operator {operator} on {date}"));
        let answer = if self.fail_operators {
            Err(LookupError {
                what: format!("the operator {operator}"),
                source: "the synthetic vocabulary is down".into(),
            })
        } else {
            Ok(self.operators.get(&operator).copied().map(ConceptId::new))
        };
        std::future::ready(answer)
    }

    fn domain_concept(
        &self,
        domain: &str,
    ) -> impl Future<Output = Result<Option<ConceptId>, LookupError>> + Send {
        self.record(format!("domain {domain}"));
        std::future::ready(Ok(self.domains.get(domain).copied().map(ConceptId::new)))
    }
}

/// Returns the seams over `stub`.
pub(crate) fn seams<'a>(
    stub: &'a StubSource,
    aliases: &'a VocabularyAliases,
) -> Seams<'a, StubSource> {
    Seams {
        concepts: stub,
        type_concept: ConceptId::new(TYPE_CONCEPT),
        aliases,
    }
}
