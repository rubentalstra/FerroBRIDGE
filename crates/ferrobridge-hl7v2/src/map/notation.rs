// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The target notation of the v2-to-FHIR `ConceptMaps`.
//!
//! A target code is a FHIR element path with two additions the mapping
//! guidelines define (`mapping_guidelines.md` §\[n\] Notation, vendored under
//! `tools/fhir-codegen/vendor/hl7.fhir.uv.v2mappings/repository/`): `[n]`
//! after an element names one instance, so rows with the same `[n]` at the
//! same place fill the same instance and rows with different ones fill
//! different instances; and a leading `[n].` in a data type map names the
//! instance of the element the data type is used at. `(Type)` after an element
//! is a reference to a resource of that type the mapping creates, optionally
//! followed by the path inside it (`performer(PractitionerRole.practitioner)`).
//! `$value` names the element the data type is used at itself. A `logos`
//! lexer feeds a `chumsky` parser.
//!
//! ```text
//! target    := ( label "." )? ( "$value" | path )
//! path      := step ( "." step )*
//! step      := name label? ( "(" reference ")" )?
//! reference := name label? ( "." path )?
//! label     := "[" ( digits | "n" ) "]"
//! ```
//!
//! `$this`, `[each]` and `[n-m]` are refused: the first names no element to
//! write, and the others spread one value over several instances, which this
//! interpreter does not do.

use core::fmt;

use chumsky::prelude::*;
use logos::Logos;

/// The instance label of a step, `[1]` or `[n]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Label {
    /// `[k]`, from 1.
    Number(u32),
    /// `[n]`, a new instance per source occurrence.
    Each,
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(number) => write!(f, "{number}"),
            Self::Each => f.write_str("n"),
        }
    }
}

/// One step of a target path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The element name as written.
    pub name: String,
    /// The instance label, when one was written.
    pub label: Option<Label>,
    /// The resource this element references, when `(Type)` follows it.
    pub reference: Option<Reference>,
}

/// A `(Type...)` reference to a resource the mapping creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The resource type.
    pub resource: String,
    /// The instance label of the resource, when one was written.
    pub label: Option<Label>,
    /// The path inside the resource, empty for the resource itself.
    pub path: Vec<Step>,
}

/// A parsed target code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A path, with the leading `[n].` of a data type map when one was
    /// written.
    Path {
        /// The instance of the element the data type is used at.
        instance: Option<Label>,
        /// The steps.
        steps: Vec<Step>,
    },
    /// `$value`: the element the data type is used at.
    Value {
        /// The instance of that element.
        instance: Option<Label>,
    },
}

/// Why a target code could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotationError {
    /// The code is empty.
    #[error("the target is empty")]
    Empty,
    /// `$this`, which names no element to write.
    #[error("`$this` names no element to write")]
    This,
    /// `[each]` or `[n-m]`, which spread one value over several instances.
    #[error("a label spreads one value over several instances")]
    Spread,
    /// A character sequence no token of the notation matches.
    #[error("no token of the target notation matches {slice:?} at byte {offset}")]
    Lex {
        /// The byte offset.
        offset: usize,
        /// The text no token matches.
        slice: String,
    },
    /// The tokens do not match the notation.
    #[error("the target leaves the notation at token {token}")]
    Syntax {
        /// The index of the first token not admitted.
        token: usize,
    },
}

/// One token of a target code.
#[derive(Logos, Debug, Clone, PartialEq, Eq)]
enum Token {
    #[token("$value")]
    Value,
    #[token("$this")]
    This,
    #[token("[")]
    OpenLabel,
    #[token("]")]
    CloseLabel,
    #[token("(")]
    Open,
    #[token(")")]
    Close,
    #[token(".")]
    Dot,
    #[token("-")]
    Dash,
    #[regex(r"[0-9]+", |lexer| lexer.slice().parse::<u32>().ok())]
    Number(u32),
    #[regex(r"[A-Za-z][A-Za-z0-9]*", |lexer| String::from(lexer.slice()))]
    Name(String),
}

/// The chumsky extra parameter of this grammar.
#[expect(
    unused_qualifications,
    reason = "the alias shadows the name it qualifies, so the path keeps the definition from naming itself"
)]
type Extra<'a> = chumsky::extra::Err<Simple<'a, Token>>;

/// Parses a target code.
///
/// # Errors
///
/// Returns [`NotationError`] for an empty code, `$this`, a spreading label,
/// text no token matches, and tokens outside the notation.
pub fn parse(code: &str) -> Result<Target, NotationError> {
    let code = code.trim();
    if code.is_empty() {
        return Err(NotationError::Empty);
    }
    let mut tokens = Vec::new();
    let mut lexer = Token::lexer(code);
    while let Some(token) = lexer.next() {
        match token {
            Ok(token) => tokens.push(token),
            Err(()) => {
                return Err(NotationError::Lex {
                    offset: lexer.span().start,
                    slice: String::from(lexer.slice()),
                });
            }
        }
    }
    if tokens.contains(&Token::This) {
        return Err(NotationError::This);
    }
    if spreads(&tokens) {
        return Err(NotationError::Spread);
    }
    target()
        .parse(tokens.as_slice())
        .into_result()
        .map_err(|errors| NotationError::Syntax {
            token: errors
                .first()
                .map_or(tokens.len(), |error| error.span().start),
        })
}

/// Whether a label is `[each]` or `[n-m]`.
fn spreads(tokens: &[Token]) -> bool {
    tokens.windows(3).any(|window| match window {
        [Token::OpenLabel, Token::Name(name), Token::CloseLabel] => name == "each",
        [Token::Number(_), Token::Dash, Token::Number(_)] => true,
        _ => false,
    })
}

/// The target notation.
fn target<'a>() -> impl Parser<'a, &'a [Token], Target, Extra<'a>> {
    let label = select! {
        Token::Number(number) if number > 0 => Label::Number(number),
        Token::Name(name) if name == "n" => Label::Each,
    }
    .delimited_by(just(Token::OpenLabel), just(Token::CloseLabel));
    let name = select! { Token::Name(name) => name };
    let path = recursive(|path| {
        let reference = name
            .then(label.clone().or_not())
            .then(just(Token::Dot).ignore_then(path).or_not())
            .delimited_by(just(Token::Open), just(Token::Close))
            .map(
                |((resource, label), path): ((String, Option<Label>), Option<Vec<Step>>)| {
                    Reference {
                        resource,
                        label,
                        path: path.unwrap_or_default(),
                    }
                },
            );
        name.then(label.clone().or_not())
            .then(reference.or_not())
            .map(|((name, label), reference)| Step {
                name,
                label,
                reference,
            })
            .separated_by(just(Token::Dot))
            .at_least(1)
            .collect::<Vec<_>>()
    });
    let instance = label.then_ignore(just(Token::Dot)).or_not();
    instance
        .then(just(Token::Value).to(None).or(path.map(Some)))
        .then_ignore(end())
        .map(|(instance, steps)| match steps {
            Some(steps) => Target::Path { instance, steps },
            None => Target::Value { instance },
        })
}

#[cfg(test)]
mod tests {
    use super::{Label, NotationError, Reference, Step, Target, parse};

    fn step(name: &str, label: Option<Label>) -> Step {
        Step {
            name: String::from(name),
            label,
            reference: None,
        }
    }

    #[test]
    fn an_instance_label_names_the_element_it_follows() {
        assert_eq!(
            parse("address[1].district"),
            Ok(Target::Path {
                instance: None,
                steps: vec![
                    step("address", Some(Label::Number(1))),
                    step("district", None)
                ],
            })
        );
    }

    #[test]
    fn a_data_type_map_prefix_names_the_outer_instance() {
        assert_eq!(
            parse("[2].given"),
            Ok(Target::Path {
                instance: Some(Label::Number(2)),
                steps: vec![step("given", None)],
            })
        );
        assert_eq!(parse("$value"), Ok(Target::Value { instance: None }));
        assert_eq!(parse("Specimen[n]").map(|_| ()), Ok(()));
    }

    #[test]
    fn a_reference_carries_the_resource_and_the_path_inside_it() {
        let parsed = parse("performer[2](PractitionerRole[1].organization(Organization))");
        let inner = Step {
            name: String::from("organization"),
            label: None,
            reference: Some(Reference {
                resource: String::from("Organization"),
                label: None,
                path: Vec::new(),
            }),
        };
        assert_eq!(
            parsed,
            Ok(Target::Path {
                instance: None,
                steps: vec![Step {
                    name: String::from("performer"),
                    label: Some(Label::Number(2)),
                    reference: Some(Reference {
                        resource: String::from("PractitionerRole"),
                        label: Some(Label::Number(1)),
                        path: vec![inner],
                    }),
                }],
            })
        );
    }

    #[test]
    fn spreading_labels_and_unbalanced_references_are_refused() {
        assert_eq!(parse("line[1-3]"), Err(NotationError::Spread));
        assert_eq!(parse("component[each].code"), Err(NotationError::Spread));
        assert!(matches!(
            parse("extension[1].valueReference(Group.member.entity(Specimen.identifier[1])"),
            Err(NotationError::Syntax { .. })
        ));
        assert_eq!(parse("$this"), Err(NotationError::This));
        assert!(matches!(
            parse("system.extension-data-absent-reason"),
            Err(NotationError::Syntax { .. })
        ));
        assert!(matches!(
            parse("identifier[0]"),
            Err(NotationError::Syntax { .. })
        ));
    }
}
