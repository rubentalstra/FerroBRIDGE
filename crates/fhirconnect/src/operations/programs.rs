// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The compiled programs and templates one call selects from.
//!
//! A program is compiled once at load and never parsed again
//! (`docs/architecture.md` §4.3), so the operations surface holds the compiled
//! set and picks one per request. The templates travel with it, because the
//! interpreter reads a composition through the index of the same template the
//! program compiled against.

use std::collections::BTreeMap;
use std::sync::Arc;

use openehr_mapping_core::index::WebTemplateIndex;

use crate::resolve::program::Program;
use crate::resolve::program::binding::TemplateId;

/// Why a program does not join the set.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProgramSetError {
    /// The program was compiled against a template the set does not hold.
    #[error("the context `{context}` maps the template `{template}`, which the set does not hold")]
    UnknownTemplate {
        /// The context mapping the program compiled from.
        context: String,
        /// The template it names.
        template: String,
    },
}

/// The compiled programs and the templates they map.
#[derive(Debug, Default)]
pub struct ProgramSet {
    programs: Vec<Arc<Program>>,
    templates: BTreeMap<String, WebTemplateIndex>,
}

impl ProgramSet {
    /// Creates an empty set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            programs: Vec::new(),
            templates: BTreeMap::new(),
        }
    }

    /// Adds a template index, replacing one of the same identifier.
    pub fn insert_template(&mut self, index: WebTemplateIndex) {
        self.templates.insert(index.template_id().to_owned(), index);
    }

    /// Adds a compiled program.
    ///
    /// # Errors
    ///
    /// Returns [`ProgramSetError::UnknownTemplate`] when the set holds no
    /// index for the template the program compiled against, because a run
    /// reads its composition through that index.
    pub fn insert_program(&mut self, program: Arc<Program>) -> Result<(), ProgramSetError> {
        let template = program.template().id().clone();
        if !self.templates.contains_key(template.as_str()) {
            return Err(ProgramSetError::UnknownTemplate {
                context: String::from(program.context().as_str()),
                template: String::from(template.as_str()),
            });
        }
        self.programs.push(program);
        Ok(())
    }

    /// Returns every compiled program, in insertion order.
    #[must_use]
    pub fn programs(&self) -> &[Arc<Program>] {
        &self.programs
    }

    /// Returns the index of the template `id` names.
    #[must_use]
    pub fn template(&self, id: &TemplateId) -> Option<&WebTemplateIndex> {
        self.templates.get(id.as_str())
    }

    /// Returns whether the set holds no program.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.programs.is_empty()
    }

    /// Returns how many programs the set holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.programs.len()
    }
}
