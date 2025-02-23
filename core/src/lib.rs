use std::fs::read_to_string;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use ariadne::{Color, Label, Report, ReportKind, Source};
use pest::error::InputLocation;
use pest::Parser;
use pest_derive::Parser;
use thiserror::Error;

use crate::codegen::{Codegen, Target};
use crate::theory::abs::builtin::all_builtins;
use crate::theory::abs::data::Term;
use crate::theory::abs::def::Def;
use crate::theory::conc::elab::Elaborator;
use crate::theory::conc::load::{prelude_path, Import, Loaded, ModuleID};
use crate::theory::conc::resolve::{NameMap, ResolvedVar, Resolver, VarKind};
use crate::theory::conc::trans::Trans;
use crate::theory::Loc;

pub mod codegen;
#[cfg(test)]
mod tests;
pub mod theory;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error")]
    IO(#[from] io::Error),
    #[error("parse error")]
    Parsing(#[from] Box<pest::error::Error<Rule>>),

    #[error("unresolved variable")]
    UnresolvedVar(Loc),
    #[error("duplicate name")]
    DuplicateName(Loc),

    #[error("unresolved implicit parameter \"{0}\"")]
    UnresolvedImplicitParam(String, Loc),
    #[error("expected function type, got \"{0}\"")]
    ExpectedPi(Term, Loc),
    #[error("expected tuple type, got \"{0}\"")]
    ExpectedSigma(Term, Loc),
    #[error("expected object type, got \"{0}\"")]
    ExpectedObject(Term, Loc),
    #[error("expected enum type, got \"{0}\"")]
    ExpectedEnum(Term, Loc),
    #[error("fields not known yet, got \"{0}\"")]
    FieldsUnknown(Term, Loc),
    #[error("expected class type, got \"{0}\"")]
    ExpectedClass(Term, Loc),
    #[error("not exhaustive, got \"{0}\"")]
    NonExhaustive(Term, Loc),
    #[error("unresolved field \"{0}\" in \"{1}\"")]
    UnresolvedField(String, Term, Loc),
    #[error("expected interface type, got \"{0}\"")]
    ExpectedInterface(Term, Loc),
    #[error("expected type alias, got \"{0}\"")]
    ExpectedAlias(Term, Loc),
    #[error("unresolved implementation, got \"{0}\"")]
    UnresolvedImplementation(Term, Loc),
    #[error("expected constraint, got \"{0}\"")]
    ExpectedImplementsOf(Term, Loc),

    #[error("expected \"{0}\", found \"{1}\"")]
    NonUnifiable(Term, Term, Loc),
    #[error("field(s) \"{0}\" not contained in \"{1}\"")]
    NonRowSat(Term, Term, Loc),

    #[error("unsolved meta \"{0}\"")]
    UnsolvedMeta(Term, Loc),
    #[error("not erasable term \"{0}\"")]
    NonErasable(Term, Loc),

    #[cfg(test)]
    #[error("codegen error")]
    CodegenTest,
}

const PARSER_FAILED: &str = "failed while parsing";
const RESOLVER_FAILED: &str = "failed while resolving";
const CHECKER_FAILED: &str = "failed while typechecking";
const UNIFIER_FAILED: &str = "failed while unifying";
const CODEGEN_FAILED: &str = "failed while generating code";

fn print_err<S: AsRef<str>>(e: Error, file: &Path, source: S) -> Error {
    fn simple_message<'a>(
        e: &Error,
        loc: &Loc,
        msg: &'a str,
    ) -> (Range<usize>, &'a str, Option<String>) {
        (loc.start..loc.end, msg, Some(e.to_string()))
    }

    use Error::*;

    let (range, title, msg) = match &e {
        IO(_) => (Range::default(), PARSER_FAILED, None),
        Parsing(e) => {
            let range = match e.location {
                InputLocation::Pos(start) => start..source.as_ref().len(),
                InputLocation::Span((start, end)) => start..end,
            };
            (range, PARSER_FAILED, Some(e.variant.message().to_string()))
        }

        UnresolvedVar(loc) => simple_message(&e, loc, RESOLVER_FAILED),
        DuplicateName(loc) => simple_message(&e, loc, RESOLVER_FAILED),

        UnresolvedImplicitParam(_, loc) => simple_message(&e, loc, CHECKER_FAILED),
        ExpectedPi(_, loc) => simple_message(&e, loc, CHECKER_FAILED),
        ExpectedSigma(_, loc) => simple_message(&e, loc, CHECKER_FAILED),
        ExpectedObject(_, loc) => simple_message(&e, loc, CHECKER_FAILED),
        ExpectedEnum(_, loc) => simple_message(&e, loc, CHECKER_FAILED),
        FieldsUnknown(_, loc) => simple_message(&e, loc, CHECKER_FAILED),
        ExpectedClass(_, loc) => simple_message(&e, loc, CHECKER_FAILED),
        NonExhaustive(_, loc) => simple_message(&e, loc, CHECKER_FAILED),
        UnresolvedField(_, _, loc) => simple_message(&e, loc, CHECKER_FAILED),
        ExpectedInterface(_, loc) => simple_message(&e, loc, CHECKER_FAILED),
        ExpectedAlias(_, loc) => simple_message(&e, loc, CHECKER_FAILED),
        UnresolvedImplementation(_, loc) => simple_message(&e, loc, CHECKER_FAILED),
        ExpectedImplementsOf(_, loc) => simple_message(&e, loc, CHECKER_FAILED),

        NonUnifiable(_, _, loc) => simple_message(&e, loc, UNIFIER_FAILED),
        NonRowSat(_, _, loc) => simple_message(&e, loc, UNIFIER_FAILED),

        UnsolvedMeta(_, loc) => simple_message(&e, loc, CODEGEN_FAILED),
        NonErasable(_, loc) => simple_message(&e, loc, CODEGEN_FAILED),

        #[cfg(test)]
        CodegenTest => (Default::default(), CODEGEN_FAILED, None),
    };
    let file_str = file.to_str().unwrap();
    let mut b = Report::build(ReportKind::Error, file_str, range.start)
        .with_message(title)
        .with_code(1);
    if let Some(m) = msg {
        b = b.with_label(
            Label::new((file_str, range))
                .with_message(m)
                .with_color(Color::Red),
        );
    }
    b.finish()
        .print((file_str, Source::from(source.as_ref())))
        .unwrap();
    e
}

#[derive(Parser)]
#[grammar = "theory/surf.pest"]
pub struct RowsParser;

pub const OUTDIR: &str = "dist";
pub const FILE_EXT: &str = "rows";

pub struct ModuleFile {
    file: PathBuf,
    imports: Vec<Import>,
    defs: Vec<Def<Term>>,
}

pub struct Module {
    module: ModuleID,
    files: Vec<ModuleFile>,
    includes: Vec<PathBuf>,
}

pub struct Driver {
    path: PathBuf,
    trans: Trans,
    builtins: NameMap,
    loaded: Loaded,
    elab: Elaborator,
    codegen: Codegen,
}

enum Loadable {
    ViaID(ModuleID),
    ViaPath(PathBuf),
}

impl Driver {
    pub fn new(path: PathBuf, target: Box<dyn Target>) -> Self {
        let codegen = Codegen::new(target, path.join(OUTDIR));
        Self {
            path,
            trans: Default::default(),
            builtins: Default::default(),
            loaded: Default::default(),
            elab: Default::default(),
            codegen,
        }
    }

    pub fn run(&mut self) -> Result<(), Error> {
        for def in all_builtins() {
            self.builtins.insert(
                def.name.to_string(),
                ResolvedVar(VarKind::InModule, def.name.clone()),
            );
            self.elab.sigma.insert(def.name.clone(), def);
        }
        self.load(Loadable::ViaPath(prelude_path()), true)?;
        self.load_module(ModuleID::default())
    }

    fn load_module(&mut self, module: ModuleID) -> Result<(), Error> {
        match self.loaded.contains(&module) {
            true => Ok(()),
            false => self.load(Loadable::ViaID(module), false),
        }
    }

    fn load(&mut self, loadable: Loadable, is_builtin: bool) -> Result<(), Error> {
        use Loadable::*;

        let mut files = Vec::default();
        let mut includes = Vec::default();

        let (path, module) = match loadable {
            ViaID(m) => (m.to_source_path(&self.path), Some(m)),
            ViaPath(p) => (p, None),
        };

        for r in path.read_dir()? {
            let entry = r?;
            if entry.file_type()?.is_dir() {
                continue;
            }
            let file = entry.path();
            match file.extension() {
                None => continue,
                Some(e) => {
                    if self.codegen.should_include(&file) {
                        includes.push(file.clone());
                        continue;
                    }

                    if e != FILE_EXT {
                        continue;
                    }

                    let src = read_to_string(&file)?;
                    let (imports, defs) = self
                        .load_src(&module, src.as_str(), is_builtin)
                        .map_err(|e| print_err(e, &file, src))?;
                    files.push(ModuleFile {
                        file,
                        imports,
                        defs,
                    });
                }
            }
        }

        if let Some(module) = module {
            self.codegen.module(
                &self.elab.sigma,
                Module {
                    module,
                    files,
                    includes,
                },
            )?;
        }

        Ok(())
    }

    fn load_src(
        &mut self,
        module: &Option<ModuleID>,
        src: &str,
        is_builtin: bool,
    ) -> Result<(Vec<Import>, Vec<Def<Term>>), Error> {
        let (mut imports, defs) = RowsParser::parse(Rule::file, src)
            .map_err(Box::new)
            .map_err(Error::from)
            .map(|p| self.trans.file(p))?;
        imports.iter().fold(Ok(()), |r, i| {
            r.and_then(|_| self.load_module(i.module.clone()))
        })?;
        let defs = Resolver::new(&self.builtins, &self.loaded)
            .file(&mut imports, defs)
            .and_then(|d| self.elab.defs(d))?;
        for d in &defs {
            if is_builtin {
                self.builtins.insert(
                    d.name.to_string(),
                    ResolvedVar(VarKind::InModule, d.name.clone()),
                );
            }
            match module {
                Some(m) if !d.is_private() => self.loaded.insert(m, d)?,
                _ => {}
            }
        }
        Ok((imports, defs))
    }
}

const DEFAULT_RED_ZONE: usize = 512 * 1024;
const DEFAULT_EXTRA_STACK: usize = 4 * 1024 * 1024;

pub fn maybe_grow<R, F: FnOnce() -> R>(f: F) -> R {
    stacker::maybe_grow(DEFAULT_RED_ZONE, DEFAULT_EXTRA_STACK, f)
}
