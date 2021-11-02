use crate::surf::diag::ErrInfo;
use rowscript_core::basis::data::Ident;
use rowscript_core::presyntax::data::Term::{Let, Unit, Var};
use rowscript_core::presyntax::data::{QualifiedType, Scheme, Term, Type};
use tree_sitter::{Language, Node, Parser, Tree};

mod diag;

#[cfg(test)]
mod tests;

extern "C" {
    fn tree_sitter_rowscript() -> Language;
}

/// Surface syntax.
#[derive(Debug)]
pub struct Surf {
    src: String,
    pub tree: Tree,
}

impl Surf {
    pub fn new(src: String) -> Result<Surf, String> {
        let mut parser = Parser::new();
        let lang = unsafe { tree_sitter_rowscript() };
        parser.set_language(lang).unwrap();

        match parser.parse(&src, None) {
            Some(tree) => {
                let node = tree.root_node();
                if node.has_error() {
                    // TODO: Better error diagnostics.
                    return Err(ErrInfo::new_string(&node, "syntax error"));
                }
                Ok(Surf { src, tree })
            }
            None => Err("parse error".to_string()),
        }
    }

    fn text(&self, node: &Node) -> String {
        self.src[node.start_byte()..node.end_byte()].into()
    }

    pub fn to_presyntax(&self) -> Term {
        self.program(self.tree.root_node())
    }

    fn program(&self, node: Node) -> Term {
        node.children(&mut node.walk())
            .map(|n| self.declaration(n))
            .reduce(|a, b| match a {
                Let(name, typ, exp, _) => Let(name, typ, exp, Box::from(b)),
                _ => unreachable!(),
            })
            .unwrap_or(Unit)
    }

    fn declaration(&self, node: Node) -> Term {
        let decl = node.child(0).unwrap();
        match decl.kind() {
            "functionDeclaration" => self.fn_decl(decl),
            _ => unimplemented!(),
        }
    }

    fn fn_decl(&self, node: Node) -> Term {
        let name = node.child_by_field_name("name").unwrap();

        let mut scheme = Scheme {
            type_vars: vec![],
            row_vars: vec![],
            qualified: QualifiedType {
                preds: vec![],
                typ: Type::Unit,
            },
        };

        if let Some(s) = node.child_by_field_name("scheme") {
            // TODO: Determine type/row variables.
            scheme.type_vars = s
                .named_children(&mut s.walk())
                .map(|n| Ident {
                    pt: n.start_position(),
                    text: self.text(&n),
                })
                .collect();
        }

        Let(
            Ident {
                pt: name.start_position(),
                text: self.text(&name),
            },
            scheme,
            // TODO
            Box::new(Var(Ident {
                pt: node.start_position(),
                text: "x".into(),
            })),
            Box::from(Unit),
        )
    }

    fn decl_sig(&self, node: Node) -> Type {
        // TODO
        unimplemented!()
    }
}
