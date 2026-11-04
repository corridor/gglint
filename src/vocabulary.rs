//! The words a codebase defines for its domain, which the business tier of the diagrams
//! is drawn from: the enums its graphs decide between, the schemas of their state and of
//! their models' structured output, and what the project says about the functions its
//! graphs call.
//!
//! - **Enums** (`class Disposition(str, Enum)`) and **schemas** (Pydantic models and
//!   `TypedDict`s, with each field's annotation and `Field(description=...)`) are read
//!   from the files that build a graph, where the walk has just been done.
//! - **Declarations** are what a function says about itself: its docstring, and the
//!   GGX declaration in the comment above it, which names it and says what it is:
//!
//!   ```python
//!   # @ggx.Rag.declare(
//!   #     name='Read Schmitt-Thompson Protocol Document',
//!   #     group='Ask Hoag',
//!   # )
//!   def read_schmitt_thompson_protocol_document(protocol_name, patient_category): ...
//!   ```
//!
//!   They're read from the files that build a graph while they're walked, and from the
//!   modules their nodes call by [`called_declarations`], only when the business tier is
//!   drawn.

use ruff_python_ast::{self as ast, Expr, PySourceType, Stmt};
use ruff_python_parser::parse_unchecked_source;
use ruff_text_size::{Ranged, TextSize};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::checker::{CandidateKind, Checker};
use crate::discovery::{FileResult, LineIndex, Location};
use crate::flow::{self, Step};

/// The domain words of one file.
#[derive(Debug, Default)]
pub struct Vocabulary {
    pub enums: Vec<Enum>,
    pub schemas: Vec<Schema>,
    pub declarations: Vec<Declaration>,
}

/// `class Disposition(str, Enum): EMERGENCY = "EMERGENCY"`
#[derive(Debug)]
pub struct Enum {
    pub name: String,
    /// Each member's name, and its value when that's a string.
    pub members: Vec<(String, Option<String>)>,
    pub location: Location,
}

/// A Pydantic model or a `TypedDict`.
#[derive(Debug)]
pub struct Schema {
    pub name: String,
    pub doc: Option<String>,
    pub fields: Vec<Field>,
    pub location: Location,
}

#[derive(Debug)]
pub struct Field {
    pub name: String,
    /// As written: `t.Optional[str]`.
    pub annotation: String,
    /// `Field(description="...")`, when it's a literal.
    pub description: Option<String>,
    pub location: Location,
}

/// A declaration's arguments written as strings or lists of strings, by name, in order.
pub type Fields = Vec<(String, Vec<String>)>;

/// What a module-level function says about itself.
#[derive(Debug)]
pub struct Declaration {
    /// The function's qualified name.
    pub function: String,
    /// The file it's in, relative to the directory it was found under, with `/`
    /// separators.
    pub path: String,
    /// What the GGX declaration above it declares it as: `Pipeline`, `Model`, `Rag`,
    /// `Prompt`, `GlobalFunction`.
    pub kind: Option<String>,
    /// The declaration's arguments written as strings or lists of strings, in order.
    pub fields: Fields,
    /// The first paragraph of its docstring.
    pub doc: Option<String>,
    pub location: Location,
    /// Where the function's body ends, to find the graphs built in it.
    pub end: Location,
}

impl Declaration {
    /// The value of a field that is one string: `name`, `usecase_type`.
    pub fn field(&self, key: &str) -> Option<&str> {
        match self.fields.iter().find(|(field, _)| field == key) {
            Some((_, values)) if values.len() == 1 => Some(&values[0]),
            _ => None,
        }
    }
}

const ENUMS: &[&str] = &["enum.Enum", "enum.StrEnum", "enum.IntEnum"];
const SCHEMAS: &[&str] = &[
    "pydantic.BaseModel",
    "pydantic.main.BaseModel",
    "typing.TypedDict",
    "typing_extensions.TypedDict",
];
const FIELDS: &[&str] = &["pydantic.Field", "pydantic.fields.Field"];

/// The enums and schemas of a file that builds graphs, and the declarations of the
/// functions the graphs are built in, from a checker run with references. `graphs` are
/// the lines the graphs are built at; `path` is the file's, with `/` separators.
pub(crate) fn extract(
    checker: &Checker<'_>,
    body: &[Stmt],
    source: &str,
    lines: &LineIndex,
    path: &str,
    graphs: &[usize],
) -> Vocabulary {
    let module = checker.semantic.module_path().join(".");
    let mut vocabulary = Vocabulary {
        declarations: declarations(body, source, lines, &module, path, Some(graphs)),
        ..Vocabulary::default()
    };
    let mut seen: FxHashSet<TextSize> = FxHashSet::default();
    for candidate in &checker.candidates {
        let (CandidateKind::Base(class), Some(base)) = (&candidate.kind, &candidate.qualified_name) else {
            continue;
        };
        let is_enum = flow::matches(base, ENUMS);
        if !is_enum && !flow::matches(base, SCHEMAS) || !seen.insert(class.name.range.start()) {
            continue;
        }
        let location = lines.location(class.name.range.start());
        if is_enum {
            vocabulary.enums.push(Enum {
                name: class.name.to_string(),
                members: members(class),
                location,
            });
        } else {
            vocabulary.schemas.push(Schema {
                name: class.name.to_string(),
                doc: docstring(&class.body),
                fields: fields(checker, class, source, lines),
                location,
            });
        }
    }
    vocabulary
}

fn members(class: &ast::StmtClassDef) -> Vec<(String, Option<String>)> {
    class
        .body
        .iter()
        .filter_map(|stmt| {
            let Stmt::Assign(assign) = stmt else { return None };
            let [Expr::Name(name)] = assign.targets.as_slice() else {
                return None;
            };
            if name.id.starts_with('_') {
                return None;
            }
            let value = match assign.value.as_ref() {
                Expr::StringLiteral(literal) => Some(literal.value.to_str().to_string()),
                _ => None,
            };
            Some((name.id.to_string(), value))
        })
        .collect()
}

fn fields(checker: &Checker<'_>, class: &ast::StmtClassDef, source: &str, lines: &LineIndex) -> Vec<Field> {
    class
        .body
        .iter()
        .filter_map(|stmt| {
            let Stmt::AnnAssign(assign) = stmt else { return None };
            let Expr::Name(name) = assign.target.as_ref() else {
                return None;
            };
            let description = match assign.value.as_deref() {
                Some(Expr::Call(call))
                    if checker
                        .resolve_recorded(&call.func)
                        .is_some_and(|name| flow::matches(&name, FIELDS)) =>
                {
                    call.arguments
                        .find_keyword("description")
                        .and_then(|keyword| match &keyword.value {
                            Expr::StringLiteral(literal) => Some(literal.value.to_str().to_string()),
                            _ => None,
                        })
                }
                _ => None,
            };
            Some(Field {
                name: name.id.to_string(),
                annotation: one_line(&source[assign.annotation.range()]),
                description,
                location: lines.location(name.range.start()),
            })
        })
        .collect()
}

/// The first paragraph of a body's docstring, on one line.
fn docstring(body: &[Stmt]) -> Option<String> {
    let Some(Stmt::Expr(ast::StmtExpr { value, .. })) = body.first() else {
        return None;
    };
    let Expr::StringLiteral(literal) = value.as_ref() else {
        return None;
    };
    let text = literal.value.to_str();
    let paragraph = text.trim().split("\n\n").next().unwrap_or("");
    let paragraph = one_line(paragraph);
    (!paragraph.is_empty()).then_some(paragraph)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Declarations
// ---------------------------------------------------------------------------

/// What the functions the graphs call in other modules declare: read from those modules
/// only when they're asked for, so a scan that doesn't draw the business tier doesn't
/// read them. The files that build a graph have their declarations already.
pub(crate) fn called_declarations(results: &[FileResult]) -> Vec<Declaration> {
    // Modules by every suffix of their name, as `flow::link` finds them.
    let mut modules: FxHashMap<&str, usize> = FxHashMap::default();
    for (index, result) in results.iter().enumerate() {
        for suffix in flow::suffixes(&result.module) {
            modules.entry(suffix).or_insert(index);
        }
    }
    let mut wanted: Vec<usize> = Vec::new();
    let steps = results
        .iter()
        .flat_map(|result| &result.graphs)
        .flat_map(|graph| &graph.nodes)
        .flat_map(|node| &node.steps);
    for step in steps {
        if let Step::Call { name, .. } = step {
            let module = name.rsplit_once('.').map_or("", |(module, _)| module);
            if let Some(&file) = modules.get(module) {
                if results[file].graphs.is_empty() {
                    wanted.push(file);
                }
            }
        }
    }
    wanted.sort_unstable();
    wanted.dedup();
    let mut found = Vec::new();
    for file in wanted {
        let result = &results[file];
        let Ok(source) = std::fs::read_to_string(&result.path) else {
            continue;
        };
        let parsed = parse_unchecked_source(&source, PySourceType::Python);
        let lines = LineIndex::new(&source);
        let path = crate::graph::slash_path(&result.relative_path);
        found.extend(declarations(
            &parsed.syntax().body,
            &source,
            &lines,
            &result.module,
            &path,
            None,
        ));
    }
    found
}

/// The module-level functions of a file with a docstring or a GGX declaration; when
/// `within` is given, only those containing one of its lines.
fn declarations(
    body: &[Stmt],
    source: &str,
    lines: &LineIndex,
    module: &str,
    path: &str,
    within: Option<&[usize]>,
) -> Vec<Declaration> {
    let mut found = Vec::new();
    for stmt in body {
        let Stmt::FunctionDef(function) = stmt else { continue };
        let start = lines.location(function.range.start());
        let end = lines.location(function.range.end());
        if within.is_some_and(|within| !within.iter().any(|&line| start.line <= line && line <= end.line)) {
            continue;
        }
        let (kind, fields) = ggx_declaration(source, lines, start.line).unwrap_or_default();
        let doc = docstring(&function.body);
        if kind.is_none() && doc.is_none() {
            continue;
        }
        let function_name = if module.is_empty() {
            format!("{path}:{}", function.name)
        } else {
            format!("{module}.{}", function.name)
        };
        found.push(Declaration {
            function: function_name,
            path: path.to_string(),
            kind,
            fields,
            doc,
            location: start,
            end,
        });
    }
    found
}

/// The GGX declaration in the comment lines above the line `line`, if any: its kind and
/// the arguments written as strings or lists of strings.
fn ggx_declaration(source: &str, lines: &LineIndex, line: usize) -> Option<(Option<String>, Fields)> {
    // The comment block directly above the `def` and its decorators, by 1-based line.
    let mut first = line;
    while first > 1 && lines.line(source, first - 1).trim_start().starts_with('#') {
        first -= 1;
    }
    let block: Vec<&str> = (first..line).map(|line| lines.line(source, line)).collect();
    let opening = block
        .iter()
        .position(|line| line.contains("@ggx.") && line.contains(".declare("))?;
    let kind = block[opening]
        .split("@ggx.")
        .nth(1)?
        .split(".declare(")
        .next()?
        .to_string();
    let mut fields = Vec::new();
    for line in &block[opening + 1..] {
        let line = line.trim_start().trim_start_matches('#').trim();
        if line.starts_with(')') {
            break;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        if let Some(values) = strings(value.trim().trim_end_matches(',')) {
            if !values.is_empty() {
                fields.push((key.to_string(), values));
            }
        }
    }
    Some((Some(kind), fields))
}

/// `'text'` or `['a', "b"]` → the strings; anything else, such as `None` or a dict, →
/// `None`.
fn strings(value: &str) -> Option<Vec<String>> {
    if let Some(list) = value.strip_prefix('[').and_then(|value| value.strip_suffix(']')) {
        return list
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(|item| quoted(item).map(str::to_string))
            .collect();
    }
    let text = quoted(value)?;
    Some(if text.is_empty() {
        Vec::new()
    } else {
        vec![text.to_string()]
    })
}

fn quoted(value: &str) -> Option<&str> {
    let quote = value.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    value.strip_prefix(quote)?.strip_suffix(quote)
}

#[cfg(test)]
mod tests {
    use ruff_python_ast::PySourceType;

    use super::*;
    use crate::checker::check_module_with_references;

    fn vocabulary(source: &str) -> Vocabulary {
        let parsed = parse_unchecked_source(source, PySourceType::Python);
        let checker = check_module_with_references(parsed.syntax(), vec!["app"]);
        extract(
            &checker,
            &parsed.syntax().body,
            source,
            &LineIndex::new(source),
            "app.py",
            &[],
        )
    }

    #[test]
    fn reads_enums_and_schemas() {
        let vocabulary = vocabulary(
            "from enum import Enum\n\
             import typing as t\n\
             from pydantic import BaseModel, Field\n\
             from typing_extensions import TypedDict\n\
             def pipeline():\n\
             \x20   class Disposition(str, Enum):\n\
             \x20       EMERGENCY = \"EMERGENCY\"\n\
             \x20       VIRTUAL_VISIT = \"Virtual visit\"\n\
             \x20   class Answer(BaseModel):\n\
             \x20       \"\"\"What the patient is told.\n\n\x20       More.\"\"\"\n\
             \x20       response: str = Field(description=\"The patient-facing \" \"message.\")\n\
             \x20       tier: t.Optional[str] = Field(default=None, description=(\"One of: NO_REVIEW, ASYNC_REVIEW\"))\n\
             \x20   class State(TypedDict):\n\
             \x20       disposition: t.Optional[str]\n",
        );
        let [disposition] = vocabulary.enums.as_slice() else {
            panic!("{vocabulary:?}")
        };
        assert_eq!(disposition.name, "Disposition");
        assert_eq!(
            disposition.members,
            [
                ("EMERGENCY".to_string(), Some("EMERGENCY".to_string())),
                ("VIRTUAL_VISIT".to_string(), Some("Virtual visit".to_string())),
            ]
        );
        let [answer, state] = vocabulary.schemas.as_slice() else {
            panic!("{vocabulary:?}")
        };
        assert_eq!(answer.doc.as_deref(), Some("What the patient is told."));
        assert_eq!(
            answer.fields[0].description.as_deref(),
            Some("The patient-facing message.")
        );
        assert_eq!(answer.fields[1].annotation, "t.Optional[str]");
        assert_eq!(
            answer.fields[1].description.as_deref(),
            Some("One of: NO_REVIEW, ASYNC_REVIEW")
        );
        assert_eq!(state.fields[0].name, "disposition");
        assert_eq!(state.fields[0].description, None);
    }

    #[test]
    fn reads_ggx_declarations_and_docstrings() {
        let source = "import pathlib\n\
             \n\
             # @ggx.Rag.declare(\n\
             #     name='Read Protocol Document',\n\
             #     group=None,\n\
             #     data_usage=['Internal Policies/Data', \"Customer Specific Data\"],\n\
             #     personas=[{'startingInput': '.'}],\n\
             # )\n\
             def read_protocol(name):\n\
             \x20   \"\"\"\n\
             \x20   This RAG has access to all the protocols.\n\
             \n\
             \x20   The zip holds one file per protocol.\n\
             \x20   \"\"\"\n\
             \x20   return name\n\
             \n\
             def helper():\n\
             \x20   return 1\n";
        let parsed = parse_unchecked_source(source, PySourceType::Python);
        let lines = LineIndex::new(source);
        let declarations = declarations(&parsed.syntax().body, source, &lines, "rags.read", "rags/read.py", None);
        let [declaration] = declarations.as_slice() else {
            panic!("{declarations:?}")
        };
        assert_eq!(declaration.function, "rags.read.read_protocol");
        assert_eq!(declaration.kind.as_deref(), Some("Rag"));
        assert_eq!(declaration.field("name"), Some("Read Protocol Document"));
        assert_eq!(
            declaration.fields[1],
            (
                "data_usage".to_string(),
                vec![
                    "Internal Policies/Data".to_string(),
                    "Customer Specific Data".to_string()
                ]
            )
        );
        assert_eq!(declaration.fields.len(), 2, "None and dicts are left out");
        assert_eq!(
            declaration.doc.as_deref(),
            Some("This RAG has access to all the protocols.")
        );
        assert_eq!(declaration.location.line, 9);
    }
}
