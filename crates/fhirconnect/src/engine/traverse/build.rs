// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The composition defaults and the build of the values a run produced.

use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::composition::NodeValue;
use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::index::paths::AqlPath;
use openehr_mapping_core::template::PathError;
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use openehr_rm::v1_2::support::terminology::openehr_terminology_group_identifiers::OpenehrTerminologyGroupIdentifiersData;

use crate::engine::outcome::Warning;
use crate::engine::rm;
use crate::engine::rm::RmError;
use crate::engine::rm::RmValue;

use crate::tree::element::Table;

use crate::engine::traverse::Defaults;
use crate::engine::traverse::Held;
use crate::engine::traverse::Routed;
use crate::engine::traverse::Run;
use crate::engine::traverse::error::EngineError;

impl<'a, T: Table + ?Sized> Run<'a, T> {
    /// Fills the composition fields no mapping wrote.
    ///
    /// Returns the openEHR path of every field it filled, in the order it
    /// filled them, which is the order of the [`Warning::Defaulted`] entries
    /// it recorded. Which fields take a default is FHIRconnect's
    /// (`engine/defaults-for-fields.adoc`); every default travels as the `ctx/`
    /// key the FLAT builder resolves (Simplified Formats, master06 §Composer,
    /// §time, §setting, §Language and Territory). `ctx/setting` carries the
    /// code and the builder takes the rubric from the openEHR terminology's
    /// [`OpenehrTerminologyGroupIdentifiersData::GROUP_ID_SETTING`] group, so
    /// the project's value is checked against the built composition.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Template`] when the template cannot answer for a
    /// defaulted field.
    pub(super) fn apply_defaults(
        &mut self,
        defaults: &Defaults,
    ) -> Result<Vec<String>, EngineError> {
        let mut filled = Vec::new();
        self.default_context(
            &mut filled,
            "/composer",
            "composer_name",
            &defaults.composer,
        )?;
        self.default_context(
            &mut filled,
            "/context/start_time",
            "time",
            &defaults.start_time,
        )?;
        if let Some(node) = self.unwritten("/context/setting")? {
            let setting = defaults.setting.clone();
            self.context_keys.push(NodeValue::context(
                "setting",
                serde_json::Value::String(setting.code.clone()),
            ));
            self.expected.push((
                node.flat_id().clone(),
                RmValue::CodedText(Box::new(DvCodedText {
                    value: setting.value,
                    hyperlink: None,
                    formatting: None,
                    mappings: None,
                    language: None,
                    encoding: None,
                    defining_code: CodePhrase {
                        terminology_id: TerminologyId {
                            value: String::from(
                                OpenehrTerminologyGroupIdentifiersData::TERMINOLOGY_ID_OPENEHR,
                            ),
                        },
                        code_string: setting.code,
                        preferred_term: None,
                    },
                })),
            ));
            self.defaulted(&mut filled, "/context/setting");
        }
        if let Some(ref code) = defaults.language {
            self.default_context(&mut filled, "/language", "language", code)?;
        }
        if let Some(ref code) = defaults.territory {
            self.default_context(&mut filled, "/territory", "territory", code)?;
        }
        Ok(filled)
    }

    /// Writes one default as the `ctx/` key `field`, when the template holds
    /// a node at `path` and nothing wrote it already.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Template`] when the template cannot answer for
    /// the path for any reason other than holding no node at it.
    fn default_context(
        &mut self,
        filled: &mut Vec<String>,
        path: &str,
        field: &str,
        value: &str,
    ) -> Result<(), EngineError> {
        if self.unwritten(path)?.is_some() {
            self.context_keys.push(NodeValue::context(
                field,
                serde_json::Value::String(String::from(value)),
            ));
            self.defaulted(filled, path);
        }
        Ok(())
    }

    /// Records that the engine filled the field at `path`.
    fn defaulted(&mut self, filled: &mut Vec<String>, path: &str) {
        self.warnings.push(Warning::Defaulted {
            field: String::from(path),
        });
        filled.push(String::from(path));
    }

    /// Returns the node at `path` when the template holds one and no mapping
    /// wrote it.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Template`] when the template cannot answer for
    /// the path for any reason other than holding no node at it.
    fn unwritten(&self, path: &str) -> Result<Option<&'a ResolvedNode>, EngineError> {
        let node = match self.index.node(&AqlPath::new(path)) {
            Ok(node) => node,
            // NOTE: engine/defaults-for-fields.adoc, the context "is usually
            // populated", so a template with no node at the path takes no default.
            Err(PathError::UnknownPath { .. }) => return Ok(None),
            Err(source) => {
                return Err(EngineError::Template {
                    mapping: String::from(self.program.context().as_str()),
                    node: String::from(path),
                    source: Box::new(source),
                });
            }
        };
        let written = self
            .written
            .iter()
            .any(|written| written.flat_id == *node.flat_id());
        Ok((!written).then_some(node))
    }

    /// Returns the values the run produced, as the composition builder wants
    /// them, with the values the builder takes through the `ctx/` keys.
    ///
    /// Every value travels whole as its canonical JSON under `|raw`
    /// (Simplified Formats, master04 §Raw canonical JSON), except at a
    /// composition attribute the builder sets from the context vocabulary
    /// ([`context_keys`]), whose value is returned beside its node so the run
    /// can check the built composition carries it whole.
    pub(super) fn node_values(&self) -> Result<(Vec<NodeValue>, Vec<Routed>), EngineError> {
        let mut values = Vec::new();
        let mut routed = self.expected.clone();
        for written in &self.written {
            let node = self
                .index
                .node_by_flat_id(&written.flat_id)
                .map_err(|source| EngineError::Build {
                    source: Box::new(source),
                })?;
            let refuse = |source: RmError| EngineError::Rm {
                mapping: String::from(node.aql_path().as_str()),
                node: String::from(node.aql_path().as_str()),
                source: Box::new(source),
            };
            let value = match written.value {
                Held::Value(ref value) => value.clone(),
                Held::Partial(ref object) => {
                    let rm_type = object
                        .get("_type")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(node.rm_type());
                    RmValue::from_canonical(
                        rm_type,
                        node.aql_path().as_str(),
                        &serde_json::Value::Object(object.clone()),
                    )
                    .map_err(refuse)?
                }
            };
            if let Some(keys) = context_keys(node, &value) {
                values.extend(keys);
                routed.push((written.flat_id.clone(), value));
                continue;
            }
            values.push(
                NodeValue::new(node, value.to_canonical().map_err(refuse)?)
                    .with_occurrences(written.positions.clone())
                    .with_datum(rm::RAW),
            );
        }
        values.extend(self.families.iter().cloned());
        values.extend(self.context_keys.iter().cloned());
        Ok((values, routed))
    }

    /// Refuses a composition whose context-set attributes do not carry the
    /// value the run wrote there whole.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::ContextAttribute`] when the built value differs
    /// from the written one, and [`EngineError::Build`] or
    /// [`EngineError::Rm`] when the built value cannot be read back.
    pub(super) fn carried_whole(
        &self,
        built: &CanonicalComposition,
        routed: &[Routed],
    ) -> Result<(), EngineError> {
        let refuse_build = |source: PathError| EngineError::Build {
            source: Box::new(source),
        };
        for (flat_id, written) in routed {
            let node = self.index.node_by_flat_id(flat_id).map_err(refuse_build)?;
            let path = node.aql_path().as_str();
            let back = self
                .index
                .read(built, node, &[])
                .map_err(refuse_build)?
                .map(|value| RmValue::from_canonical(node.rm_type(), path, &value))
                .transpose()
                .map_err(|source| EngineError::Rm {
                    mapping: String::from(self.program.context().as_str()),
                    node: String::from(path),
                    source: Box::new(source),
                })?;
            if back.as_ref() != Some(written) {
                return Err(EngineError::ContextAttribute {
                    mapping: String::from(self.program.context().as_str()),
                    node: String::from(path),
                });
            }
        }
        Ok(())
    }
}

// TODO(#241): the three attributes travel as `ctx/` keys until openehr-sdt
// honours `|raw` on the COMPOSITION `language`, `territory` and `composer`
// nodes, which its builder routes through the context (new sibling request S6).
/// Returns the `ctx/` keys a value at a composition attribute the FLAT
/// builder sets from the context vocabulary travels as, `None` for any other
/// node.
///
/// Simplified Formats, master06 §Composer and §Language and Territory:
/// `ctx/composer_name` sets the composer's name and `ctx/language` and
/// `ctx/territory` the two codes. A part the keys do not hold is left out
/// here and refused by [`Run::carried_whole`] once the composition is built.
pub(super) fn context_keys(node: &ResolvedNode, value: &RmValue) -> Option<Vec<NodeValue>> {
    let key = |field: &str, text: &str| {
        NodeValue::context(field, serde_json::Value::String(String::from(text)))
    };
    let field = match node.aql_path().as_str() {
        "/language" => "language",
        "/territory" => "territory",
        "/composer" => "composer_name",
        _ => return None,
    };
    Some(match *value {
        RmValue::CodePhrase(ref code) if field != "composer_name" => {
            vec![key(field, &code.code_string)]
        }
        RmValue::Party(ref party) if field == "composer_name" => {
            party.name.iter().map(|name| key(field, name)).collect()
        }
        _ => Vec::new(),
    })
}
