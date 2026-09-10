#![allow(
    clippy::unwrap_used,
    reason = "Style test failures must fail the check"
)]

use proc_macro2::{TokenStream, TokenTree};
use std::{fs, path::Path};
use syn::visit::{self, Visit};

#[derive(Default)]
struct Style {
    depth: usize,
    violations: Vec<String>,
}

impl Style {
    fn function(&mut self, block: &syn::Block) {
        let enclosing = self.depth;
        self.depth = 0;
        visit::visit_block(self, block);
        self.depth = enclosing;
    }
}

impl<'ast> Visit<'ast> for Style {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.function(&node.block);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.function(&node.block);
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.depth += 1;
        if self.depth > 2 {
            self.violations.push(format!(
                "line {}: nesting {} exceeds 2",
                block.brace_token.span.open().start().line,
                self.depth
            ));
        }
        visit::visit_block(self, block);
        self.depth -= 1;
    }
}

fn reject_else(tokens: TokenStream, violations: &mut Vec<String>) {
    for token in tokens {
        match token {
            TokenTree::Group(group) => reject_else(group.stream(), violations),
            TokenTree::Ident(ident) if ident == "else" => {
                violations.push(format!(
                    "line {}: else is forbidden",
                    ident.span().start().line
                ));
            }
            _ => (),
        }
    }
}

fn violations(source: &str) -> Vec<String> {
    let mut style = Style::default();
    style.visit_file(&syn::parse_file(source).unwrap());
    reject_else(source.parse().unwrap(), &mut style.violations);
    style.violations
}

fn check_dir(path: &Path, found: &mut Vec<String>) {
    for entry in fs::read_dir(path).unwrap() {
        check_path(&entry.unwrap().path(), found);
    }
}

fn check_path(path: &Path, found: &mut Vec<String>) {
    if path.is_dir() {
        check_dir(path, found);
        return;
    }
    if path.extension().is_none_or(|ext| ext != "rs") {
        return;
    }
    found.extend(
        violations(&fs::read_to_string(path).unwrap())
            .into_iter()
            .map(|v| format!("{}: {v}", path.display())),
    );
}

#[test]
fn repository_rust_obeys_style() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    check_dir(&root.join("src"), &mut found);
    check_dir(&root.join("tests"), &mut found);
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn style_boundaries_are_enforced() {
    assert!(violations("fn f() { if true { loop { break; } } }").is_empty());
    assert!(!violations("fn f() { if true { loop { if true {} } } }").is_empty());
    assert!(!violations("fn f() { if true {} else {} }").is_empty());
    assert!(!violations("fn f() { let Some(x) = None else { return; }; }").is_empty());
    assert!(violations("fn f() { let x = r#\"else {}\"#; /* else */ }").is_empty());
    assert!(violations("impl X { fn f() { if true { loop { break; } } } }").is_empty());
    assert!(!violations("#[cfg(windows)] fn f() { if true { loop { if true {} } } }").is_empty());
}
