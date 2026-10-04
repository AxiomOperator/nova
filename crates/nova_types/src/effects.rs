//! The effect hierarchy (docs/spec/effects.md §3) and operation lookup (§5.2).

use std::collections::{BTreeSet, HashMap};

use nova_diag::{Diagnostic, Span};
use nova_syntax::ast::{EffectDecl, EffectMember};

#[derive(Debug)]
pub struct OpInfo {
    pub name: String,
    pub params: Vec<String>,
    pub span: Span,
}

#[derive(Debug)]
pub struct EffectNode {
    /// Dotted path from the root, e.g. `fs.read`.
    pub path: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Non-empty only for leaf nodes.
    pub ops: Vec<OpInfo>,
    pub span: Span,
    pub builtin: bool,
}

pub enum OpLookup {
    Found { leaf: usize, op: usize },
    Unknown,
    Ambiguous(Vec<String>),
}

#[derive(Debug, Default)]
pub struct EffectTree {
    pub nodes: Vec<EffectNode>,
    by_path: HashMap<String, usize>,
}

impl EffectTree {
    pub fn add_decl(&mut self, decl: &EffectDecl, parent: Option<usize>, builtin: bool, diags: &mut Vec<Diagnostic>) {
        let path = match parent {
            Some(p) => format!("{}.{}", self.nodes[p].path, decl.name.name),
            None => decl.name.name.clone(),
        };
        if path == "ffi" {
            diags.push(
                Diagnostic::error("E0413", "the effect name `ffi` is reserved")
                    .primary(decl.name.span, "reserved for foreign-function calls"),
            );
            return;
        }
        if let Some(&existing) = self.by_path.get(&path) {
            diags.push(
                Diagnostic::error("E0423", format!("effect `{path}` is declared more than once"))
                    .primary(decl.name.span, "duplicate declaration")
                    .secondary(self.nodes[existing].span, "first declared here"),
            );
            return;
        }
        let idx = self.nodes.len();
        self.nodes.push(EffectNode {
            path: path.clone(),
            parent,
            children: Vec::new(),
            ops: Vec::new(),
            span: decl.name.span,
            builtin,
        });
        self.by_path.insert(path.clone(), idx);
        if let Some(p) = parent {
            self.nodes[p].children.push(idx);
        }

        let mut has_ops = false;
        let mut has_children = false;
        for member in &decl.members {
            match member {
                EffectMember::Effect(child) => {
                    has_children = true;
                    self.add_decl(child, Some(idx), builtin, diags);
                }
                EffectMember::Op(op) => {
                    has_ops = true;
                    if self.nodes[idx].ops.iter().any(|o| o.name == op.name.name) {
                        diags.push(
                            Diagnostic::error(
                                "E0416",
                                format!("operation `{}` is declared twice in `{path}`", op.name.name),
                            )
                            .primary(op.name.span, "duplicate operation"),
                        );
                        continue;
                    }
                    self.nodes[idx].ops.push(OpInfo {
                        name: op.name.name.clone(),
                        params: op.params.iter().map(|p| p.name.name.clone()).collect(),
                        span: op.name.span,
                    });
                }
            }
        }
        if has_ops && has_children {
            diags.push(
                Diagnostic::error("E0405", format!("effect `{path}` mixes operations and nested effects"))
                    .primary(decl.name.span, "an effect contains either operations or nested effects")
                    .help("move the operations into a nested effect"),
            );
        } else if !has_ops && !has_children {
            diags.push(
                Diagnostic::error("E0405", format!("effect `{path}` is empty"))
                    .primary(decl.name.span, "declare at least one operation"),
            );
        }
    }

    pub fn lookup(&self, path: &str) -> Option<usize> {
        self.by_path.get(path).copied()
    }

    pub fn root(&self, name: &str) -> Option<usize> {
        self.lookup(name).filter(|&i| self.nodes[i].parent.is_none())
    }

    pub fn child(&self, node: usize, name: &str) -> Option<usize> {
        self.lookup(&format!("{}.{name}", self.nodes[node].path))
    }

    /// Leaf paths under `node` (a leaf's leaf set is itself).
    pub fn leaves(&self, node: usize) -> Vec<String> {
        let n = &self.nodes[node];
        if n.children.is_empty() {
            vec![n.path.clone()]
        } else {
            n.children.iter().flat_map(|&c| self.leaves(c)).collect()
        }
    }

    /// `(leaf, op index)` for every operation in the subtree of `node`.
    pub fn ops_in_subtree(&self, node: usize) -> Vec<(usize, usize)> {
        let n = &self.nodes[node];
        if n.children.is_empty() {
            (0..n.ops.len()).map(|i| (node, i)).collect()
        } else {
            n.children.iter().flat_map(|&c| self.ops_in_subtree(c)).collect()
        }
    }

    /// Finds the unique operation named `name` in the subtree of `node` (§5.2).
    pub fn resolve_op(&self, node: usize, name: &str) -> OpLookup {
        let found: Vec<(usize, usize)> =
            self.ops_in_subtree(node).into_iter().filter(|&(leaf, op)| self.nodes[leaf].ops[op].name == name).collect();
        match found.as_slice() {
            [] => OpLookup::Unknown,
            [(leaf, op)] => OpLookup::Found { leaf: *leaf, op: *op },
            many => OpLookup::Ambiguous(many.iter().map(|&(l, _)| format!("{}.{name}", self.nodes[l].path)).collect()),
        }
    }

    /// Displays a leaf set compactly: complete subtrees collapse to their parent (§4.2).
    pub fn compact(&self, leaves: &BTreeSet<String>) -> Vec<String> {
        let mut out = Vec::new();
        let mut roots: Vec<usize> = (0..self.nodes.len()).filter(|&i| self.nodes[i].parent.is_none()).collect();
        roots.sort_by(|&a, &b| self.nodes[a].path.cmp(&self.nodes[b].path));
        for root in roots {
            self.compact_into(root, leaves, &mut out);
        }
        let known: BTreeSet<String> = (0..self.nodes.len()).flat_map(|i| self.leaves(i)).collect();
        out.extend(leaves.iter().filter(|l| !known.contains(*l)).cloned());
        out.sort();
        out
    }

    fn compact_into(&self, node: usize, leaves: &BTreeSet<String>, out: &mut Vec<String>) {
        let all = self.leaves(node);
        if all.iter().all(|l| leaves.contains(l)) {
            out.push(self.nodes[node].path.clone());
        } else {
            for &c in &self.nodes[node].children {
                self.compact_into(c, leaves, out);
            }
        }
    }

    pub fn display(&self, leaves: &BTreeSet<String>) -> String {
        self.compact(leaves).join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_diag::FileId;
    use nova_syntax::ast::Item;

    fn tree(src: &str) -> (EffectTree, Vec<Diagnostic>) {
        let (file, parse_diags) = nova_syntax::parse_file(FileId(0), src);
        assert!(parse_diags.is_empty(), "{parse_diags:?}");
        let mut t = EffectTree::default();
        let mut diags = Vec::new();
        for item in &file.items {
            if let Item::Effect(e) = item {
                t.add_decl(e, None, false, &mut diags);
            }
        }
        (t, diags)
    }

    const KV: &str = "effect kv {\n  effect read {\n    fn get(k: String) -> String?\n  }\n  effect write {\n    fn put(k: String, v: String)\n    fn delete(k: String)\n  }\n}\n";

    #[test]
    fn leaves_and_short_op_resolution() {
        let (t, diags) = tree(KV);
        assert!(diags.is_empty());
        let kv = t.root("kv").unwrap();
        assert_eq!(t.leaves(kv), vec!["kv.read", "kv.write"]);
        let OpLookup::Found { leaf, .. } = t.resolve_op(kv, "delete") else { panic!() };
        assert_eq!(t.nodes[leaf].path, "kv.write");
        assert!(matches!(t.resolve_op(kv, "nope"), OpLookup::Unknown));
    }

    #[test]
    fn ambiguous_op() {
        let (t, _) = tree("effect a {\n  effect x {\n    fn go()\n  }\n  effect y {\n    fn go()\n  }\n}\n");
        let OpLookup::Ambiguous(paths) = t.resolve_op(t.root("a").unwrap(), "go") else { panic!() };
        assert_eq!(paths, vec!["a.x.go", "a.y.go"]);
    }

    #[test]
    fn compaction() {
        let (t, _) = tree(KV);
        let both: BTreeSet<String> = ["kv.read".to_string(), "kv.write".to_string()].into();
        assert_eq!(t.compact(&both), vec!["kv"]);
        let one: BTreeSet<String> = ["kv.write".to_string()].into();
        assert_eq!(t.compact(&one), vec!["kv.write"]);
    }

    #[test]
    fn mixed_and_empty_effects_rejected() {
        let (_, diags) = tree("effect m {\n  fn op()\n  effect inner {\n    fn x()\n  }\n}\neffect e {\n}\n");
        let codes: Vec<_> = diags.iter().map(|d| d.code).collect();
        assert_eq!(codes, vec!["E0405", "E0405"]);
    }
}
