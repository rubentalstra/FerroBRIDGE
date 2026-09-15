// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The interpreter over a compiled program, one traversal for both
//! directions.
//!
//! The walk is top-down in program order. For each mapping it evaluates the
//! input-side conditions, reads the input occurrences, runs the data-type cell
//! the two classes fix, and writes the result where the recurrence rules put
//! it. A `followedBy` child runs once per occurrence its parent bound, which
//! is what pairs two repeating lists against each other
//! (`recurrence/PopulatingAnEntry.adoc`).
//!
//! Where a mapping writes is decided by one rule, applied on whichever side is
//! the output: the axes the parent bound keep their instance, and every
//! repeating element below them takes a fresh instance per input occurrence,
//! because "FHIRconnect always appends to a `0..n` path if it's not iterated"
//! (`recurrence/PopulatingAnEntry.adoc` §Wrong mapping). An output with no
//! repeating element below the parent holds one value, so a second write
//! overwrites the first (`recurrence/Overwriting.adoc`) and a `0..n` input
//! leaves only its last occurrence, with the rest declared lost
//! (`recurrence/TransformingList.adoc`).

use core::fmt;
use std::collections::BTreeMap;

use fhir_types::codec::Object;
use fhir_types::codec::Value;
use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::composition::NodeValue;
use openehr_mapping_core::composition::PositionError;
use openehr_mapping_core::composition::RmPosition;
use openehr_mapping_core::index::AqlPath;
use openehr_mapping_core::index::FlatId;
use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::template::PathError;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentifiedData;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;

use crate::engine::cell;
use crate::engine::condition;
use crate::engine::condition::ConditionError;
use crate::engine::condition::Verdict;
use crate::engine::fhir::FhirError;
use crate::engine::fhir::FhirKind;
use crate::engine::fhir::FhirValue;
use crate::engine::lens::LensError;
use crate::engine::outcome::Outcome;
use crate::engine::outcome::SkipReason;
use crate::engine::outcome::Warning;
use crate::engine::recurrence::Cardinality;
use crate::engine::recurrence::Placement;
use crate::engine::rm::RmError;
use crate::engine::rm::RmValue;
use crate::model::ast::Direction;
use crate::resolve::program::FhirTarget;
use crate::resolve::program::Manual;
use crate::resolve::program::Mapping;
use crate::resolve::program::Method;
use crate::resolve::program::OpenehrTarget;
use crate::resolve::program::Program;
use crate::resolve::program::Target;
use crate::tree::Occurrence;

use crate::tree::element::Move;
use crate::tree::element::Table;
use crate::tree::error::ReadError;
use crate::tree::error::WriteError;
use crate::tree::read::Selected;
use crate::tree::read::read;
use crate::tree::write::write;

/// How many instances of one repeating node a read probes for.
///
/// The composition carries no instance count of its own, so the read stops at
/// the first absent instance and this is the ceiling on that probe. No
/// specification governs this: our own design.
const INSTANCE_CEILING: u32 = 1024;

/// The registry a `mappingCode` resolves against.
///
/// The specification leaves the functions to the engine
/// (`types-of-mappings/concept-type/concept-mappings.adoc`), so the registry
/// is a seam rather than a fixed list, and it ships empty.
pub trait MappingFunctions: fmt::Debug {
    /// Runs the function `code` names over one openEHR value.
    ///
    /// # Errors
    ///
    /// Returns [`MappingCodeError`] when the registry does not hold `code` or
    /// the function refuses its input.
    fn to_fhir(&self, code: &str, source: Option<&RmValue>) -> Result<FhirValue, MappingCodeError>;

    /// Runs the function `code` names over one FHIR element.
    ///
    /// # Errors
    ///
    /// Returns [`MappingCodeError`] when the registry does not hold `code` or
    /// the function refuses its input.
    fn to_openehr(&self, code: &str, view: Option<&FhirValue>)
    -> Result<RmValue, MappingCodeError>;
}

/// The registry this milestone ships.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoMappingFunctions;

impl MappingFunctions for NoMappingFunctions {
    fn to_fhir(
        &self,
        code: &str,
        _source: Option<&RmValue>,
    ) -> Result<FhirValue, MappingCodeError> {
        Err(MappingCodeError::Unknown {
            code: String::from(code),
        })
    }

    fn to_openehr(
        &self,
        code: &str,
        _view: Option<&FhirValue>,
    ) -> Result<RmValue, MappingCodeError> {
        Err(MappingCodeError::Unknown {
            code: String::from(code),
        })
    }
}

/// Why a registered function did not produce a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MappingCodeError {
    /// The registry holds no function of that name.
    #[error("no registered function is named {code}")]
    Unknown {
        /// The name the mapping wrote.
        code: String,
    },
    /// The function refused its input.
    #[error("the function {code} refused its input: {reason}")]
    Refused {
        /// The name the mapping wrote.
        code: String,
        /// What the function refused.
        reason: String,
    },
}

/// The composition fields the engine fills when no mapping does.
///
/// `engine/defaults-for-fields.adoc` puts the composer and the context start
/// time on the engine and the rest on "the project performing the mapping", so
/// only those two carry a value here unless the caller sets more. Every field
/// the engine fills is recorded as [`Warning::Defaulted`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    composer: String,
    start_time: String,
    language: Option<String>,
    territory: Option<String>,
}

impl Defaults {
    /// Creates the defaults, with `now` as the context start time.
    ///
    /// "For the `context_start_time`, the default mapping time can be used",
    /// and the composer is defaulted "with a value such as `FHIRconnect`
    /// party" (`engine/defaults-for-fields.adoc`). This crate reads no clock,
    /// so the caller passes the time.
    #[must_use]
    pub fn at(now: impl Into<String>) -> Self {
        Self {
            composer: String::from("FHIRconnect"),
            start_time: now.into(),
            language: None,
            territory: None,
        }
    }

    /// Returns these defaults with `name` as the composer.
    #[must_use]
    pub fn with_composer(mut self, name: impl Into<String>) -> Self {
        self.composer = name.into();
        self
    }

    /// Returns these defaults with the composition language `code`.
    ///
    /// The code set is ISO 639-1
    /// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/ehr.html#_composition_class>).
    #[must_use]
    pub fn with_language(mut self, code: impl Into<String>) -> Self {
        self.language = Some(code.into());
        self
    }

    /// Returns these defaults with the composition territory `code`.
    ///
    /// The code set is ISO 3166-1
    /// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/ehr.html#_composition_class>).
    #[must_use]
    pub fn with_territory(mut self, code: impl Into<String>) -> Self {
        self.territory = Some(code.into());
        self
    }
}

/// Why a run refused.
///
/// Every variant names the mapping it refused in, so a diagnostic points at
/// the file the mapping author edits.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The program's own condition does not admit the input.
    #[error("the context {context} does not apply to this input")]
    NotApplicable {
        /// The context mapping the program compiled from.
        context: String,
    },
    /// A condition could not be decided.
    #[error("the condition of {mapping} could not be decided")]
    Condition {
        /// The mapping the condition belongs to.
        mapping: String,
        /// The refusal the evaluator returned.
        #[source]
        source: Box<ConditionError>,
    },
    /// The input side could not be read.
    #[error("{mapping} could not read {expression}")]
    Read {
        /// The mapping being run.
        mapping: String,
        /// The expression that refused.
        expression: String,
        /// The refusal the path model returned.
        #[source]
        source: Box<ReadError>,
    },
    /// The output side could not be written.
    #[error("{mapping} could not write {element}")]
    Write {
        /// The mapping being run.
        mapping: String,
        /// The element that refused.
        element: String,
        /// The refusal the path model returned.
        #[source]
        source: Box<WriteError>,
    },
    /// The data-type cell refused the value.
    #[error("{mapping} could not convert {element}")]
    Cell {
        /// The mapping being run.
        mapping: String,
        /// The element the cell refused.
        element: String,
        /// The refusal the cell returned.
        #[source]
        source: Box<LensError>,
    },
    /// An openEHR value could not be read or written.
    #[error("{mapping} could not carry the openEHR value at {node}")]
    Rm {
        /// The mapping being run.
        mapping: String,
        /// The node the value belongs to.
        node: String,
        /// The refusal the value model returned.
        #[source]
        source: Box<RmError>,
    },
    /// A FHIR element could not be read or written.
    #[error("{mapping} could not carry the FHIR element {element}")]
    Fhir {
        /// The mapping being run.
        mapping: String,
        /// The element the value belongs to.
        element: String,
        /// The refusal the element model returned.
        #[source]
        source: Box<FhirError>,
    },
    /// The template could not answer for a node.
    #[error("{mapping} could not reach {node} in the template")]
    Template {
        /// The mapping being run.
        mapping: String,
        /// The node the mapping names.
        node: String,
        /// The refusal the index returned.
        #[source]
        source: Box<PathError>,
    },
    /// An instance index has no openEHR position.
    #[error("{mapping} produced an instance index with no openEHR position")]
    Position {
        /// The mapping being run.
        mapping: String,
        /// The refusal the position model returned.
        #[source]
        source: PositionError,
    },
    /// Neither the mapping nor the element table says which element to write.
    #[error("{mapping} names no FHIR data type for {element}, and the table fixes none")]
    UnknownElement {
        /// The mapping being run.
        mapping: String,
        /// The element the mapping writes.
        element: String,
    },
    /// The mapping names an openEHR path below the template's deepest node.
    #[error("{mapping} names {tail} below {node}, whose class the program does not carry")]
    UnresolvedTail {
        /// The mapping being run.
        mapping: String,
        /// The node the path resolved to.
        node: String,
        /// The reference-model attributes below it.
        tail: String,
    },
    /// A required child produced no value.
    ///
    /// "The mapping should fail if the child is not provided"
    /// (`engine/Fail.adoc`).
    #[error("{mapping} produced no value for {node}, which the template requires")]
    MissingRequired {
        /// The mapping that produced nothing.
        mapping: String,
        /// The node the template requires.
        node: String,
    },
    /// A slot chain reaches a model mapping it already entered.
    #[error("the slot chain {chain} reaches {model} a second time")]
    SlotCycle {
        /// The chain as the traversal entered it.
        chain: String,
        /// The model the chain reaches again.
        model: String,
    },
    /// A registered function did not produce a value.
    #[error("{mapping} could not run the function {code}")]
    MappingCode {
        /// The mapping being run.
        mapping: String,
        /// The function the mapping names.
        code: String,
        /// The refusal the registry returned.
        #[source]
        source: Box<MappingCodeError>,
    },
    /// The mapping names a method this engine does not run.
    #[error("{mapping} uses {method}, which this engine does not run")]
    Unsupported {
        /// The mapping being run.
        mapping: String,
        /// The method the mapping names.
        method: &'static str,
    },
    /// A reference the path model left for the engine to resolve.
    #[error("{mapping} reads through a reference at {expression}, which the engine resolves")]
    DeferredReference {
        /// The mapping being run.
        mapping: String,
        /// The expression that reached the reference.
        expression: String,
    },
    /// The composition did not build.
    #[error("the values the run produced do not build a composition")]
    Build {
        /// The refusal the builder returned.
        #[source]
        source: Box<PathError>,
    },
}

/// The occurrence a mapping is bound to, on both sides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Binding {
    fhir: Occurrence,
    openehr: Vec<RmPosition>,
}

/// One openEHR value the run produced.
#[derive(Debug, Clone, PartialEq)]
struct Written {
    flat_id: FlatId,
    positions: Vec<RmPosition>,
    value: RmValue,
}

/// Runs `program` over a FHIR document, producing a composition.
///
/// # Errors
///
/// Returns [`EngineError`] for any element the program cannot map, and for a
/// set of values the template does not admit as a composition.
pub fn to_openehr<T: Table + ?Sized>(
    program: &Program,
    table: &T,
    index: &WebTemplateIndex,
    document: &Value,
    functions: &dyn MappingFunctions,
    defaults: &Defaults,
) -> Result<Outcome<CanonicalComposition>, EngineError> {
    let mut run = Run {
        program,
        table,
        index,
        direction: Direction::FhirToOpenehr,
        functions,
        warnings: Vec::new(),
        fhir: document.clone(),
        composition: None,
        written: Vec::new(),
        counters: BTreeMap::new(),
        chain: Vec::new(),
    };
    run.admits_context()?;
    run.mappings(program.mappings(), &Binding::default())?;
    run.apply_defaults(defaults);
    let values = run.node_values()?;
    let built = index
        .build_composition(&values, &defaults.start_time)
        .map_err(|source| EngineError::Build {
            source: Box::new(source),
        })?;
    Ok(Outcome::new(built, run.warnings))
}

/// Runs `program` over a composition, producing a FHIR resource.
///
/// # Errors
///
/// Returns [`EngineError`] for any element the program cannot map.
pub fn to_fhir<T: Table + ?Sized>(
    program: &Program,
    table: &T,
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
    functions: &dyn MappingFunctions,
) -> Result<Outcome<Value>, EngineError> {
    let mut resource = Object::new();
    resource.insert(
        String::from("resourceType"),
        Value::String(String::from(program.resource().as_str())),
    );
    let mut run = Run {
        program,
        table,
        index,
        direction: Direction::OpenehrToFhir,
        functions,
        warnings: Vec::new(),
        fhir: Value::Object(resource),
        composition: Some(composition),
        written: Vec::new(),
        counters: BTreeMap::new(),
        chain: Vec::new(),
    };
    run.mappings(program.mappings(), &Binding::default())?;
    Ok(Outcome::new(run.fhir, run.warnings))
}

/// One run of one program, in one direction.
struct Run<'a, T: Table + ?Sized> {
    program: &'a Program,
    table: &'a T,
    index: &'a WebTemplateIndex,
    direction: Direction,
    functions: &'a dyn MappingFunctions,
    warnings: Vec<Warning>,
    fhir: Value,
    composition: Option<&'a CanonicalComposition>,
    written: Vec<Written>,
    counters: BTreeMap<(String, String), usize>,
    chain: Vec<String>,
}

impl<T: Table + ?Sized> Run<'_, T> {
    /// Refuses an input the program's own condition does not admit.
    fn admits_context(&self) -> Result<(), EngineError> {
        let Some(gate) = self.program.fhir_condition() else {
            return Ok(());
        };
        if !condition::runs(gate, self.direction) {
            return Ok(());
        }
        let verdict =
            condition::evaluate(self.table, &self.fhir, gate, false).map_err(|source| {
                EngineError::Condition {
                    mapping: String::from(self.program.context().as_str()),
                    source: Box::new(source),
                }
            })?;
        if verdict.admits_any() {
            return Ok(());
        }
        Err(EngineError::NotApplicable {
            context: String::from(self.program.context().as_str()),
        })
    }

    /// Runs a list of mappings under one binding, in program order.
    fn mappings(&mut self, mappings: &[Mapping], parent: &Binding) -> Result<(), EngineError> {
        for mapping in mappings {
            self.mapping(mapping, parent)?;
        }
        Ok(())
    }

    /// Runs one mapping under one binding.
    fn mapping(&mut self, mapping: &Mapping, parent: &Binding) -> Result<(), EngineError> {
        if let Some(only) = mapping.direction()
            && only != self.direction
        {
            self.warnings.push(Warning::Skipped {
                mapping: String::from(mapping.name()),
                reason: SkipReason::Unidirectional,
            });
            return Ok(());
        }
        let Some(admitted) = self.admits(mapping, parent)? else {
            return Ok(());
        };
        let bindings = self.apply(mapping, parent, &admitted)?;
        for binding in &bindings {
            self.children(mapping, binding)?;
        }
        Ok(())
    }

    /// Returns the input occurrences the conditions admit, `None` for a
    /// mapping a gate closed.
    fn admits(
        &self,
        mapping: &Mapping,
        parent: &Binding,
    ) -> Result<Option<Vec<Occurrence>>, EngineError> {
        let inputs = self.inputs(mapping, parent)?;
        let Some(gate) = self.input_condition(mapping) else {
            return Ok(Some(inputs));
        };
        match self.direction {
            Direction::FhirToOpenehr => {
                let attached = mapping
                    .fhir()
                    .is_some_and(|input| condition::attached(gate, input.expression()));
                let verdict = condition::evaluate(self.table, &self.fhir, gate, attached).map_err(
                    |source| EngineError::Condition {
                        mapping: String::from(mapping.name()),
                        source: Box::new(source),
                    },
                )?;
                match verdict {
                    Verdict::Gate(false) => Ok(None),
                    Verdict::Gate(true) => Ok(Some(inputs)),
                    Verdict::Filter(admitted) => Ok(Some(
                        inputs
                            .into_iter()
                            .filter(|occurrence| admitted.contains(occurrence))
                            .collect(),
                    )),
                }
            }
            // NOTE: Conditions.adoc, an openehrCondition is evaluated over the
            // openEHR input, which this milestone reads only through the node
            // the mapping names, so it gates rather than filters.
            Direction::OpenehrToFhir => Ok(Some(inputs)),
        }
    }

    /// Returns the condition the direction evaluates, if the mapping carries
    /// one.
    fn input_condition<'mapping>(
        &self,
        mapping: &'mapping Mapping,
    ) -> Option<&'mapping crate::resolve::program::Condition> {
        match self.direction {
            Direction::FhirToOpenehr => mapping.fhir_condition(),
            Direction::OpenehrToFhir => mapping.openehr_condition(),
        }
        .filter(|gate| condition::runs(gate, self.direction))
    }

    /// Returns the occurrences of the input side, under the parent binding.
    fn inputs(&self, mapping: &Mapping, parent: &Binding) -> Result<Vec<Occurrence>, EngineError> {
        match self.direction {
            Direction::FhirToOpenehr => {
                let Some(input) = mapping.fhir() else {
                    return Ok(vec![parent.fhir.clone()]);
                };
                let matches =
                    read(self.table, &self.fhir, input.expression()).map_err(|source| {
                        EngineError::Read {
                            mapping: String::from(mapping.name()),
                            expression: String::from(input.expression().as_str()),
                            source: Box::new(source),
                        }
                    })?;
                let mut occurrences = Vec::new();
                for matched in matches {
                    if !matched
                        .occurrence()
                        .indices()
                        .starts_with(parent.fhir.indices())
                    {
                        continue;
                    }
                    if let Selected::Deferred(_) = *matched.selected() {
                        return Err(EngineError::DeferredReference {
                            mapping: String::from(mapping.name()),
                            expression: String::from(input.expression().as_str()),
                        });
                    }
                    occurrences.push(matched.occurrence().clone());
                }
                Ok(occurrences)
            }
            Direction::OpenehrToFhir => {
                let Some(input) = mapping.openehr() else {
                    return Ok(vec![parent.fhir.clone()]);
                };
                let instances = self.instances(mapping, input, parent)?;
                let mut occurrences = Vec::with_capacity(instances.len());
                for positions in instances {
                    let mut indices = Vec::with_capacity(positions.len());
                    for position in positions {
                        indices.push(usize::try_from(position.get()).map_err(|_refused| {
                            EngineError::Position {
                                mapping: String::from(mapping.name()),
                                source: PositionError::Overflow,
                            }
                        })?);
                    }
                    occurrences.push(Occurrence::new(indices));
                }
                Ok(occurrences)
            }
        }
    }

    /// Returns the instances of the input node under the parent binding.
    ///
    /// The composition carries no instance count, so the probe stops at the
    /// first instance the read does not find.
    fn instances(
        &self,
        mapping: &Mapping,
        input: &OpenehrTarget,
        parent: &Binding,
    ) -> Result<Vec<Vec<RmPosition>>, EngineError> {
        let depth = input.occurrences().len();
        let bound = parent.openehr.len().min(depth);
        let prefix: Vec<RmPosition> = parent.openehr.iter().take(bound).copied().collect();
        if prefix.len() == depth {
            return Ok(vec![prefix]);
        }
        let mut found = Vec::new();
        let mut instance = 1u32;
        while instance <= INSTANCE_CEILING {
            let mut positions = prefix.clone();
            positions.push(
                RmPosition::new(instance).map_err(|source| EngineError::Position {
                    mapping: String::from(mapping.name()),
                    source,
                })?,
            );
            positions.resize(depth, RmPosition::first());
            if self.value_at(mapping, input, &positions)?.is_none() {
                break;
            }
            found.push(positions);
            instance = instance.saturating_add(1);
        }
        Ok(found)
    }

    /// Returns the openEHR value one instance of the input node holds.
    fn value_at(
        &self,
        mapping: &Mapping,
        input: &OpenehrTarget,
        positions: &[RmPosition],
    ) -> Result<Option<RmValue>, EngineError> {
        let Some(composition) = self.composition else {
            return Ok(None);
        };
        let node = input.node();
        let read = self
            .index
            .read(composition, node, positions)
            .map_err(|source| EngineError::Template {
                mapping: String::from(mapping.name()),
                node: String::from(node.aql_path().as_str()),
                source: Box::new(source),
            })?;
        let Some(value) = read else {
            return Ok(None);
        };
        if !input.tail().segments.is_empty() {
            // TODO(#177): read the class the compiler validated for the tail.
            return Err(EngineError::UnresolvedTail {
                mapping: String::from(mapping.name()),
                node: String::from(node.aql_path().as_str()),
                tail: input.tail().to_string(),
            });
        }
        RmValue::from_canonical(node.rm_type(), node.aql_path().as_str(), &value)
            .map(Some)
            .map_err(|source| EngineError::Rm {
                mapping: String::from(mapping.name()),
                node: String::from(node.aql_path().as_str()),
                source: Box::new(source),
            })
    }

    /// Applies one mapping's method to every admitted input occurrence.
    ///
    /// Returns the binding each occurrence produced, which is what a
    /// `followedBy` child runs under.
    fn apply(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
    ) -> Result<Vec<Binding>, EngineError> {
        match *mapping.method() {
            Method::Value => self.values(mapping, parent, inputs),
            Method::Slot {
                ref model,
                ref mappings,
            } => {
                let name = String::from(model.as_str());
                if self.chain.contains(&name) {
                    return Err(EngineError::SlotCycle {
                        chain: self.chain.join(" -> "),
                        model: name,
                    });
                }
                let bindings = self.bindings(mapping, parent, inputs)?;
                self.chain.push(name);
                for binding in &bindings {
                    self.mappings(mappings, binding)?;
                }
                self.chain.pop();
                Ok(bindings)
            }
            Method::Programmed { ref code } => self.programmed(mapping, parent, inputs, code),
            Method::Reference { .. } => Err(EngineError::Unsupported {
                mapping: String::from(mapping.name()),
                method: "reference",
            }),
            Method::Link { .. } => Err(EngineError::Unsupported {
                mapping: String::from(mapping.name()),
                method: "link",
            }),
            Method::Participation { .. } => Err(EngineError::Unsupported {
                mapping: String::from(mapping.name()),
                method: "participationsFunction",
            }),
        }
    }

    /// Runs the data-type cell of a plain `with` mapping, or only binds.
    fn values(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
    ) -> Result<Vec<Binding>, EngineError> {
        let bindings = self.bindings(mapping, parent, inputs)?;
        if !mapping.manual().is_empty() {
            for entry in mapping.manual() {
                for binding in &bindings {
                    self.manual(mapping, entry, binding)?;
                }
            }
            return Ok(bindings);
        }
        // NOTE: data-mappings.adoc, `type: NONE` "does not transform anything"
        // and only anchors the mappings below it.
        if mapping.data_type() == Some(crate::model::ast::DataType::None) {
            return Ok(bindings);
        }
        let (Some(_fhir), Some(_openehr)) = (mapping.fhir(), mapping.openehr()) else {
            return Ok(bindings);
        };
        for (input, binding) in inputs.iter().zip(bindings.iter()) {
            self.convert(mapping, input, binding)?;
        }
        Ok(bindings)
    }

    /// Runs the registered function a `mappingCode` names.
    fn programmed(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
        code: &str,
    ) -> Result<Vec<Binding>, EngineError> {
        let bindings = self.bindings(mapping, parent, inputs)?;
        let refuse = |source: MappingCodeError| EngineError::MappingCode {
            mapping: String::from(mapping.name()),
            code: String::from(code),
            source: Box::new(source),
        };
        for binding in &bindings {
            match self.direction {
                Direction::FhirToOpenehr => {
                    let produced = self.functions.to_openehr(code, None).map_err(refuse)?;
                    self.put_openehr(mapping, binding, produced);
                }
                Direction::OpenehrToFhir => {
                    let produced = self.functions.to_fhir(code, None).map_err(refuse)?;
                    self.put_fhir(mapping, binding, &produced)?;
                }
            }
        }
        Ok(bindings)
    }

    /// Runs one occurrence of a mapping through its data-type cell.
    fn convert(
        &mut self,
        mapping: &Mapping,
        input: &Occurrence,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        match self.direction {
            Direction::FhirToOpenehr => {
                let Some(target) = mapping.fhir() else {
                    return Ok(());
                };
                let Some(openehr) = mapping.openehr() else {
                    return Ok(());
                };
                let kind = Self::kind(mapping, target)?;
                let value = self.element_at(mapping, target, input, kind)?;
                let Some(view) = value else {
                    return Ok(());
                };
                let node = openehr.node();
                if !openehr.tail().segments.is_empty() {
                    // TODO(#177): read the class the compiler validated for the tail.
                    return Err(EngineError::UnresolvedTail {
                        mapping: String::from(mapping.name()),
                        node: String::from(node.aql_path().as_str()),
                        tail: openehr.tail().to_string(),
                    });
                }
                let held = self.held(&binding.openehr, node.flat_id());
                let produced = cell::put(
                    &view,
                    node.rm_type(),
                    held.as_ref(),
                    self.binding_of(node).as_deref(),
                )
                .map_err(|source| EngineError::Cell {
                    mapping: String::from(mapping.name()),
                    element: String::from(target.expression().as_str()),
                    source: Box::new(source),
                })?;
                for taken in &produced.fallbacks {
                    self.warnings.push(Warning::fallback(taken));
                }
                {
                    self.put_openehr(mapping, binding, produced.value);
                    Ok(())
                }
            }
            Direction::OpenehrToFhir => {
                let Some(target) = mapping.fhir() else {
                    return Ok(());
                };
                let Some(openehr) = mapping.openehr() else {
                    return Ok(());
                };
                let positions = Self::positions_of(mapping, input, openehr)?;
                let Some(source) = self.value_at(mapping, openehr, &positions)? else {
                    return Ok(());
                };
                let kind = Self::kind(mapping, target)?;
                let view = cell::get(&source, kind, self.binding_of(openehr.node()).as_deref())
                    .map_err(|source| EngineError::Cell {
                        mapping: String::from(mapping.name()),
                        element: String::from(target.expression().as_str()),
                        source: Box::new(source),
                    })?;
                self.put_fhir(mapping, binding, &view)
            }
        }
    }

    /// Returns the positions one input occurrence names for an openEHR node.
    fn positions_of(
        mapping: &Mapping,
        input: &Occurrence,
        target: &OpenehrTarget,
    ) -> Result<Vec<RmPosition>, EngineError> {
        let depth = target.occurrences().len();
        let mut positions = Vec::with_capacity(depth);
        for index in input.indices().iter().take(depth) {
            let position = u32::try_from(*index)
                .ok()
                .and_then(|index| RmPosition::new(index).ok())
                .ok_or_else(|| EngineError::Position {
                    mapping: String::from(mapping.name()),
                    source: PositionError::Zero,
                })?;
            positions.push(position);
        }
        positions.resize(depth, RmPosition::first());
        Ok(positions)
    }

    /// Returns which FHIR element a mapping writes.
    fn kind(mapping: &Mapping, target: &FhirTarget) -> Result<FhirKind, EngineError> {
        mapping
            .data_type()
            .and_then(FhirKind::of)
            .or_else(|| FhirKind::at(target.resolved().location()))
            .ok_or_else(|| EngineError::UnknownElement {
                mapping: String::from(mapping.name()),
                element: String::from(target.resolved().leaf()),
            })
    }

    /// Reads the FHIR element one input occurrence names.
    fn element_at(
        &self,
        mapping: &Mapping,
        target: &FhirTarget,
        occurrence: &Occurrence,
        kind: FhirKind,
    ) -> Result<Option<FhirValue>, EngineError> {
        let matches = read(self.table, &self.fhir, target.expression()).map_err(|source| {
            EngineError::Read {
                mapping: String::from(mapping.name()),
                expression: String::from(target.expression().as_str()),
                source: Box::new(source),
            }
        })?;
        let Some(found) = matches
            .iter()
            .find(|matched| matched.occurrence() == occurrence)
            .and_then(crate::tree::read::Match::value)
        else {
            return Ok(None);
        };
        FhirValue::read(kind, target.resolved().leaf(), found)
            .map(Some)
            .map_err(|source| EngineError::Fhir {
                mapping: String::from(mapping.name()),
                element: String::from(target.resolved().leaf()),
                source: Box::new(source),
            })
    }

    /// Returns the terminology the template binds an openEHR node to.
    fn binding_of(&self, node: &ResolvedNode) -> Option<String> {
        self.index
            .bindings(node)
            .ok()
            .and_then(<[openehr_mapping_core::template::Binding]>::first)
            .map(|binding| String::from(binding.system()))
    }

    /// Returns the openEHR value the run already wrote at one place.
    fn held(&self, positions: &[RmPosition], flat_id: &FlatId) -> Option<RmValue> {
        self.written
            .iter()
            .find(|written| written.flat_id == *flat_id && written.positions == positions)
            .map(|written| written.value.clone())
    }

    /// Writes one openEHR value, overwriting what the same place holds.
    fn put_openehr(&mut self, mapping: &Mapping, binding: &Binding, value: RmValue) {
        let Some(target) = mapping.openehr() else {
            return;
        };
        let flat_id = target.node().flat_id().clone();
        let positions = binding.openehr.clone();
        let written = Written {
            flat_id,
            positions,
            value,
        };
        if let Some(slot) = self
            .written
            .iter_mut()
            .find(|held| held.flat_id == written.flat_id && held.positions == written.positions)
        {
            *slot = written;
        } else {
            self.written.push(written);
        }
    }

    /// Writes one FHIR element at the occurrence its binding names.
    fn put_fhir(
        &mut self,
        mapping: &Mapping,
        binding: &Binding,
        view: &FhirValue,
    ) -> Result<(), EngineError> {
        let Some(target) = mapping.fhir() else {
            return Ok(());
        };
        let element = String::from(target.resolved().leaf());
        let value = view.write(&element).map_err(|source| EngineError::Fhir {
            mapping: String::from(mapping.name()),
            element: element.clone(),
            source: Box::new(source),
        })?;
        write(
            self.table,
            &mut self.fhir,
            target.expression(),
            &binding.fhir,
            value,
        )
        .map_err(|source| EngineError::Write {
            mapping: String::from(mapping.name()),
            element,
            source: Box::new(source),
        })
    }

    /// Writes one manual entry, merging every path it names into one element.
    fn manual(
        &mut self,
        mapping: &Mapping,
        entry: &Manual,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        if let Some(only) = entry.direction()
            && only != self.direction
        {
            self.warnings.push(Warning::Skipped {
                mapping: format!("{}.{}", mapping.name(), entry.name()),
                reason: SkipReason::Unidirectional,
            });
            return Ok(());
        }
        if !self.manual_admits(mapping, entry)? {
            return Ok(());
        }
        match self.direction {
            Direction::FhirToOpenehr => self.manual_openehr(mapping, entry, binding),
            Direction::OpenehrToFhir => self.manual_fhir(mapping, entry, binding),
        }
    }

    /// Returns whether the input-side conditions admit a manual entry.
    fn manual_admits(&self, mapping: &Mapping, entry: &Manual) -> Result<bool, EngineError> {
        let gate = match self.direction {
            Direction::FhirToOpenehr => entry.fhir_condition(),
            Direction::OpenehrToFhir => entry.openehr_condition(),
        };
        let Some(gate) = gate.filter(|gate| condition::runs(gate, self.direction)) else {
            return Ok(true);
        };
        match self.direction {
            Direction::FhirToOpenehr => condition::evaluate(self.table, &self.fhir, gate, false)
                .map(|verdict| verdict.admits_any())
                .map_err(|source| EngineError::Condition {
                    mapping: String::from(mapping.name()),
                    source: Box::new(source),
                }),
            // NOTE: Conditions.adoc, an openehrCondition reads the openEHR
            // input, which this milestone reaches only through the mapping's
            // own node, so a manual entry's gate answers true.
            Direction::OpenehrToFhir => Ok(true),
        }
    }

    /// Writes the openEHR paths of one manual entry as one value.
    fn manual_openehr(
        &mut self,
        mapping: &Mapping,
        entry: &Manual,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        let mut merged = serde_json::Map::new();
        let mut node: Option<&OpenehrTarget> = None;
        for path in entry.openehr() {
            let Target::Openehr(ref target) = *path.target() else {
                continue;
            };
            node = Some(target.as_ref());
            let segments: Vec<&str> = target
                .tail()
                .segments
                .iter()
                .map(|segment| segment.attribute.as_str())
                .collect();
            merge(&mut merged, &segments, path.value());
        }
        let Some(target) = node else {
            return Ok(());
        };
        let rm_type = target.node().rm_type();
        merged.insert(
            String::from("_type"),
            serde_json::Value::String(String::from(rm_type)),
        );
        let value = RmValue::from_canonical(
            rm_type,
            target.node().aql_path().as_str(),
            &serde_json::Value::Object(merged),
        )
        .map_err(|source| EngineError::Rm {
            mapping: format!("{}.{}", mapping.name(), entry.name()),
            node: String::from(target.node().aql_path().as_str()),
            source: Box::new(source),
        })?;
        let written = Written {
            flat_id: target.node().flat_id().clone(),
            positions: binding.openehr.clone(),
            value,
        };
        if let Some(slot) = self
            .written
            .iter_mut()
            .find(|held| held.flat_id == written.flat_id && held.positions == written.positions)
        {
            *slot = written;
        } else {
            self.written.push(written);
        }
        Ok(())
    }

    /// Writes the FHIR paths of one manual entry into one element.
    fn manual_fhir(
        &mut self,
        mapping: &Mapping,
        entry: &Manual,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        let mut shared: BTreeMap<String, usize> = BTreeMap::new();
        for path in entry.fhir() {
            let Target::Fhir(ref target) = *path.target() else {
                continue;
            };
            let occurrence = self.entry_occurrence(target, binding, &mut shared);
            let element = String::from(target.resolved().leaf());
            write(
                self.table,
                &mut self.fhir,
                target.expression(),
                &occurrence,
                Value::String(String::from(path.value())),
            )
            .map_err(|source| EngineError::Write {
                mapping: format!("{}.{}", mapping.name(), entry.name()),
                element,
                source: Box::new(source),
            })?;
        }
        Ok(())
    }

    /// Returns the occurrence one path of a manual entry writes at.
    ///
    /// "Full FHIR or openEHR elements are created from all given paths"
    /// (`types-of-mappings/concept-type/manual.adoc`), so every path of one
    /// entry that passes the same repeating element takes the same instance
    /// of it.
    fn entry_occurrence(
        &mut self,
        target: &FhirTarget,
        binding: &Binding,
        shared: &mut BTreeMap<String, usize>,
    ) -> Occurrence {
        let axes = fhir_axes(target);
        let mut indices: Vec<usize> = binding.fhir.indices().to_vec();
        for axis in axes.iter().skip(indices.len()) {
            let index = if let Some(&taken) = shared.get(axis) {
                taken
            } else {
                let taken = self.next_index(axis, &indices);
                shared.insert(axis.clone(), taken);
                taken
            };
            indices.push(index);
        }
        Occurrence::new(indices)
    }

    /// Returns the next free instance of one axis under one prefix.
    fn next_index(&mut self, axis: &str, prefix: &[usize]) -> usize {
        let key = (String::from(axis), render(prefix));
        let next = self.counters.entry(key).or_insert(0);
        let taken = *next;
        *next = next.saturating_add(1);
        taken
    }

    /// Returns the binding each input occurrence writes under.
    fn bindings(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
    ) -> Result<Vec<Binding>, EngineError> {
        let axes = self.output_axes(mapping);
        let cardinality = Cardinality::of(axes.len() > parent_depth(parent, self.direction));
        let placement = Placement::decide(inputs.len(), cardinality, 0);
        if placement.lossy() {
            self.warnings.push(Warning::LastOfMany {
                path: self.output_name(mapping),
                dropped: placement.dropped(),
            });
        }
        let mut bindings = Vec::with_capacity(inputs.len());
        for (position, input) in inputs.iter().enumerate() {
            let taken = placement.slots().get(position).copied().flatten();
            bindings.push(self.bind(mapping, parent, input, &axes, taken)?);
        }
        Ok(bindings)
    }

    /// Returns one binding, appending at every axis the parent did not bind.
    fn bind(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        input: &Occurrence,
        axes: &[String],
        taken: Option<usize>,
    ) -> Result<Binding, EngineError> {
        let bound = parent_depth(parent, self.direction);
        let mut indices: Vec<usize> = match self.direction {
            Direction::FhirToOpenehr => parent.openehr.iter().map(|_axis| 0).collect(),
            Direction::OpenehrToFhir => parent.fhir.indices().to_vec(),
        };
        indices.truncate(bound);
        if taken.is_some() {
            for (depth, axis) in axes.iter().enumerate().skip(bound) {
                let index = if depth == bound {
                    let key = (axis.clone(), render(&indices));
                    let next = self.counters.entry(key).or_insert(0);
                    let taken = *next;
                    *next = next.saturating_add(1);
                    taken
                } else {
                    0
                };
                indices.push(index);
            }
        }
        match self.direction {
            Direction::FhirToOpenehr => {
                let mut positions = Vec::with_capacity(indices.len());
                for index in &indices {
                    let index =
                        u32::try_from(*index).map_err(|_refused| EngineError::Position {
                            mapping: String::from(mapping.name()),
                            source: PositionError::Overflow,
                        })?;
                    positions.push(
                        RmPosition::try_from(openehr_mapping_core::composition::FlatIndex::new(
                            index,
                        ))
                        .map_err(|source| EngineError::Position {
                            mapping: String::from(mapping.name()),
                            source,
                        })?,
                    );
                }
                Ok(Binding {
                    fhir: input.clone(),
                    openehr: positions,
                })
            }
            Direction::OpenehrToFhir => Ok(Binding {
                fhir: Occurrence::new(indices),
                openehr: input
                    .indices()
                    .iter()
                    .filter_map(|index| u32::try_from(*index).ok())
                    .filter_map(|index| RmPosition::new(index).ok())
                    .collect(),
            }),
        }
    }

    /// Returns the repeating elements of the output side, outermost first.
    fn output_axes(&self, mapping: &Mapping) -> Vec<String> {
        match self.direction {
            Direction::FhirToOpenehr => mapping.openehr().map_or_else(Vec::new, |target| {
                target
                    .occurrences()
                    .iter()
                    .map(|axis| String::from(axis.as_str()))
                    .collect()
            }),
            Direction::OpenehrToFhir => mapping.fhir().map_or_else(Vec::new, fhir_axes),
        }
    }

    /// Returns the name of the output side, for a declared loss.
    fn output_name(&self, mapping: &Mapping) -> String {
        match self.direction {
            Direction::FhirToOpenehr => mapping
                .openehr()
                .map(|target| String::from(target.node().aql_path().as_str()))
                .unwrap_or_default(),
            Direction::OpenehrToFhir => mapping
                .fhir()
                .map(|target| String::from(target.resolved().leaf()))
                .unwrap_or_default(),
        }
    }

    /// Runs the `followedBy` children of one mapping under its binding.
    fn children(&mut self, mapping: &Mapping, binding: &Binding) -> Result<(), EngineError> {
        for child in mapping.followed_by() {
            let before = self.written.len();
            self.mapping(child, binding)?;
            if self.direction == Direction::FhirToOpenehr
                && self.written.len() == before
                && let Some(target) = child.openehr()
                && target.node().min().is_some_and(|min| min >= 1)
                && child.direction().is_none_or(|only| only == self.direction)
            {
                return Err(EngineError::MissingRequired {
                    mapping: String::from(child.name()),
                    node: String::from(target.node().aql_path().as_str()),
                });
            }
        }
        Ok(())
    }

    /// Fills the composition fields no mapping wrote.
    fn apply_defaults(&mut self, defaults: &Defaults) {
        let composer = defaults.composer.clone();
        self.default_at("/composer", || {
            RmValue::Party(PartyIdentifiedData {
                external_ref: None,
                name: Some(composer),
                identifiers: None,
            })
        });
        let start_time = defaults.start_time.clone();
        self.default_at("/context/start_time", || {
            RmValue::DateTime(Box::new(DvDateTime {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: None,
                magnitude_status: None,
                accuracy: None,
                value: start_time,
            }))
        });
        if let Some(code) = defaults.language.clone() {
            self.default_at("/language", || coded(&code, "ISO_639-1"));
        }
        if let Some(code) = defaults.territory.clone() {
            self.default_at("/territory", || coded(&code, "ISO_3166-1"));
        }
    }

    /// Writes one default value at `path`, when nothing wrote it already.
    fn default_at(&mut self, path: &str, value: impl FnOnce() -> RmValue) {
        let Ok(node) = self.index.node(&AqlPath::new(path)) else {
            return;
        };
        if self
            .written
            .iter()
            .any(|written| written.flat_id == *node.flat_id())
        {
            return;
        }
        self.written.push(Written {
            flat_id: node.flat_id().clone(),
            positions: Vec::new(),
            value: value(),
        });
        self.warnings.push(Warning::Defaulted {
            field: String::from(path),
        });
    }

    /// Returns the values the run produced, as the composition builder wants
    /// them.
    fn node_values(&self) -> Result<Vec<NodeValue>, EngineError> {
        let mut values = Vec::new();
        for written in &self.written {
            let node = self
                .index
                .node_by_flat_id(&written.flat_id)
                .map_err(|source| EngineError::Build {
                    source: Box::new(source),
                })?;
            let parts = written.value.parts().map_err(|source| EngineError::Rm {
                mapping: String::from(node.aql_path().as_str()),
                node: String::from(node.aql_path().as_str()),
                source: Box::new(source),
            })?;
            for part in parts {
                let mut value = NodeValue::new(node, part.value().clone())
                    .with_occurrences(written.positions.clone())
                    .under(part.sub_path().to_vec());
                if let Some(datum) = part.datum() {
                    value = value.with_datum(datum);
                }
                values.push(value);
            }
        }
        Ok(values)
    }
}

/// Returns how many axes of the output side the parent bound.
fn parent_depth(parent: &Binding, direction: Direction) -> usize {
    match direction {
        Direction::FhirToOpenehr => parent.openehr.len(),
        Direction::OpenehrToFhir => parent.fhir.indices().len(),
    }
}

/// Returns the repeating elements a FHIR path steps through, outermost first.
///
/// An `extension(url)` step takes an index of its own, because the url names
/// one entry of a repeating element
/// (<https://hl7.org/fhir/R4/fhirpath.html>, §Additional functions).
fn fhir_axes(target: &FhirTarget) -> Vec<String> {
    target
        .resolved()
        .moves()
        .iter()
        .filter_map(|step| match *step {
            Move::Member(ref field) => field.repeats().then(|| String::from(field.path())),
            Move::Extension { ref field, ref url } => Some(format!("{}({url})", field.path())),
            Move::Choice { .. }
            | Move::Ordinal(_)
            | Move::Index(_)
            | Move::Predicate(_)
            | Move::Resolve { .. } => None,
        })
        .collect()
}

/// Returns one `CODE_PHRASE`, for a defaulted composition field.
fn coded(code: &str, terminology: &str) -> RmValue {
    RmValue::CodePhrase(CodePhrase {
        terminology_id: TerminologyId {
            value: String::from(terminology),
        },
        code_string: String::from(code),
        preferred_term: None,
    })
}

/// Renders an index prefix, for the counter that appends under it.
fn render(indices: &[usize]) -> String {
    indices
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join(".")
}

/// Merges one manual path into the object the entry builds.
fn merge(object: &mut serde_json::Map<String, serde_json::Value>, segments: &[&str], value: &str) {
    let Some((first, rest)) = segments.split_first() else {
        object.insert(
            String::from("value"),
            serde_json::Value::String(String::from(value)),
        );
        return;
    };
    if rest.is_empty() {
        object.insert(
            String::from(*first),
            serde_json::Value::String(String::from(value)),
        );
        return;
    }
    let entry = object
        .entry(String::from(*first))
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let serde_json::Value::Object(ref mut nested) = *entry {
        merge(nested, rest, value);
    }
}

#[cfg(test)]
mod tests {
    use super::MappingFunctions;
    use super::NoMappingFunctions;
    use super::merge;
    use super::render;

    #[test]
    fn the_registry_this_milestone_ships_holds_nothing() {
        let error = NoMappingFunctions
            .to_openehr("diagnosis", None)
            .expect_err("the registry ships empty");
        assert!(
            error.to_string().contains("diagnosis"),
            "the refusal names the function: {error}"
        );
    }

    #[test]
    fn manual_paths_that_share_a_prefix_merge_into_one_element() {
        // manual.adoc: "`defining_code/terminology_id/value` and
        // `defining_code/code_string` are merged together in the same data
        // element and do not overwrite".
        let mut object = serde_json::Map::new();
        merge(
            &mut object,
            &["defining_code", "terminology_id", "value"],
            "openehr",
        );
        merge(&mut object, &["defining_code", "code_string"], "524");
        merge(&mut object, &["value"], "Initial");
        assert_eq!(
            serde_json::Value::Object(object),
            serde_json::json!({
                "defining_code": {"terminology_id": {"value": "openehr"}, "code_string": "524"},
                "value": "Initial"
            })
        );
    }

    #[test]
    fn an_index_prefix_renders_as_the_counter_key() {
        assert_eq!(render(&[]), "");
        assert_eq!(render(&[0, 2]), "0.2");
    }
}
