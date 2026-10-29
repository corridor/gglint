//! The Python walker that builds the semantic model, modelled on Ruff's `Checker`
//! (`ruff_linter/src/checkers/ast/mod.rs`).
//!
//! Parsing is done by Ruff's own parser (`ruff_python_parser`), and the walk uses Ruff's
//! [`Visitor`], which already knows how to reach every kind of node. The checker only
//! overrides the nodes that matter to the semantic model:
//! - imports, assignments, `def`s, `class`es and parameters create bindings;
//! - `class` bodies, functions, lambdas and comprehensions open new scopes.
//!
//! Everything else falls through to Ruff's `walk_*` functions.
//!
//! Function and lambda bodies are *deferred*: when the walker reaches a `def`, it binds
//! the function's name, saves a snapshot of the current scope, and only visits the body
//! after the rest of the enclosing scope is done. A function body runs when it's called,
//! not where it's written, so it can use names defined further down the file:
//!
//! ```python
//! def build():
//!     return A("gpt-4o")      # `A` is defined below, but this is valid Python
//!
//! from pydantic_ai import Agent as A
//! ```
//!
//! While walking, the checker also records every call, class base and decorator as a
//! [`Candidate`], together with the qualified name it resolved to at that point. Agent
//! discovery then only has to compare those names against a registry.
//!
//! Prompts are found during the walk itself (see `prompts.rs`), because a prompt's
//! text depends on what its variables hold at that point. To make that possible, the
//! checker remembers, for each variable, the text it was assigned, the file path it
//! was assigned (`PROMPTS = Path(__file__).parent / "prompts"`), and the call it was
//! assigned (`client = Anthropic()`, so `client.messages.create` can be recognised).
//! It also remembers what each function returns, so `instruction=build()` can use it.

use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    self as ast, Comprehension, ExceptHandler, Expr, ExprContext, ModModule, Parameters,
    Pattern, Stmt,
};
use ruff_text_size::{Ranged, TextRange, TextSize};
use rustc_hash::FxHashMap;

use crate::prompts::{self, Piece, Prompt, PromptSink, SourcePath};
use crate::semantic::{
    BindingId, BindingKind, QualifiedName, ScopeId, ScopeKind, SemanticModel, Snapshot,
    StarImport,
};

/// A syntax node that might create an agent, found during the walk.
#[derive(Debug)]
pub struct Candidate<'a> {
    pub kind: CandidateKind<'a>,
    /// What the callee, base class or decorator resolved to, if it could be resolved.
    pub qualified_name: Option<QualifiedName<'a>>,
    /// The scope the candidate appeared in.
    pub scope: ScopeId,
    /// For a call that is the whole value of a module-level assignment, the name it's
    /// assigned to: `agent` in `agent = create_agent(...)`. That's the name other code
    /// imports it by.
    pub assigned_to: Option<&'a str>,
}

#[derive(Debug)]
pub enum CandidateKind<'a> {
    /// `X(...)`
    Call(&'a ast::ExprCall),
    /// `class C(X): ...`
    Base(&'a ast::StmtClassDef),
    /// `@X` on a function or class. `function` is set when it decorates a function.
    Decorator {
        expr: &'a Expr,
        function: Option<&'a ast::StmtFunctionDef>,
    },
}

impl Ranged for Candidate<'_> {
    fn range(&self) -> TextRange {
        match &self.kind {
            CandidateKind::Call(call) => call.range(),
            CandidateKind::Base(class) => class.name.range,
            CandidateKind::Decorator { expr, .. } => expr.range(),
        }
    }
}

/// Work postponed until the enclosing scope has been fully visited.
#[derive(Default)]
struct Visit<'a> {
    functions: Vec<(Snapshot, &'a ast::StmtFunctionDef, BindingId)>,
    lambdas: Vec<(Snapshot, &'a ast::ExprLambda)>,
}

pub struct Checker<'a> {
    pub semantic: SemanticModel<'a>,
    pub candidates: Vec<Candidate<'a>>,
    pub prompts: Vec<Prompt<'a>>,
    visit: Visit<'a>,
    /// The name of the module-level assignment whose value is being visited, until the
    /// call that is that value takes it.
    assigned_to: Option<&'a str>,
    /// The text of each variable assigned a string, worked out where it's assigned:
    /// `BASE = "You are..."`, `RULES = BASE + "..."`.
    binding_texts: FxHashMap<BindingId, Vec<Piece<'a>>>,
    /// The call each variable is assigned: `client = Anthropic()`.
    binding_calls: FxHashMap<BindingId, &'a ast::ExprCall>,
    /// Functions whose return value is a prompt, by the start of their name:
    /// `@agent.instructions def f(): ...`.
    prompt_functions: FxHashMap<TextSize, (&'static PromptSink, String)>,
    /// The prompt function whose body is being visited, and its scope.
    prompt_function: Option<(ScopeId, &'static PromptSink, String)>,
    /// The file path each variable is assigned:
    /// `PROMPTS = Path(__file__).parent / "prompts"`.
    binding_paths: FxHashMap<BindingId, SourcePath<'a>>,
    /// The file each `with open(...) as f:` variable reads.
    binding_opened: FxHashMap<BindingId, SourcePath<'a>>,
    /// The text each function returns, if it has exactly one `return` and that
    /// returns a string. `None` once a second `return` is seen.
    function_returns: FxHashMap<BindingId, Option<Vec<Piece<'a>>>>,
    /// The function whose body is being visited, and its scope.
    function: Option<(ScopeId, BindingId)>,
}

/// Builds the semantic model for a module and collects agent candidates.
///
/// `module_path` is the module's dotted path, such as `["myapp", "agents"]`.
pub fn check_module<'a>(module: &'a ModModule, module_path: Vec<&'a str>) -> Checker<'a> {
    let mut checker = Checker {
        semantic: SemanticModel::new(module_path),
        candidates: Vec::new(),
        prompts: Vec::new(),
        visit: Visit::default(),
        assigned_to: None,
        binding_texts: FxHashMap::default(),
        binding_calls: FxHashMap::default(),
        prompt_functions: FxHashMap::default(),
        prompt_function: None,
        binding_paths: FxHashMap::default(),
        binding_opened: FxHashMap::default(),
        function_returns: FxHashMap::default(),
        function: None,
    };
    checker.visit_body(&module.body);
    checker.visit_deferred();
    checker
}

impl<'a> Visitor<'a> for Checker<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        self.assigned_to = if self.semantic.at_top_level() {
            assignment_of_call(stmt)
        } else {
            None
        };
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    let module = alias.name.as_str();
                    if let Some(asname) = &alias.asname {
                        // `import a.b as c` binds `c` to the module `a.b`.
                        self.add_binding(
                            asname.as_str(),
                            asname.range,
                            BindingKind::Import(QualifiedName::from_dotted_name(module)),
                        );
                    } else if let Some((head, _)) = module.split_once('.') {
                        // `import a.b` binds `a`.
                        self.add_binding(
                            head,
                            alias.range,
                            BindingKind::SubmoduleImport(QualifiedName::from_dotted_name(module)),
                        );
                    } else {
                        self.add_binding(
                            module,
                            alias.range,
                            BindingKind::Import(QualifiedName::from_dotted_name(module)),
                        );
                    }
                }
            }
            Stmt::ImportFrom(import) => {
                let module = import.module.as_ref().map(ast::Identifier::as_str);
                for alias in &import.names {
                    let member = alias.name.as_str();
                    if member == "*" {
                        self.semantic.current_scope_mut().add_star_import(StarImport {
                            level: import.level,
                            module,
                        });
                        continue;
                    }

                    // Given `from foo import bar as baz`, the name is `baz` and the
                    // qualified name is `foo.bar`.
                    let (name, range) = match &alias.asname {
                        Some(asname) => (asname.as_str(), asname.range),
                        None => (member, alias.range),
                    };
                    let qualified_name = collect_import_from_member(import.level, module, member);
                    self.add_binding(name, range, BindingKind::FromImport(qualified_name));
                }
            }
            Stmt::FunctionDef(function) => {
                // Decorators, defaults and annotations run where the `def` is written.
                for decorator in &function.decorator_list {
                    self.visit_candidate_decorator(&decorator.expression, Some(function));
                }
                if let Some(type_params) = &function.type_params {
                    self.visit_type_params(type_params);
                }
                self.visit_parameters(&function.parameters);
                if let Some(returns) = &function.returns {
                    self.visit_annotation(returns);
                }

                let binding = self.semantic.push_binding(
                    function.name.as_str(),
                    function.name.range,
                    BindingKind::FunctionDefinition,
                );
                self.visit.functions.push((self.semantic.snapshot(), function, binding));
            }
            Stmt::ClassDef(class) => {
                for decorator in &class.decorator_list {
                    self.visit_candidate_decorator(&decorator.expression, None);
                }
                if let Some(type_params) = &class.type_params {
                    self.visit_type_params(type_params);
                }
                if let Some(arguments) = &class.arguments {
                    for base in &*arguments.args {
                        self.visit_expr(base);
                        let qualified_name = self.resolve_expr(base);
                        self.candidates.push(Candidate {
                            kind: CandidateKind::Base(class),
                            qualified_name,
                            scope: self.semantic.scope_id,
                            assigned_to: None,
                        });
                    }
                    for keyword in &*arguments.keywords {
                        self.visit_keyword(keyword);
                    }
                }

                // The class body runs immediately, in its own scope. The class name is
                // bound only after the body has run, as in Python.
                let class_scope = self.semantic.push_scope(ScopeKind::Class);
                self.visit_body(&class.body);
                self.semantic.pop_scope();
                self.add_binding(
                    class.name.as_str(),
                    class.name.range,
                    BindingKind::ClassDefinition(class_scope),
                );
            }
            Stmt::AnnAssign(ast::StmtAnnAssign {
                target,
                annotation,
                value: None,
                ..
            }) => {
                // `x: int` declares `x` without giving it a value.
                self.visit_annotation(annotation);
                if let Expr::Name(name) = target.as_ref() {
                    self.add_binding(name.id.as_str(), name.range, BindingKind::Annotation);
                } else {
                    self.visit_expr(target);
                }
            }
            Stmt::Assign(_) | Stmt::AnnAssign(_) | Stmt::AugAssign(_) => self.visit_assignment(stmt),
            Stmt::Return(ast::StmtReturn { value, .. }) => {
                self.visit_return(value.as_deref());
                visitor::walk_stmt(self, stmt);
            }
            Stmt::Global(global) => {
                if !self.semantic.at_top_level() {
                    for name in &global.names {
                        self.add_binding(name.as_str(), name.range, BindingKind::Global);
                    }
                }
            }
            Stmt::Nonlocal(nonlocal) => {
                for name in &nonlocal.names {
                    self.add_binding(name.as_str(), name.range, BindingKind::Nonlocal);
                }
            }
            // Python has no block scopes: names bound inside `if`, `for`, `while`, `try`
            // and `with` belong to the enclosing function or module. Ruff's walker visits
            // assigned values before their targets, and every target name reaches
            // `visit_expr` with a `Store` context, which is where it gets bound.
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Call(call) => {
                let qualified_name = self.resolve_expr(&call.func);
                if let Some((sink, via)) = self.prompt_sink(&call.func, qualified_name.as_ref()) {
                    let found = prompts::prompts_of_call(self, sink, via.to_string(), call);
                    self.prompts.extend(found);
                }
                self.candidates.push(Candidate {
                    kind: CandidateKind::Call(call),
                    qualified_name,
                    scope: self.semantic.scope_id,
                    assigned_to: self.assigned_to.take(),
                });
                visitor::walk_expr(self, expr);
            }
            Expr::Name(ast::ExprName {
                id,
                ctx: ExprContext::Store,
                range,
                ..
            }) => {
                self.add_binding(id.as_str(), *range, BindingKind::Assignment);
            }
            Expr::Dict(dict) => {
                if let Some(prompt) = prompts::prompt_of_message_dict(self, dict) {
                    self.prompts.push(prompt);
                }
                visitor::walk_expr(self, expr);
            }
            Expr::Named(named) => {
                self.visit_expr(&named.value);
                // Ruff binds the target in the nearest scope that isn't a comprehension
                // (PEP 572). We bind it in the current scope.
                self.visit_expr(&named.target);
            }
            Expr::Lambda(lambda) => {
                if let Some(parameters) = &lambda.parameters {
                    self.visit_parameters(parameters);
                }
                self.visit.lambdas.push((self.semantic.snapshot(), lambda));
            }
            Expr::ListComp(comprehension) => {
                self.visit_generators(&comprehension.generators, &[comprehension.elt.as_ref()]);
            }
            Expr::SetComp(comprehension) => {
                self.visit_generators(&comprehension.generators, &[comprehension.elt.as_ref()]);
            }
            Expr::Generator(generator) => {
                self.visit_generators(&generator.generators, &[generator.elt.as_ref()]);
            }
            Expr::DictComp(comprehension) => match &comprehension.key {
                Some(key) => {
                    self.visit_generators(&comprehension.generators, &[key.as_ref(), comprehension.value.as_ref()]);
                }
                None => self.visit_generators(&comprehension.generators, &[comprehension.value.as_ref()]),
            },
            _ => visitor::walk_expr(self, expr),
        }
    }

    fn visit_with_item(&mut self, with_item: &'a ast::WithItem) {
        self.visit_expr(&with_item.context_expr);
        let Some(target) = &with_item.optional_vars else {
            return;
        };
        self.visit_expr(target);
        // `with open(path) as f:`, so `f.read()` reads `path`.
        let (Expr::Call(call), Expr::Name(name)) = (&with_item.context_expr, target.as_ref()) else {
            return;
        };
        if let Some(path) = prompts::opened_path(self, call) {
            if let Some(binding) = self.semantic.current_scope().get(name.id.as_str()) {
                self.binding_opened.insert(binding, path);
            }
        }
    }

    fn visit_except_handler(&mut self, except_handler: &'a ExceptHandler) {
        let ExceptHandler::ExceptHandler(handler) = except_handler;
        if let Some(type_) = &handler.type_ {
            self.visit_expr(type_);
        }
        if let Some(name) = &handler.name {
            self.add_binding(name.as_str(), name.range, BindingKind::BoundException);
        }
        self.visit_body(&handler.body);
    }

    fn visit_pattern(&mut self, pattern: &'a Pattern) {
        // `case Point(x=px)`, `case [first, *rest]` and `case {**rest}` capture names.
        visitor::walk_pattern(self, pattern);
        let name = match pattern {
            Pattern::MatchAs(ast::PatternMatchAs { name, .. })
            | Pattern::MatchStar(ast::PatternMatchStar { name, .. }) => name.as_ref(),
            Pattern::MatchMapping(ast::PatternMatchMapping { rest, .. }) => rest.as_ref(),
            _ => None,
        };
        if let Some(name) = name {
            self.add_binding(name.as_str(), name.range, BindingKind::Assignment);
        }
    }
}

impl<'a> Checker<'a> {
    fn visit_candidate_decorator(
        &mut self,
        decorator: &'a Expr,
        function: Option<&'a ast::StmtFunctionDef>,
    ) {
        self.visit_expr(decorator);
        // `@agent` and `@agent(...)` both refer to `agent`.
        let target = match decorator {
            Expr::Call(call) => call.func.as_ref(),
            _ => decorator,
        };
        let qualified_name = self.resolve_expr(target);
        if let Some(function) = function {
            if let Some((sink, via)) = self.prompt_sink(target, qualified_name.as_ref()) {
                if sink.returns_prompt() {
                    self.prompt_functions
                        .insert(function.name.range.start(), (sink, via.to_string()));
                }
            }
        }
        self.candidates.push(Candidate {
            kind: CandidateKind::Decorator {
                expr: decorator,
                function,
            },
            qualified_name,
            scope: self.semantic.scope_id,
            assigned_to: None,
        });
    }

    fn bind_parameters(&mut self, parameters: &'a Parameters) {
        for parameter in parameters.iter() {
            let name = parameter.name();
            self.add_binding(name.as_str(), name.range, BindingKind::Argument);
        }
    }

    /// Visits a comprehension. The first iterable is evaluated in the enclosing scope.
    /// Everything else runs in the comprehension's own scope.
    fn visit_generators(&mut self, generators: &'a [Comprehension], elements: &[&'a Expr]) {
        let Some((first, rest)) = generators.split_first() else {
            return;
        };
        self.visit_expr(&first.iter);

        self.semantic.push_scope(ScopeKind::Generator);
        self.visit_expr(&first.target);
        for condition in &first.ifs {
            self.visit_expr(condition);
        }
        for generator in rest {
            self.visit_comprehension(generator);
        }
        for element in elements {
            self.visit_expr(element);
        }
        self.semantic.pop_scope();
    }

    // -----------------------------------------------------------------------
    // Deferred visits
    // -----------------------------------------------------------------------

    /// Visits function and lambda bodies once their enclosing scopes are complete.
    /// Visiting one body can defer more (nested functions), so this loops until no
    /// work is left.
    fn visit_deferred(&mut self) {
        let snapshot = self.semantic.snapshot();
        while !self.visit.functions.is_empty() || !self.visit.lambdas.is_empty() {
            for (snapshot, function, binding) in std::mem::take(&mut self.visit.functions) {
                self.semantic.restore(snapshot);
                let scope = self.semantic.push_scope(ScopeKind::Function);
                self.function = Some((scope, binding));
                self.prompt_function = self
                    .prompt_functions
                    .get(&function.name.range.start())
                    .map(|(sink, via)| (scope, *sink, via.clone()));
                self.bind_parameters(&function.parameters);
                self.visit_body(&function.body);
            }
            self.prompt_function = None;
            self.function = None;
            for (snapshot, lambda) in std::mem::take(&mut self.visit.lambdas) {
                self.semantic.restore(snapshot);
                self.semantic.push_scope(ScopeKind::Lambda);
                if let Some(parameters) = &lambda.parameters {
                    self.bind_parameters(parameters);
                }
                self.visit_expr(&lambda.body);
            }
        }
        self.semantic.restore(snapshot);
    }

    // -----------------------------------------------------------------------
    // Prompts
    // -----------------------------------------------------------------------

    /// Visits `x = ...`, `x: T = ...` and `x += ...`, and remembers what `x` holds if
    /// it's a string or a call.
    fn visit_assignment(&mut self, stmt: &'a Stmt) {
        let (target, value, append) = match stmt {
            Stmt::Assign(ast::StmtAssign { targets, value, .. }) => match targets.as_slice() {
                [Expr::Name(name)] => (Some(name), value.as_ref(), false),
                _ => (None, value.as_ref(), false),
            },
            Stmt::AnnAssign(ast::StmtAnnAssign {
                target,
                value: Some(value),
                ..
            }) => (target.as_name_expr(), value.as_ref(), false),
            Stmt::AugAssign(ast::StmtAugAssign {
                target,
                op: ast::Operator::Add,
                value,
                ..
            }) => (target.as_name_expr(), value.as_ref(), true),
            _ => {
                visitor::walk_stmt(self, stmt);
                return;
            }
        };
        let Some(target) = target else {
            visitor::walk_stmt(self, stmt);
            return;
        };

        // The value is read before the name is rebound: in `p = p + "..."` and
        // `p += "..."`, the `p` on the right is the old one.
        let path = if append || value.is_string_literal_expr() {
            None
        } else {
            prompts::path_of(self, value)
        };
        let text = if append {
            let previous = self
                .semantic
                .lookup_symbol(target.id.as_str())
                .and_then(|binding| self.binding_texts.get(&binding));
            previous.and_then(|previous| {
                let mut text = previous.clone();
                text.extend(prompts::text_or_dynamic(self, value));
                Some(text)
            })
        } else {
            prompts::text_of(self, value)
        };

        visitor::walk_stmt(self, stmt);

        let Some(binding) = self.semantic.current_scope().get(target.id.as_str()) else {
            return;
        };
        if let Some(text) = text {
            self.binding_texts.insert(binding, text);
        }
        if let Some(path) = path {
            self.binding_paths.insert(binding, path);
        }
        if let Expr::Call(call) = value {
            if !append {
                self.binding_calls.insert(binding, call);
            }
        }
    }

    /// The text a variable holds, if it was assigned a string.
    pub fn binding_text(&self, binding: BindingId) -> Option<&[Piece<'a>]> {
        self.binding_texts.get(&binding).map(Vec::as_slice)
    }

    /// The file path a variable holds, if it was assigned one.
    pub fn binding_path(&self, binding: BindingId) -> Option<&SourcePath<'a>> {
        self.binding_paths.get(&binding)
    }

    /// The file a `with open(...) as f:` variable reads.
    pub fn binding_opened(&self, binding: BindingId) -> Option<&SourcePath<'a>> {
        self.binding_opened.get(&binding)
    }

    /// The text `function` returns, if it has exactly one `return` and that returns a
    /// string.
    pub fn function_return(&self, function: BindingId) -> Option<&[Piece<'a>]> {
        self.function_returns.get(&function)?.as_deref()
    }

    /// Records what the function being visited returns, and reports it as a prompt
    /// if the function is decorated as one (`@agent.instructions`).
    fn visit_return(&mut self, value: Option<&'a Expr>) {
        if let Some((scope, function)) = self.function {
            if scope == self.semantic.scope_id {
                let text = value.and_then(|value| prompts::text_of(self, value));
                self.function_returns
                    .entry(function)
                    .and_modify(|returns| *returns = None)
                    .or_insert(text);
            }
        }
        let Some(value) = value else {
            return;
        };
        if let Some((scope, sink, via)) = &self.prompt_function {
            if *scope == self.semantic.scope_id {
                self.prompts.push(Prompt {
                    framework: sink.framework,
                    role: prompts::Role::System,
                    via: Some(via.clone()),
                    argument: "return",
                    value,
                    pieces: prompts::text_or_dynamic(self, value),
                    call: None,
                });
            }
        }
    }

    /// The YAML file a crewAI `self.agents_config` or `self.tasks_config` refers to:
    /// the class attribute of that name (`agents_config = "config/agents.yaml"`) in
    /// the class being visited, or crewAI's default.
    pub fn crewai_config_file(&self, attribute: &'a ast::ExprAttribute) -> Option<&'a str> {
        let Expr::Name(receiver) = attribute.value.as_ref() else {
            return None;
        };
        if receiver.id.as_str() != "self" {
            return None;
        }
        let name = attribute.attr.as_str();
        let class = self
            .semantic
            .ancestor_ids(self.semantic.scope_id)
            .find(|id| self.semantic.scopes[*id].kind == ScopeKind::Class);
        let declared = class
            .and_then(|class| self.semantic.scopes[class].get(name))
            .and_then(|binding| match self.binding_text(binding)? {
                [Piece::Text(text, _)] => Some(*text),
                _ => None,
            });
        declared.or(match name {
            "agents_config" => Some("config/agents.yaml"),
            "tasks_config" => Some("config/tasks.yaml"),
            _ => None,
        })
    }

    /// The prompt sink `func` refers to, and its qualified name, if any. `resolved` is
    /// what `func` already resolved to, which the walker works out for every call.
    ///
    /// Methods on objects are resolved through the call the object was created by:
    /// with `client = Anthropic()`, `client.messages.create` is
    /// `anthropic.Anthropic.messages.create`.
    pub fn prompt_sink(
        &self,
        func: &'a Expr,
        resolved: Option<&QualifiedName<'a>>,
    ) -> Option<(&'static PromptSink, QualifiedName<'a>)> {
        if let Some(name) = resolved {
            return prompts::find_sink(name.segments()).map(|sink| (sink, name.clone()));
        }
        // An unresolved method call, such as `client.messages.create`. Almost every one
        // stops here, before the object is looked up.
        let Expr::Attribute(attribute) = func else {
            return None;
        };
        if !prompts::is_sink_method(attribute.attr.as_str()) {
            return None;
        }
        let name = self.resolve_through_instance(func)?;
        let sink = prompts::find_sink(name.segments())?;
        Some((sink, name))
    }

    /// `client.messages.create`, where `client = Anthropic()`, or
    /// `Anthropic().messages.create` → `anthropic.Anthropic.messages.create`.
    fn resolve_through_instance(&self, expr: &'a Expr) -> Option<QualifiedName<'a>> {
        let mut tail = Vec::new();
        let mut current = expr;
        while let Expr::Attribute(attribute) = current {
            tail.push(attribute.attr.as_str());
            current = &attribute.value;
        }
        if tail.is_empty() {
            return None;
        }
        tail.reverse();
        let call = match current {
            Expr::Name(name) => {
                let binding = self.semantic.lookup_symbol(name.id.as_str())?;
                *self.binding_calls.get(&binding)?
            }
            Expr::Call(call) => call,
            _ => return None,
        };
        Some(self.resolve_expr(&call.func)?.append(&tail))
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn add_binding(&mut self, name: &'a str, range: TextRange, kind: BindingKind<'a>) {
        self.semantic.push_binding(name, range, kind);
    }

    /// Resolves `a.b.c` (as written at the current position) to a qualified name.
    pub fn resolve_expr(&self, expr: &'a Expr) -> Option<QualifiedName<'a>> {
        let segments = collect_dotted_name(expr)?;
        self.semantic.resolve_qualified_name(&segments)
    }
}

/// `x = call(...)` or `x: T = call(...)` → `x`. The value is visited before the target,
/// so the call is the first expression visited after the statement.
fn assignment_of_call(stmt: &Stmt) -> Option<&str> {
    let (targets, value): (&[Expr], &Expr) = match stmt {
        Stmt::Assign(assign) => (&assign.targets, &assign.value),
        Stmt::AnnAssign(ast::StmtAnnAssign {
            target,
            value: Some(value),
            ..
        }) => (std::slice::from_ref(target.as_ref()), value),
        _ => return None,
    };
    match (targets, value) {
        ([Expr::Name(name)], Expr::Call(_)) => Some(name.id.as_str()),
        _ => None,
    }
}

/// Turns `a.b.c` into `["a", "b", "c"]`. Returns `None` for anything that isn't a
/// plain chain of names, such as `make()()` or `agents[0]`.
///
/// This is the only bridge from Python syntax to the language-independent model.
fn collect_dotted_name(expr: &Expr) -> Option<Vec<&str>> {
    match expr {
        Expr::Name(name) => Some(vec![name.id.as_str()]),
        Expr::Attribute(attribute) => {
            let mut segments = collect_dotted_name(&attribute.value)?;
            segments.push(attribute.attr.as_str());
            Some(segments)
        }
        _ => None,
    }
}

/// `from ..foo.bar import baz` → `[".", ".", "foo", "bar", "baz"]`.
///
/// Relative imports keep their dots. Resolving them needs the module's location in its
/// package, which is the job of the cross-file step.
fn collect_import_from_member<'a>(
    level: u32,
    module: Option<&'a str>,
    member: &'a str,
) -> QualifiedName<'a> {
    let dots = std::iter::repeat_n(".", level as usize);
    let module = module.into_iter().flat_map(|module| module.split('.'));
    QualifiedName::from_segments(dots.chain(module).chain(std::iter::once(member)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call_names(source: &str) -> Vec<Option<String>> {
        let parsed = ruff_python_parser::parse_module(source).expect("valid Python");
        let checker = check_module(parsed.syntax(), vec!["app"]);
        checker
            .candidates
            .iter()
            .filter(|candidate| matches!(candidate.kind, CandidateKind::Call(_)))
            .map(|candidate| candidate.qualified_name.as_ref().map(ToString::to_string))
            .collect()
    }

    #[test]
    fn resolves_imports_and_aliases() {
        let names = call_names(
            "from pydantic_ai import Agent as A\nimport crewai\nimport google.adk.agents\n\
             a = A('m')\nb = crewai.Agent(role='r')\nc = google.adk.agents.LlmAgent(name='x')\n",
        );
        assert_eq!(
            names,
            vec![
                Some("pydantic_ai.Agent".to_string()),
                Some("crewai.Agent".to_string()),
                Some("google.adk.agents.LlmAgent".to_string()),
            ]
        );
    }

    #[test]
    fn function_bodies_see_later_imports() {
        let names = call_names("def build():\n    return A('m')\n\nfrom pydantic_ai import Agent as A\n");
        assert_eq!(names, vec![Some("pydantic_ai.Agent".to_string())]);
    }

    #[test]
    fn local_names_shadow_imports() {
        let names = call_names(
            "from crewai import Agent\n\ndef f(Agent):\n    return Agent()\n\nAgent()\n",
        );
        // Inside `f`, `Agent` is a parameter, so it can't be resolved.
        assert_eq!(names, vec![Some("crewai.Agent".to_string()), None]);
    }

    #[test]
    fn assignments_in_blocks_and_comprehensions_shadow_imports() {
        let names = call_names(
            "from crewai import Agent\n\
             [Agent() for Agent in factories]\n\
             Agent()\n\
             for Agent in factories:\n    Agent()\n",
        );
        // The comprehension variable only shadows the import inside the comprehension,
        // but the `for` loop variable rebinds it in the module scope.
        assert_eq!(names, vec![None, Some("crewai.Agent".to_string()), None]);
    }

    #[test]
    fn match_captures_bind_names() {
        let names = call_names(
            "from crewai import Agent\nmatch x:\n    case [Agent]:\n        Agent()\n",
        );
        assert_eq!(names, vec![None]);
    }

    #[test]
    fn local_definitions_get_module_paths() {
        let names = call_names("class Reviewer:\n    pass\n\nReviewer()\n");
        assert_eq!(names, vec![Some("app.Reviewer".to_string())]);
    }

    #[test]
    fn relative_imports_keep_their_dots() {
        let names = call_names("from ..agents import make\nmake()\n");
        assert_eq!(names, vec![Some("..agents.make".to_string())]);
    }
}
