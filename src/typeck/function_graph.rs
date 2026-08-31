//! Declaration-level user-function dependencies.
//!
//! Ordinary generic inference must solve callees before callers and keep a
//! mutually recursive component monomorphic until its shared constraints are
//! known. This graph is intentionally syntax-directed: free calls resolve by
//! their unique source name, while a method call conservatively depends on
//! every source method with the written member name. Type checking later picks
//! the exact method; the conservative edge cannot make inference unsound.

use std::collections::HashMap;

use crate::{
    ast::{Expr, ExprKind, FunctionId, Program},
    visit::{self, Visitor},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FunctionComponent {
    pub(super) functions: Vec<FunctionId>,
}

/// Returns strongly connected components in dependency order: every component
/// containing a possible callee precedes the components that call it.
pub(super) fn dependency_order(program: &Program) -> Vec<FunctionComponent> {
    let mut free_functions = HashMap::new();
    let mut methods = HashMap::<&str, Vec<FunctionId>>::new();
    for function in &program.functions {
        if function.method_of.is_some() {
            methods
                .entry(function.name.as_str())
                .or_default()
                .push(function.id);
        } else {
            free_functions.insert(function.name.as_str(), function.id);
        }
    }

    // Function IDs are stable parser identities, not dense vector indexes.
    // Recovery can consume an ID for a malformed declaration and continue with
    // a later function, leaving a deliberate gap in the surviving syntax tree.
    let functions = program
        .functions
        .iter()
        .map(|function| function.id)
        .collect::<Vec<_>>();
    let positions = functions
        .iter()
        .enumerate()
        .map(|(position, function)| (*function, position))
        .collect::<HashMap<_, _>>();
    let mut edges = vec![Vec::new(); functions.len()];
    for function in &program.functions {
        let mut collector = DependencyCollector {
            free_functions: &free_functions,
            methods: &methods,
            dependencies: Vec::new(),
        };
        collector.visit_block(&function.body);
        collector
            .dependencies
            .sort_by_key(|function| positions[function]);
        collector.dependencies.dedup();
        edges[positions[&function.id]] = collector
            .dependencies
            .into_iter()
            .map(|dependency| positions[&dependency])
            .collect();
    }

    Components::new(edges, functions).finish()
}

struct DependencyCollector<'a> {
    free_functions: &'a HashMap<&'a str, FunctionId>,
    methods: &'a HashMap<&'a str, Vec<FunctionId>>,
    dependencies: Vec<FunctionId>,
}

impl<'ast> Visitor<'ast> for DependencyCollector<'_> {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let ExprKind::Call { callee, .. } = &expression.kind {
            match callee.as_slice() {
                [name] => {
                    if let Some(function) = self.free_functions.get(name.as_str()) {
                        self.dependencies.push(*function);
                    }
                }
                [.., method] => {
                    if let Some(functions) = self.methods.get(method.as_str()) {
                        self.dependencies.extend(functions.iter().copied());
                    }
                }
                [] => {}
            }
        }
        visit::walk_expr(self, expression);
    }
}

struct Components {
    edges: Vec<Vec<usize>>,
    functions: Vec<FunctionId>,
    next_index: usize,
    indices: Vec<Option<usize>>,
    lowlinks: Vec<usize>,
    stack: Vec<usize>,
    on_stack: Vec<bool>,
    output: Vec<FunctionComponent>,
}

impl Components {
    fn new(edges: Vec<Vec<usize>>, functions: Vec<FunctionId>) -> Self {
        let count = edges.len();
        Self {
            edges,
            functions,
            next_index: 0,
            indices: vec![None; count],
            lowlinks: vec![0; count],
            stack: Vec::new(),
            on_stack: vec![false; count],
            output: Vec::new(),
        }
    }

    fn finish(mut self) -> Vec<FunctionComponent> {
        for function in 0..self.functions.len() {
            if self.indices[function].is_none() {
                self.visit(function);
            }
        }
        self.output
    }

    fn visit(&mut self, function: usize) {
        let index = self.next_index;
        self.next_index += 1;
        self.indices[function] = Some(index);
        self.lowlinks[function] = index;
        self.stack.push(function);
        self.on_stack[function] = true;

        // Clone this small, deduplicated adjacency list so recursive traversal
        // can mutably update the graph state without split-borrow machinery.
        for dependency in self.edges[function].clone() {
            if self.indices[dependency].is_none() {
                self.visit(dependency);
                self.lowlinks[function] = self.lowlinks[function].min(self.lowlinks[dependency]);
            } else if self.on_stack[dependency] {
                self.lowlinks[function] = self.lowlinks[function]
                    .min(self.indices[dependency].expect("visited functions have indices"));
            }
        }

        if self.lowlinks[function] != index {
            return;
        }
        let mut functions = Vec::new();
        loop {
            let member = self
                .stack
                .pop()
                .expect("a component root remains on the traversal stack");
            self.on_stack[member] = false;
            functions.push(self.functions[member]);
            if member == function {
                break;
            }
        }
        functions.sort_by_key(|function| function.index());
        self.output.push(FunctionComponent { functions });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component_names(source: &str) -> Vec<Vec<String>> {
        let parsed = crate::parse(source).expect("function graph fixture should parse");
        dependency_order(&parsed.syntax)
            .into_iter()
            .map(|component| {
                component
                    .functions
                    .into_iter()
                    .map(|function| {
                        parsed
                            .syntax
                            .functions
                            .iter()
                            .find(|declaration| declaration.id == function)
                            .expect("graph identities belong to source functions")
                            .name
                            .clone()
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn orders_callees_before_callers_and_groups_recursion() {
        let names = component_names(
            r#"
                state "game.exe" {}

                fn top(value: i32) -> i32 { return middle(value) }
                fn middle(value: i32) -> i32 { return leaf(value) }
                fn leaf(value: i32) -> i32 { return value }

                fn first(value: i32) -> i32 {
                    if value == 0 { return value }
                    return second(value - 1)
                }
                fn second(value: i32) -> i32 {
                    if value == 0 { return value }
                    return first(value - 1)
                }
            "#,
        );

        assert_eq!(
            names,
            vec![
                vec!["leaf"],
                vec!["middle"],
                vec!["top"],
                vec!["first", "second"],
            ]
        );
    }

    #[test]
    fn method_calls_depend_on_the_written_source_method() {
        let names = component_names(
            r#"
                state "game.exe" {}
                struct Box { value: i32 }

                fn use(box: Box) -> i32 { return box.unwrap() }
                fn Box.unwrap() -> i32 { return self.value }
            "#,
        );

        assert_eq!(names, vec![vec!["unwrap"], vec!["use"]]);
    }

    #[test]
    fn recovered_function_identity_gaps_are_not_vector_indexes() {
        let recovered = crate::parse_recovering(
            r#"
                fn before() { return }
                fn missing()
                debug fn after() { return before() }
            "#,
        )
        .expect("parser recovery should retain later declarations");
        assert!(!recovered.diagnostics().is_empty());
        assert!(
            recovered
                .syntax()
                .functions
                .iter()
                .any(|function| function.id.index() >= recovered.syntax().functions.len())
        );

        let names = dependency_order(recovered.syntax())
            .into_iter()
            .map(|component| {
                component
                    .functions
                    .into_iter()
                    .map(|function| {
                        recovered
                            .syntax()
                            .functions
                            .iter()
                            .find(|declaration| declaration.id == function)
                            .unwrap()
                            .name
                            .clone()
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec![vec!["before"], vec!["after"]]);
    }
}
