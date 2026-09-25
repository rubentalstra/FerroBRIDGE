// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Instances of a composition, and the walk from one to another.
//!
//! An instance is one node of the canonical JSON together with the path that
//! reaches it from the composition root, every step carrying the 1-based
//! position of the node among the siblings of its archetype node id (openEHR
//! BASE Release 1.2.0 §Paths and Locators, positional predicates). That path
//! is what a record key names, and what a parent step walks back up.

use openehr_rm::v1_2::paths::PathSegment;
use openehr_rm::v1_2::paths::Predicate;
use openehr_rm::v1_2::paths::select_children;
use serde_json::Value;

use crate::resolve::program::Hop;

/// One step of an instance path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Step {
    attribute: String,
    node_id: Option<String>,
    position: usize,
}

/// One node of a composition, with the path that reaches it.
#[derive(Debug, Clone)]
pub(crate) struct Instance<'a> {
    value: &'a Value,
    steps: Vec<Step>,
}

impl<'a> Instance<'a> {
    /// Returns the composition root.
    pub(crate) const fn root(composition: &'a Value) -> Self {
        Self {
            value: composition,
            steps: Vec::new(),
        }
    }

    /// Returns the node.
    pub(crate) const fn value(&self) -> &'a Value {
        self.value
    }

    /// Returns the instance path, every segment with its position.
    pub(crate) fn path(&self) -> String {
        render(&self.steps)
    }
}

/// Returns every instance `hop` reaches from `from`, in document order.
///
/// A parent step walks back along `from`'s own path, so `../` always names
/// the ancestor the instance was reached through.
pub(crate) fn follow<'a>(
    composition: &'a Value,
    from: &Instance<'a>,
    hop: &Hop,
) -> Vec<Instance<'a>> {
    let start = if hop.up() == 0 {
        from.clone()
    } else {
        let Some(kept) = from.steps.len().checked_sub(hop.up()) else {
            return Vec::new();
        };
        let mut steps = from.steps.clone();
        steps.truncate(kept);
        let Some(ancestor) = revisit(composition, &steps) else {
            return Vec::new();
        };
        ancestor
    };
    let mut current = vec![start];
    for segment in hop.down() {
        let mut next = Vec::new();
        for instance in &current {
            next.append(&mut children(instance, segment));
        }
        current = next;
    }
    current
}

/// Returns every child of `instance` one written segment selects.
fn children<'a>(instance: &Instance<'a>, segment: &PathSegment) -> Vec<Instance<'a>> {
    select_children(instance.value, segment)
        .into_iter()
        .map(|child| {
            let node_id = child
                .get("archetype_node_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let identity = identity(&segment.attribute, node_id.as_deref());
            let position = select_children(instance.value, &identity)
                .iter()
                .position(|sibling| core::ptr::eq(*sibling, child))
                .map_or(1, |index| index.saturating_add(1));
            let mut steps = instance.steps.clone();
            steps.push(Step {
                attribute: segment.attribute.clone(),
                node_id,
                position,
            });
            Instance {
                value: child,
                steps,
            }
        })
        .collect()
}

/// Walks the composition along an instance path.
fn revisit<'a>(composition: &'a Value, steps: &[Step]) -> Option<Instance<'a>> {
    let mut current = composition;
    for step in steps {
        let identity = identity(&step.attribute, step.node_id.as_deref());
        current = *select_children(current, &identity).get(step.position.checked_sub(1)?)?;
    }
    Some(Instance {
        value: current,
        steps: steps.to_vec(),
    })
}

/// Returns the segment that selects the siblings of one archetype node id.
fn identity(attribute: &str, node_id: Option<&str>) -> PathSegment {
    PathSegment {
        attribute: attribute.to_owned(),
        predicate: Predicate {
            archetype_node_id: node_id.map(str::to_owned),
            ..Predicate::default()
        },
        descendant: false,
    }
}

/// Renders an instance path.
fn render(steps: &[Step]) -> String {
    let mut rendered = String::new();
    for step in steps {
        rendered.push('/');
        rendered.push_str(&step.attribute);
        rendered.push('[');
        if let Some(ref node_id) = step.node_id {
            rendered.push_str(node_id);
            rendered.push_str(" and ");
        }
        rendered.push_str(&step.position.to_string());
        rendered.push(']');
    }
    rendered
}

/// Returns the instance path of every `ELEMENT` of the composition that
/// carries a value, in document order.
pub(crate) fn valued_elements(composition: &Value) -> Vec<String> {
    let mut found = Vec::new();
    collect(composition, &mut Vec::new(), &mut found);
    found
}

/// Collects the valued elements under one node.
fn collect(node: &Value, steps: &mut Vec<Step>, found: &mut Vec<String>) {
    let Value::Object(object) = node else {
        return;
    };
    let class = object.get("_type").and_then(Value::as_str);
    if class.is_some_and(|class| openehr_rm::v1_2::model::is_a(class, "ELEMENT")) {
        if object.contains_key("value") {
            found.push(render(steps));
        }
        return;
    }
    for (attribute, child) in object {
        let items: Vec<&Value> = match *child {
            Value::Array(ref items) => items.iter().collect(),
            Value::Object(_) => vec![child],
            _ => continue,
        };
        let mut seen: Vec<(&str, usize)> = Vec::new();
        for (index, item) in items.into_iter().enumerate() {
            let node_id = item.get("archetype_node_id").and_then(Value::as_str);
            // A step with no node id selects every sibling, as `identity` does.
            let position = node_id.map_or(index.saturating_add(1), |id| {
                if let Some(entry) = seen.iter_mut().find(|entry| entry.0 == id) {
                    entry.1 = entry.1.saturating_add(1);
                    return entry.1;
                }
                seen.push((id, 1));
                1
            });
            steps.push(Step {
                attribute: attribute.clone(),
                node_id: node_id.map(str::to_owned),
                position,
            });
            collect(item, steps, found);
            steps.pop();
        }
    }
}
