// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The strongly connected components of the type reference graph.

use std::collections::{BTreeMap, BTreeSet};

/// Tarjan's algorithm over the containment graph; components in a stable order.
pub(super) fn strongly_connected_components(
    edges: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<Vec<String>> {
    let mut search = TarjanSearch {
        edges,
        index: BTreeMap::new(),
        low: BTreeMap::new(),
        on_stack: BTreeSet::new(),
        stack: Vec::new(),
        next: 0,
        components: Vec::new(),
    };
    for node in edges.keys() {
        if !search.index.contains_key(node) {
            search.visit(node);
        }
    }
    search.components
}

/// One depth-first search of Tarjan's algorithm, and the components it found.
struct TarjanSearch<'a> {
    edges: &'a BTreeMap<String, BTreeSet<String>>,
    index: BTreeMap<String, usize>,
    low: BTreeMap<String, usize>,
    on_stack: BTreeSet<String>,
    stack: Vec<String>,
    next: usize,
    components: Vec<Vec<String>>,
}

impl TarjanSearch<'_> {
    /// Visits `node`, its unvisited successors, and pops a component at a root.
    fn visit(&mut self, node: &str) {
        self.index.insert(node.to_owned(), self.next);
        self.low.insert(node.to_owned(), self.next);
        self.next += 1;
        self.stack.push(node.to_owned());
        self.on_stack.insert(node.to_owned());
        let successors: Vec<String> = self
            .edges
            .get(node)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default();
        for next in successors {
            if !self.index.contains_key(&next) {
                self.visit(&next);
                let next_low = self.low.get(&next).copied().unwrap_or(usize::MAX);
                let own = self.low.get(node).copied().unwrap_or(usize::MAX);
                self.low.insert(node.to_owned(), own.min(next_low));
            } else if self.on_stack.contains(&next) {
                let next_index = self.index.get(&next).copied().unwrap_or(usize::MAX);
                let own = self.low.get(node).copied().unwrap_or(usize::MAX);
                self.low.insert(node.to_owned(), own.min(next_index));
            }
        }
        if self.low.get(node) == self.index.get(node) {
            let component = self.pop_component(node);
            self.components.push(component);
        }
    }

    /// Pops the stack down to `root`, the component it closes, in name order.
    fn pop_component(&mut self, root: &str) -> Vec<String> {
        let mut component = Vec::new();
        while let Some(top) = self.stack.pop() {
            self.on_stack.remove(&top);
            let done = top == root;
            component.push(top);
            if done {
                break;
            }
        }
        component.sort();
        component
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::strongly_connected_components;

    fn graph(pairs: &[(&str, &str)]) -> BTreeMap<String, BTreeSet<String>> {
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (from, to) in pairs {
            edges
                .entry((*from).to_owned())
                .or_default()
                .insert((*to).to_owned());
            edges.entry((*to).to_owned()).or_default();
        }
        edges
    }

    #[test]
    fn a_two_cycle_is_one_component() {
        let components = strongly_connected_components(&graph(&[
            ("Identifier", "Reference"),
            ("Reference", "Identifier"),
            ("Coding", "Identifier"),
        ]));
        assert!(components.contains(&vec!["Identifier".to_owned(), "Reference".to_owned()]));
        assert!(components.contains(&vec!["Coding".to_owned()]));
    }

    #[test]
    fn a_dag_has_singleton_components() {
        let components = strongly_connected_components(&graph(&[("A", "B"), ("B", "C")]));
        assert_eq!(components.len(), 3);
        assert!(components.iter().all(|c| c.len() == 1));
    }
}
