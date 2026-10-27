//! The semantic model, modelled on `ruff_python_semantic`.
//!
//! The model answers one question cheaply: "what does this name refer to?" It is a
//! few flat arrays:
//!
//! - **Scopes**: one per module, class, function, lambda or comprehension. Each scope
//!   holds a map from a name to the binding that name currently refers to, plus the
//!   ID of its parent scope.
//! - **Bindings**: one per place a name is given a value, such as an import, an
//!   assignment, a `def` or a parameter. A binding records *how* the name was bound
//!   ([`BindingKind`]) and where.
//!
//! Scopes and bindings refer to each other by small integer IDs ([`ScopeId`],
//! [`BindingId`]), which are just positions in those arrays. This is the same "arena"
//! layout Ruff uses: no reference counting, no pointers between nodes, and
//! everything for one file lives in a few contiguous vectors.
//!
//! This module doesn't depend on any language's syntax tree. A language-specific
//! walker (for Python, `checker.rs`) creates the scopes and bindings while it
//! traverses the tree, and rules then query the finished model.

use std::fmt;
use std::marker::PhantomData;

use rustc_hash::FxHashMap;

use ruff_text_size::TextRange;

// ---------------------------------------------------------------------------
// Typed index vectors (Ruff: the `ruff_index` crate)
// ---------------------------------------------------------------------------

/// A type that can be used as a typed index into an [`IndexVec`].
pub trait Idx: Copy + Eq {
    fn new(index: usize) -> Self;
    fn index(self) -> usize;
}

macro_rules! newtype_index {
    ($(#[$attr:meta])* $name:ident) => {
        $(#[$attr])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(u32);

        impl Idx for $name {
            #[expect(clippy::cast_possible_truncation)]
            fn new(index: usize) -> Self {
                assert!(index <= u32::MAX as usize);
                Self(index as u32)
            }

            fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

/// A `Vec` that can only be indexed by its own ID type, so a `BindingId` can never be
/// used to index the scopes by mistake.
#[derive(Debug)]
pub struct IndexVec<I, T> {
    raw: Vec<T>,
    index: PhantomData<I>,
}

impl<I: Idx, T> IndexVec<I, T> {
    pub const fn new() -> Self {
        Self {
            raw: Vec::new(),
            index: PhantomData,
        }
    }

    /// Appends a value and returns its ID.
    pub fn push(&mut self, value: T) -> I {
        let id = I::new(self.raw.len());
        self.raw.push(value);
        id
    }

    pub fn len(&self) -> usize {
        self.raw.len()
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.raw.iter()
    }

    pub fn iter_enumerated(&self) -> impl Iterator<Item = (I, &T)> {
        self.raw
            .iter()
            .enumerate()
            .map(|(index, value)| (I::new(index), value))
    }
}

impl<I: Idx, T> Default for IndexVec<I, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: Idx, T> std::ops::Index<I> for IndexVec<I, T> {
    type Output = T;

    fn index(&self, index: I) -> &T {
        &self.raw[index.index()]
    }
}

impl<I: Idx, T> std::ops::IndexMut<I> for IndexVec<I, T> {
    fn index_mut(&mut self, index: I) -> &mut T {
        &mut self.raw[index.index()]
    }
}

newtype_index! {
    /// The ID of a [`Scope`]. The module scope is always `ScopeId(0)`.
    ScopeId
}

newtype_index! {
    /// The ID of a [`Binding`].
    BindingId
}

impl ScopeId {
    pub const fn global() -> Self {
        Self(0)
    }

    pub const fn is_global(self) -> bool {
        self.0 == 0
    }
}

// ---------------------------------------------------------------------------
// Qualified names
// ---------------------------------------------------------------------------

/// A fully qualified dotted name, such as `pydantic_ai.Agent`, stored as segments.
///
/// Segments borrow from the syntax tree, so building one copies no text. A relative
/// import that hasn't been resolved keeps its leading dots as `"."` segments, as Ruff
/// does (`from .agents import x` → `[".", "agents", "x"]`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QualifiedName<'a>(Vec<&'a str>);

impl<'a> QualifiedName<'a> {
    pub fn from_segments(segments: impl IntoIterator<Item = &'a str>) -> Self {
        Self(segments.into_iter().collect())
    }

    /// Splits a dotted string: `"a.b.c"` → `["a", "b", "c"]`.
    pub fn from_dotted_name(name: &'a str) -> Self {
        Self(name.split('.').collect())
    }

    pub fn segments(&self) -> &[&'a str] {
        &self.0
    }

    /// Whether this name comes from a relative import that couldn't be resolved.
    pub fn is_unresolved_import(&self) -> bool {
        self.0.first() == Some(&".")
    }

    /// Appends attribute segments: `pydantic_ai` + `["Agent"]` → `pydantic_ai.Agent`.
    #[must_use]
    pub fn append(&self, tail: &[&'a str]) -> Self {
        Self(self.0.iter().chain(tail).copied().collect())
    }
}

impl fmt::Display for QualifiedName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for segment in &self.0 {
            if !first && *segment != "." {
                f.write_str(".")?;
            }
            f.write_str(segment)?;
            first = *segment == ".";
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Scopes (Ruff: `scope.rs`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    Module,
    Class,
    Function,
    Lambda,
    /// A comprehension or generator expression, which has its own scope in Python 3.
    Generator,
}

#[derive(Debug)]
pub struct Scope<'a> {
    pub kind: ScopeKind,

    /// The enclosing scope, if any. Only the module scope has none.
    pub parent: Option<ScopeId>,

    /// A map from a name to the binding it currently refers to in this scope.
    ///
    /// When a name is bound again (`x = 1` then `x = 2`), the map points at the newest
    /// binding. The older binding stays in [`SemanticModel::bindings`].
    bindings: FxHashMap<&'a str, BindingId>,

    /// Modules star-imported into this scope (`from x import *`). Names that aren't
    /// found anywhere else may come from one of these.
    star_imports: Vec<StarImport<'a>>,
}

impl<'a> Scope<'a> {
    pub fn new(kind: ScopeKind, parent: Option<ScopeId>) -> Self {
        Self {
            kind,
            parent,
            bindings: FxHashMap::default(),
            star_imports: Vec::new(),
        }
    }

    /// Returns the binding that `name` currently refers to in this scope.
    pub fn get(&self, name: &str) -> Option<BindingId> {
        self.bindings.get(name).copied()
    }

    /// Points `name` at `id`, returning the binding it pointed at before, if any.
    pub fn add(&mut self, name: &'a str, id: BindingId) -> Option<BindingId> {
        self.bindings.insert(name, id)
    }

    /// Iterates over each name and the binding it currently refers to.
    pub fn bindings(&self) -> impl Iterator<Item = (&'a str, BindingId)> + '_ {
        self.bindings.iter().map(|(name, id)| (*name, *id))
    }

    pub fn add_star_import(&mut self, import: StarImport<'a>) {
        self.star_imports.push(import);
    }

    pub fn star_imports(&self) -> &[StarImport<'a>] {
        &self.star_imports
    }
}

/// `from module import *`
#[derive(Clone, Debug)]
pub struct StarImport<'a> {
    pub level: u32,
    pub module: Option<&'a str>,
}

// ---------------------------------------------------------------------------
// Bindings (Ruff: `binding.rs`)
// ---------------------------------------------------------------------------

/// One place where a name is given a value.
#[derive(Debug)]
pub struct Binding<'a> {
    pub kind: BindingKind<'a>,
    /// Where the name appears in the source.
    pub range: TextRange,
    /// The scope the binding belongs to.
    pub scope: ScopeId,
}

/// How a name was bound. This is what lets a rule turn a name back into what it means.
#[derive(Debug)]
pub enum BindingKind<'a> {
    /// `import foo` or `import foo.bar as baz`: the name is the module itself.
    Import(QualifiedName<'a>),

    /// `import foo.bar` without `as`: binds `foo`, but also loads `foo.bar`.
    SubmoduleImport(QualifiedName<'a>),

    /// `from foo import bar` or `from foo import bar as baz`: the name is `foo.bar`.
    FromImport(QualifiedName<'a>),

    /// `x = ...`, `for x in ...`, `with ... as x`, or `x := ...`.
    Assignment,

    /// `x: int` with no value.
    Annotation,

    /// A function or lambda parameter.
    Argument,

    /// `def name(...)`.
    FunctionDefinition,

    /// `class Name: ...`. Holds the class's own scope.
    ClassDefinition(ScopeId),

    /// `except E as name`.
    BoundException,

    /// `global name`: lookups continue in the module scope.
    Global,

    /// `nonlocal name`: lookups continue in the enclosing scopes.
    Nonlocal,
}

// ---------------------------------------------------------------------------
// The model (Ruff: `model.rs`)
// ---------------------------------------------------------------------------

/// The semantic model for one module.
#[derive(Debug)]
pub struct SemanticModel<'a> {
    /// The module's own dotted path, such as `["myapp", "agents"]`, used to give
    /// qualified names to the classes and functions it defines.
    module_path: Vec<&'a str>,

    /// Every scope in the module.
    pub scopes: IndexVec<ScopeId, Scope<'a>>,

    /// The scope the walker is currently in.
    pub scope_id: ScopeId,

    /// Every binding created in any scope, in the order they were created.
    pub bindings: IndexVec<BindingId, Binding<'a>>,
}

/// The walker's position, saved so it can return to it later.
///
/// Ruff's snapshot also saves the current syntax node, branch and flags. Discovery
/// only needs the scope.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    scope_id: ScopeId,
}

impl<'a> SemanticModel<'a> {
    pub fn new(module_path: Vec<&'a str>) -> Self {
        let mut scopes = IndexVec::new();
        let global = scopes.push(Scope::new(ScopeKind::Module, None));
        debug_assert_eq!(global, ScopeId::global());
        Self {
            module_path,
            scopes,
            scope_id: ScopeId::global(),
            bindings: IndexVec::new(),
        }
    }

    pub fn module_path(&self) -> &[&'a str] {
        &self.module_path
    }

    pub fn current_scope(&self) -> &Scope<'a> {
        &self.scopes[self.scope_id]
    }

    pub fn current_scope_mut(&mut self) -> &mut Scope<'a> {
        &mut self.scopes[self.scope_id]
    }

    pub fn global_scope(&self) -> &Scope<'a> {
        &self.scopes[ScopeId::global()]
    }

    pub fn at_top_level(&self) -> bool {
        self.scope_id.is_global()
    }

    /// Enters a new scope nested in the current one.
    pub fn push_scope(&mut self, kind: ScopeKind) -> ScopeId {
        let id = self.scopes.push(Scope::new(kind, Some(self.scope_id)));
        self.scope_id = id;
        id
    }

    /// Returns to the parent of the current scope.
    pub fn pop_scope(&mut self) {
        self.scope_id = self.scopes[self.scope_id]
            .parent
            .expect("attempted to pop the module scope");
    }

    /// Creates a binding for `name` in the current scope.
    pub fn push_binding(&mut self, name: &'a str, range: TextRange, kind: BindingKind<'a>) -> BindingId {
        let id = self.bindings.push(Binding {
            kind,
            range,
            scope: self.scope_id,
        });
        self.current_scope_mut().add(name, id);
        id
    }

    pub fn binding(&self, id: BindingId) -> &Binding<'a> {
        &self.bindings[id]
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            scope_id: self.scope_id,
        }
    }

    pub fn restore(&mut self, snapshot: Snapshot) {
        self.scope_id = snapshot.scope_id;
    }

    /// Iterates from `scope_id` up through its parents to the module scope.
    pub fn ancestor_ids(&self, scope_id: ScopeId) -> impl Iterator<Item = ScopeId> + '_ {
        std::iter::successors(Some(scope_id), |id| self.scopes[*id].parent)
    }

    /// Finds the binding that `name` refers to from the current scope.
    ///
    /// Walks outwards from the current scope, following Python's rules (Ruff:
    /// `SemanticModel::lookup_symbol`):
    /// - Class bodies are only visible from directly inside them. A method can't see
    ///   names defined in its class body without going through `self`.
    /// - `global x` sends the lookup to the module scope, and `nonlocal x` to the
    ///   enclosing scopes.
    pub fn lookup_symbol(&self, name: &str) -> Option<BindingId> {
        let mut seen_function = false;
        for scope_id in self.ancestor_ids(self.scope_id) {
            let scope = &self.scopes[scope_id];
            if scope.kind == ScopeKind::Class && seen_function {
                continue;
            }

            if let Some(binding_id) = scope.get(name) {
                match self.bindings[binding_id].kind {
                    BindingKind::Global => {
                        return self.global_scope().get(name);
                    }
                    BindingKind::Nonlocal => {
                        seen_function = true;
                        continue;
                    }
                    _ => return Some(binding_id),
                }
            }

            seen_function |= matches!(scope.kind, ScopeKind::Function | ScopeKind::Lambda);
        }
        None
    }

    /// Whether any scope visible from here has a star import, which means a name that
    /// wasn't found might still exist.
    pub fn has_star_imports(&self) -> bool {
        self.ancestor_ids(self.scope_id)
            .any(|id| !self.scopes[id].star_imports().is_empty())
    }

    /// Resolves a dotted name, as written at the current position, to its fully
    /// qualified name (Ruff: `SemanticModel::resolve_qualified_name`).
    ///
    /// `name` is the dotted name split into segments: `A` is `["A"]`, and
    /// `crewai.Agent` is `["crewai", "Agent"]`. The first segment is looked up; the
    /// rest are appended to whatever it resolves to:
    ///
    /// ```text
    /// from pydantic_ai import Agent as A    ["A"]              → pydantic_ai.Agent
    /// import crewai                         ["crewai", "Agent"] → crewai.Agent
    /// import google.adk.agents              ["google", "adk", "agents", "LlmAgent"]
    ///                                                           → google.adk.agents.LlmAgent
    /// class Reviewer: ...  (in myapp.bots)  ["Reviewer"]        → myapp.bots.Reviewer
    /// ```
    ///
    /// Returns `None` for names that aren't imports or definitions, such as local
    /// variables and parameters, because their value isn't known without running code.
    pub fn resolve_qualified_name(&self, name: &[&'a str]) -> Option<QualifiedName<'a>> {
        let (head, tail) = name.split_first()?;
        let binding = self.binding(self.lookup_symbol(head)?);

        match &binding.kind {
            BindingKind::Import(qualified_name) | BindingKind::FromImport(qualified_name) => {
                Some(qualified_name.append(tail))
            }
            BindingKind::SubmoduleImport(_) => {
                // `import a.b` binds `a`, so `a.b.c` resolves to exactly `a.b.c`.
                Some(QualifiedName::from_segments(name.iter().copied()))
            }
            BindingKind::ClassDefinition(_) | BindingKind::FunctionDefinition => {
                // Only module-level definitions have a stable qualified name.
                (binding.scope.is_global()).then(|| {
                    QualifiedName::from_segments(
                        self.module_path.iter().copied().chain(name.iter().copied()),
                    )
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_aliases_through_scopes() {
        let mut model = SemanticModel::new(vec!["app"]);
        model.push_binding(
            "A",
            TextRange::default(),
            BindingKind::FromImport(QualifiedName::from_dotted_name("pydantic_ai.Agent")),
        );
        model.push_binding(
            "crewai",
            TextRange::default(),
            BindingKind::Import(QualifiedName::from_dotted_name("crewai")),
        );

        model.push_scope(ScopeKind::Function);
        assert_eq!(
            model.resolve_qualified_name(&["A"]).unwrap().to_string(),
            "pydantic_ai.Agent"
        );
        assert_eq!(
            model.resolve_qualified_name(&["crewai", "Agent"]).unwrap().to_string(),
            "crewai.Agent"
        );

        // A local assignment shadows the import inside the function only.
        model.push_binding("A", TextRange::default(), BindingKind::Assignment);
        assert_eq!(model.resolve_qualified_name(&["A"]), None);
        model.pop_scope();
        assert!(model.resolve_qualified_name(&["A"]).is_some());
    }

    #[test]
    fn methods_cannot_see_class_bodies() {
        let mut model = SemanticModel::new(vec![]);
        model.push_scope(ScopeKind::Class);
        model.push_binding(
            "Agent",
            TextRange::default(),
            BindingKind::FromImport(QualifiedName::from_dotted_name("crewai.Agent")),
        );
        assert!(model.lookup_symbol("Agent").is_some());
        model.push_scope(ScopeKind::Function);
        assert!(model.lookup_symbol("Agent").is_none());
    }

    #[test]
    fn display_relative_names() {
        let name = QualifiedName::from_segments([".", ".", "agents", "make"]);
        assert_eq!(name.to_string(), "..agents.make");
        assert!(name.is_unresolved_import());
    }
}
