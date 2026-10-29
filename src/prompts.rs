//! Prompt discovery: finds the text passed to agents and model APIs as prompts.
//!
//! A string is a prompt because of where it goes, not what it says. `system =
//! platform.system()` and `prompt = usage["prompt_tokens"]` aren't prompts, and
//! `instructions = "- Use currentColor"` is. So discovery starts from a registry of
//! *sinks*: the arguments that take a prompt, such as `system_prompt=` on
//! `pydantic_ai.Agent` or `system=` on `anthropic.Anthropic().messages.create`.
//!
//! When the checker reaches a sink, it works out the argument's text right there,
//! using the semantic model as it stands at that point in the walk:
//!
//! ```python
//! BASE = "You review pull requests."
//! RULES = BASE + "\nBe brief."                   # remembered as two pieces of text
//! agent = Agent("openai:gpt-4o", instructions=f"{RULES}\nToday is {date}.")
//! #                                → "You review pull requests.\nBe brief.\nToday is {date}."
//! ```
//!
//! The text is a list of [`Piece`]s that borrow from the syntax tree, so nothing is
//! copied until a prompt is reported. Parts that are only known at runtime, such as
//! `{date}` above, stay as [`Piece::Dynamic`] and point at their expression.
//!
//! Some text lives somewhere else, and is marked so the cross-file step
//! (`project.rs`) can fill it in once every file has been checked:
//! - a name imported from another module ([`Piece::Import`]): `prompt.ROOT_PROMPT`;
//! - a file read at runtime ([`Piece::File`]): `Path("system.md").read_text()`;
//! - a crewAI YAML config entry ([`Piece::Config`]): `config=self.agents_config["writer"]`.
//!
//! What a function in the same module returns ([`Piece::Returned`]) is filled in at
//! the end of the walk, once function bodies have been visited.
//!
//! The work is proportional to the number of sinks and string assignments, not to
//! the size of the file: a call that isn't a sink costs one hash lookup.

use std::sync::LazyLock;

use ruff_python_ast::{self as ast, Expr, FStringPartRef, InterpolatedStringElement, Operator};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::FxHashMap;

use crate::checker::Checker;
use crate::semantic::{BindingId, BindingKind, QualifiedName};

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Who the prompt speaks as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Instructions to the model: a system prompt, an agent's instructions or persona.
    System,
    /// A request to the model: a user message, a task, the input to a run.
    User,
    /// A template whose role isn't known where it's written (`PromptTemplate`).
    Template,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Template => "template",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Arg {
    /// `name=...`
    Keyword(&'static str),
    /// The argument at this position, or `name=...`.
    Positional(usize, &'static str),
    /// A list of `(role, text)` tuples at position 0 or `messages=`, as taken by
    /// LangChain's `ChatPromptTemplate.from_messages`.
    MessageTuples,
    /// `config=self.agents_config["writer"]`: these fields of an entry in a crewAI
    /// YAML config.
    Config(&'static [&'static str]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SinkKind {
    /// Calling it with the prompt as an argument.
    Call,
    /// Decorating a function with it makes the function's return value a prompt:
    /// `@agent.instructions`.
    Returns,
}

pub struct PromptSink {
    pub framework: &'static str,
    kind: SinkKind,
    args: &'static [(Arg, Role)],
}

const fn call(framework: &'static str, args: &'static [(Arg, Role)]) -> PromptSink {
    PromptSink {
        framework,
        kind: SinkKind::Call,
        args,
    }
}

use Arg::{Config, Keyword, MessageTuples, Positional};
use Role::{System, Template, User};

const PYDANTIC_AGENT: PromptSink = call(
    "pydantic-ai",
    &[(Keyword("system_prompt"), System), (Keyword("instructions"), System)],
);
const PYDANTIC_RUN: PromptSink = call("pydantic-ai", &[(Positional(0, "user_prompt"), User)]);
const PYDANTIC_DECORATOR: PromptSink = PromptSink {
    framework: "pydantic-ai",
    kind: SinkKind::Returns,
    args: &[],
};
const OPENAI_AGENT: PromptSink = call("openai-agents", &[(Keyword("instructions"), System)]);
const OPENAI_RUNNER: PromptSink = call("openai-agents", &[(Positional(1, "input"), User)]);
const CREWAI_AGENT: PromptSink = call(
    "crewai",
    &[
        (Keyword("role"), System),
        (Keyword("goal"), System),
        (Keyword("backstory"), System),
        (Config(&["role", "goal", "backstory"]), System),
    ],
);
const CREWAI_TASK: PromptSink = call(
    "crewai",
    &[
        (Keyword("description"), User),
        (Keyword("expected_output"), User),
        (Config(&["description", "expected_output"]), User),
    ],
);
const ADK_AGENT: PromptSink = call(
    "google-adk",
    &[
        (Keyword("instruction"), System),
        (Keyword("global_instruction"), System),
        (Keyword("static_instruction"), System),
    ],
);
const LANGCHAIN_SYSTEM_MESSAGE: PromptSink = call("langchain", &[(Positional(0, "content"), System)]);
const LANGCHAIN_HUMAN_MESSAGE: PromptSink = call("langchain", &[(Positional(0, "content"), User)]);
const LANGCHAIN_FROM_MESSAGES: PromptSink = call("langchain", &[(MessageTuples, Template)]);
const LANGCHAIN_TEMPLATE: PromptSink = call("langchain", &[(Positional(0, "template"), Template)]);
const LANGCHAIN_SYSTEM_TEMPLATE: PromptSink = call("langchain", &[(Positional(0, "template"), System)]);
const LANGCHAIN_HUMAN_TEMPLATE: PromptSink = call("langchain", &[(Positional(0, "template"), User)]);
const GENAI_CONFIG: PromptSink = call("google-genai", &[(Keyword("system_instruction"), System)]);
const ANTHROPIC_MESSAGES: PromptSink = call("anthropic", &[(Keyword("system"), System)]);
const OPENAI_RESPONSES: PromptSink = call(
    "openai",
    &[(Keyword("instructions"), System), (Keyword("input"), User)],
);
const OPENAI_ASSISTANTS: PromptSink = call("openai", &[(Keyword("instructions"), System)]);

/// Public import paths of prompt sinks, as for the agent registry in `discovery.rs`.
/// Methods on an object are written as `Class.method`: `pydantic_ai.Agent.run_sync`
/// matches `agent.run_sync(...)` where `agent = Agent(...)`.
const REGISTRY: &[(&str, &PromptSink)] = &[
    ("pydantic_ai.Agent", &PYDANTIC_AGENT),
    ("pydantic_ai.agent.Agent", &PYDANTIC_AGENT),
    ("pydantic_ai.Agent.run", &PYDANTIC_RUN),
    ("pydantic_ai.Agent.run_sync", &PYDANTIC_RUN),
    ("pydantic_ai.Agent.run_stream", &PYDANTIC_RUN),
    ("pydantic_ai.Agent.iter", &PYDANTIC_RUN),
    ("pydantic_ai.Agent.system_prompt", &PYDANTIC_DECORATOR),
    ("pydantic_ai.Agent.instructions", &PYDANTIC_DECORATOR),
    ("agents.Agent", &OPENAI_AGENT),
    ("agents.agent.Agent", &OPENAI_AGENT),
    ("agents.Runner.run", &OPENAI_RUNNER),
    ("agents.Runner.run_sync", &OPENAI_RUNNER),
    ("agents.Runner.run_streamed", &OPENAI_RUNNER),
    ("crewai.Agent", &CREWAI_AGENT),
    ("crewai.agent.Agent", &CREWAI_AGENT),
    ("crewai.Task", &CREWAI_TASK),
    ("crewai.task.Task", &CREWAI_TASK),
    ("google.adk.agents.Agent", &ADK_AGENT),
    ("google.adk.agents.LlmAgent", &ADK_AGENT),
    ("google.adk.agents.llm_agent.LlmAgent", &ADK_AGENT),
    ("google.genai.types.GenerateContentConfig", &GENAI_CONFIG),
    ("google.generativeai.GenerativeModel", &GENAI_CONFIG),
    ("autogen_agentchat.agents.AssistantAgent", &call("autogen", &[(Keyword("system_message"), System)])),
    ("langchain.agents.create_agent", &call("langchain", &[(Keyword("system_prompt"), System)])),
    ("langgraph.prebuilt.create_react_agent", &call("langgraph", &[(Keyword("prompt"), System)])),
    (
        "deepagents.create_deep_agent",
        &call("deepagents", &[(Keyword("system_prompt"), System), (Keyword("instructions"), System)]),
    ),
    ("langchain_core.messages.SystemMessage", &LANGCHAIN_SYSTEM_MESSAGE),
    ("langchain_core.messages.system.SystemMessage", &LANGCHAIN_SYSTEM_MESSAGE),
    ("langchain.messages.SystemMessage", &LANGCHAIN_SYSTEM_MESSAGE),
    ("langchain.schema.SystemMessage", &LANGCHAIN_SYSTEM_MESSAGE),
    ("langchain_core.messages.HumanMessage", &LANGCHAIN_HUMAN_MESSAGE),
    ("langchain_core.messages.human.HumanMessage", &LANGCHAIN_HUMAN_MESSAGE),
    ("langchain.messages.HumanMessage", &LANGCHAIN_HUMAN_MESSAGE),
    ("langchain.schema.HumanMessage", &LANGCHAIN_HUMAN_MESSAGE),
    ("langchain_core.prompts.ChatPromptTemplate.from_messages", &LANGCHAIN_FROM_MESSAGES),
    ("langchain_core.prompts.chat.ChatPromptTemplate.from_messages", &LANGCHAIN_FROM_MESSAGES),
    ("langchain.prompts.ChatPromptTemplate.from_messages", &LANGCHAIN_FROM_MESSAGES),
    ("langchain_core.prompts.ChatPromptTemplate.from_template", &LANGCHAIN_HUMAN_TEMPLATE),
    ("langchain_core.prompts.PromptTemplate", &LANGCHAIN_TEMPLATE),
    ("langchain_core.prompts.PromptTemplate.from_template", &LANGCHAIN_TEMPLATE),
    ("langchain_core.prompts.prompt.PromptTemplate", &LANGCHAIN_TEMPLATE),
    ("langchain.prompts.PromptTemplate", &LANGCHAIN_TEMPLATE),
    ("langchain.prompts.PromptTemplate.from_template", &LANGCHAIN_TEMPLATE),
    ("langchain_core.prompts.SystemMessagePromptTemplate.from_template", &LANGCHAIN_SYSTEM_TEMPLATE),
    ("langchain_core.prompts.HumanMessagePromptTemplate.from_template", &LANGCHAIN_HUMAN_TEMPLATE),
];

/// SDK methods called on a client object, for each client class that has them.
const CLIENT_METHODS: &[(&[&str], &[&str], &PromptSink)] = &[
    (
        &[
            "anthropic.Anthropic",
            "anthropic.AsyncAnthropic",
            "anthropic.AnthropicBedrock",
            "anthropic.AnthropicVertex",
        ],
        &["messages.create", "messages.stream", "beta.messages.create", "beta.messages.stream"],
        &ANTHROPIC_MESSAGES,
    ),
    (
        &[
            "openai.OpenAI",
            "openai.AsyncOpenAI",
            "openai.AzureOpenAI",
            "openai.AsyncAzureOpenAI",
        ],
        &["responses.create", "responses.stream"],
        &OPENAI_RESPONSES,
    ),
    (
        &[
            "openai.OpenAI",
            "openai.AsyncOpenAI",
            "openai.AzureOpenAI",
            "openai.AsyncAzureOpenAI",
        ],
        &["beta.assistants.create"],
        &OPENAI_ASSISTANTS,
    ),
];

/// The registry, indexed by the last segment of each name. A call is looked up by its
/// last segment first, so a call that isn't a sink (almost all of them) costs one hash
/// lookup and no allocation.
struct SinkIndex {
    by_last_segment: FxHashMap<&'static str, Vec<(Vec<&'static str>, &'static PromptSink)>>,
}

static SINKS: LazyLock<SinkIndex> = LazyLock::new(|| {
    let mut names: Vec<(&'static str, &'static PromptSink)> = REGISTRY.to_vec();
    for (clients, methods, sink) in CLIENT_METHODS {
        for client in *clients {
            for method in *methods {
                // Built once per process. Leaking gives the segments the same
                // `'static` lifetime as the rest of the registry.
                let name: &'static str = Box::leak(format!("{client}.{method}").into_boxed_str());
                names.push((name, sink));
            }
        }
    }

    let mut by_last_segment: FxHashMap<_, Vec<_>> = FxHashMap::default();
    for (name, sink) in names {
        let segments: Vec<&'static str> = name.split('.').collect();
        let last = *segments.last().expect("non-empty name");
        by_last_segment.entry(last).or_default().push((segments, sink));
    }
    SinkIndex { by_last_segment }
});

/// The top-level modules sinks are imported from, for the prefilter in `discovery.rs`.
pub fn registry_roots() -> impl Iterator<Item = &'static str> {
    REGISTRY
        .iter()
        .map(|(name, _)| *name)
        .chain(CLIENT_METHODS.iter().flat_map(|(clients, _, _)| clients.iter().copied()))
        .filter_map(|name| name.split('.').next())
}

/// Whether some sink's name ends in `segment`. Checked before the more expensive
/// lookup through client objects.
pub fn is_sink_method(segment: &str) -> bool {
    SINKS.by_last_segment.contains_key(segment)
}

pub fn find_sink(segments: &[&str]) -> Option<&'static PromptSink> {
    let candidates = SINKS.by_last_segment.get(segments.last()?)?;
    candidates
        .iter()
        .find(|(name, _)| name.as_slice() == segments)
        .map(|(_, sink)| *sink)
}

// ---------------------------------------------------------------------------
// Prompt text
// ---------------------------------------------------------------------------

/// One piece of a prompt's text.
#[derive(Clone, Debug)]
pub enum Piece<'a> {
    /// Text written in the source. `range` is where it's written, which is not
    /// necessarily next to the other pieces: it may be a constant defined elsewhere.
    Text(&'a str, TextRange),
    /// A part only known at runtime, such as `{date}` in an f-string or a parameter.
    Dynamic(&'a Expr),
    /// A name imported from another module: `PROMPT` after `from .prompts import
    /// PROMPT`, or `prompts.PROMPT` after `from . import prompts`. `called` when it's
    /// called, `build()`, which means its text is what the function returns.
    Import {
        name: QualifiedName<'a>,
        called: bool,
        expr: &'a Expr,
    },
    /// What a function defined in this module returns: `instruction=build()`.
    Returned { function: BindingId, expr: &'a Expr },
    /// The contents of a file read at runtime: `Path("system.md").read_text()`.
    File { path: SourcePath<'a>, expr: &'a Expr },
    /// A field of an entry in a crewAI YAML config, relative to the module's directory:
    /// `role` of `writer` in `config/agents.yaml`.
    Config {
        file: &'a str,
        key: &'a str,
        field: &'static str,
        expr: &'a Expr,
    },
}

/// A file path as written in the source, such as `Path(__file__).parent / "system.md"`.
#[derive(Clone, Debug)]
pub struct SourcePath<'a> {
    pub anchor: Anchor,
    pub steps: Vec<PathStep<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// `__file__`: the module's own file.
    File,
    /// A relative path, which Python resolves against the working directory.
    WorkingDirectory,
}

#[derive(Clone, Copy, Debug)]
pub enum PathStep<'a> {
    /// `.parent` or `os.path.dirname(...)`.
    Parent,
    /// `/ "prompts"`, `.joinpath("prompts")` or `os.path.join(..., "prompts")`.
    Join(&'a str),
}

/// Pieces beyond this are dropped and the prompt is marked as having a dynamic tail.
/// Repeated `prompt += ...` in a long function would otherwise copy ever-growing lists.
const MAX_PIECES: usize = 256;

/// A prompt found during the walk.
#[derive(Debug)]
pub struct Prompt<'a> {
    pub framework: &'static str,
    pub role: Role,
    /// What the prompt was passed to, such as `pydantic_ai.Agent`, or `None` for a
    /// message dict (`{"role": "system", "content": ...}`).
    pub via: Option<String>,
    /// The argument it was passed as: `system_prompt`, `content`, or `return` for a
    /// function whose return value is the prompt.
    pub argument: &'static str,
    /// The expression the prompt was written as, at the sink.
    pub value: &'a Expr,
    pub pieces: Vec<Piece<'a>>,
    /// The call it was passed to, which is also where the agent is reported when the
    /// call creates one.
    pub call: Option<TextRange>,
}

impl Ranged for Prompt<'_> {
    fn range(&self) -> TextRange {
        self.value.range()
    }
}

/// The text of `expr`, if it's built from strings. `None` for anything else, such as
/// a number, a list, or a call that isn't a known string helper.
pub fn text_of<'a>(checker: &Checker<'a>, expr: &'a Expr) -> Option<Vec<Piece<'a>>> {
    let mut pieces = Vec::new();
    push_text(checker, expr, &mut pieces).then_some(pieces)
}

/// The text of `expr`, or a single dynamic piece if it isn't built from strings.
pub fn text_or_dynamic<'a>(checker: &Checker<'a>, expr: &'a Expr) -> Vec<Piece<'a>> {
    text_of(checker, expr).unwrap_or_else(|| vec![Piece::Dynamic(expr)])
}

/// Appends the text of `expr` and returns `true`, or returns `false` without appending
/// anything meaningful if `expr` isn't built from strings.
fn push_text<'a>(checker: &Checker<'a>, expr: &'a Expr, out: &mut Vec<Piece<'a>>) -> bool {
    if out.len() >= MAX_PIECES {
        out.push(Piece::Dynamic(expr));
        return true;
    }
    match expr {
        Expr::StringLiteral(literal) => {
            // Implicit concatenation (`"a" "b"`) keeps each part and where it's written.
            for part in literal.value.iter() {
                out.push(Piece::Text(&part.value, part.range));
            }
            true
        }
        Expr::FString(fstring) => {
            for part in fstring.value.iter() {
                match part {
                    FStringPartRef::Literal(literal) => out.push(Piece::Text(&literal.value, literal.range)),
                    FStringPartRef::FString(fstring) => {
                        for element in fstring.elements.iter() {
                            match element {
                                InterpolatedStringElement::Literal(literal) => {
                                    out.push(Piece::Text(&literal.value, literal.range));
                                }
                                InterpolatedStringElement::Interpolation(interpolation) => {
                                    push_text_or_dynamic(checker, &interpolation.expression, out);
                                }
                            }
                        }
                    }
                }
            }
            true
        }
        Expr::BinOp(ast::ExprBinOp {
            left, op: Operator::Add, right, ..
        }) => {
            let start = out.len();
            let left = push_text_or_dynamic(checker, left, out);
            let right = push_text_or_dynamic(checker, right, out);
            if !left && !right {
                out.truncate(start);
                return false;
            }
            true
        }
        // `"Hello %s" % name`: the template is the prompt and keeps its placeholders.
        Expr::BinOp(ast::ExprBinOp {
            left, op: Operator::Mod, ..
        }) => push_text(checker, left, out),
        Expr::Call(call) => push_call_text(checker, expr, call, out),
        Expr::Name(name) => {
            let Some(binding) = checker.semantic.lookup_symbol(name.id.as_str()) else {
                return false;
            };
            if let Some(text) = checker.binding_text(binding) {
                out.extend_from_slice(text);
                return true;
            }
            push_import(checker, binding, expr, false, out)
        }
        // `prompts.ROOT_PROMPT`, where `prompts` is an imported module.
        Expr::Attribute(attribute) => {
            let Some(root) = root_name(&attribute.value) else {
                return false;
            };
            let Some(binding) = checker.semantic.lookup_symbol(root) else {
                return false;
            };
            push_import(checker, binding, expr, false, out)
        }
        _ => false,
    }
}

/// The name a chain of attributes starts from: `a` in `a.b.c`.
fn root_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Name(name) => Some(name.id.as_str()),
        Expr::Attribute(attribute) => root_name(&attribute.value),
        _ => None,
    }
}

/// Appends an [`Piece::Import`] if `binding` is an import, so the text can be found in
/// the module it comes from.
fn push_import<'a>(
    checker: &Checker<'a>,
    binding: BindingId,
    expr: &'a Expr,
    called: bool,
    out: &mut Vec<Piece<'a>>,
) -> bool {
    if !matches!(
        checker.semantic.binding(binding).kind,
        BindingKind::Import(_) | BindingKind::FromImport(_) | BindingKind::SubmoduleImport(_)
    ) {
        return false;
    }
    let Some(name) = checker.resolve_expr(expr) else {
        return false;
    };
    // `ImageUrl(...)` creates an object; it doesn't return a prompt. Looking it up
    // would mean parsing the module that defines the class to learn nothing, so a
    // called name written as a class (`CapWords`) is left as runtime data.
    if called && name.segments().last().is_some_and(|last| is_class_name(last)) {
        return false;
    }
    out.push(Piece::Import { name, called, expr });
    true
}

/// `ImageUrl` but not `ROOT_PROMPT` or `build`.
fn is_class_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase()) && name.contains(|c: char| c.is_ascii_lowercase())
}

/// Appends the text of `expr`, or a dynamic piece for it. Returns whether it was text.
fn push_text_or_dynamic<'a>(checker: &Checker<'a>, expr: &'a Expr, out: &mut Vec<Piece<'a>>) -> bool {
    let start = out.len();
    if push_text(checker, expr, out) {
        return true;
    }
    out.truncate(start);
    out.push(Piece::Dynamic(expr));
    false
}

/// String methods and helpers that return their input's text, more or less as written.
fn push_call_text<'a>(
    checker: &Checker<'a>,
    expr: &'a Expr,
    call: &'a ast::ExprCall,
    out: &mut Vec<Piece<'a>>,
) -> bool {
    if let Expr::Attribute(attribute) = call.func.as_ref() {
        match attribute.attr.as_str() {
            // The template is the prompt; the values filled in are runtime data.
            "format" | "format_map" | "strip" | "lstrip" | "rstrip" => {
                return push_text(checker, &attribute.value, out);
            }
            // `"\n".join([a, b])`, when the parts are written out.
            "join" => {
                let ([Expr::List(ast::ExprList { elts, .. }) | Expr::Tuple(ast::ExprTuple { elts, .. })], []) =
                    (&*call.arguments.args, &*call.arguments.keywords)
                else {
                    return false;
                };
                let mut separator = Vec::new();
                if !push_text(checker, &attribute.value, &mut separator) {
                    return false;
                }
                for (index, element) in elts.iter().enumerate() {
                    if index > 0 {
                        out.extend_from_slice(&separator);
                    }
                    push_text_or_dynamic(checker, element, out);
                }
                return true;
            }
            // `path.read_text()`
            "read_text" => {
                let Some(path) = path_of(checker, &attribute.value) else {
                    return false;
                };
                out.push(Piece::File {
                    path,
                    expr,
                });
                return true;
            }
            // `open(path).read()`, or `f.read()` inside `with open(path) as f:`.
            "read" => {
                let path = match attribute.value.as_ref() {
                    Expr::Call(opened) => opened_path(checker, opened),
                    Expr::Name(name) => checker
                        .semantic
                        .lookup_symbol(name.id.as_str())
                        .and_then(|binding| checker.binding_opened(binding))
                        .cloned(),
                    _ => None,
                };
                let Some(path) = path else {
                    return false;
                };
                out.push(Piece::File {
                    path,
                    expr,
                });
                return true;
            }
            _ => {}
        }
    }

    match call.func.as_ref() {
        // `build()`, where `build` is a function in this module or an imported one.
        Expr::Name(name) => {
            let Some(binding) = checker.semantic.lookup_symbol(name.id.as_str()) else {
                return false;
            };
            if matches!(checker.semantic.binding(binding).kind, BindingKind::FunctionDefinition) {
                out.push(Piece::Returned {
                    function: binding,
                    expr,
                });
                return true;
            }
            if let Some(argument) = whitespace_helper_argument(checker, call) {
                return push_text(checker, argument, out);
            }
            push_import(checker, binding, &call.func, true, out)
                && fix_import_expr(out, expr)
        }
        Expr::Attribute(attribute) => {
            if let Some(argument) = whitespace_helper_argument(checker, call) {
                return push_text(checker, argument, out);
            }
            // `prompts.build()`, where `prompts` is an imported module.
            let Some(root) = root_name(&attribute.value) else {
                return false;
            };
            let Some(binding) = checker.semantic.lookup_symbol(root) else {
                return false;
            };
            push_import(checker, binding, &call.func, true, out)
                && fix_import_expr(out, expr)
        }
        _ => false,
    }
}

/// The argument of `textwrap.dedent(...)` or `inspect.cleandoc(...)`, which only
/// change whitespace.
fn whitespace_helper_argument<'a>(checker: &Checker<'a>, call: &'a ast::ExprCall) -> Option<&'a Expr> {
    let last = match call.func.as_ref() {
        Expr::Name(name) => name.id.as_str(),
        Expr::Attribute(attribute) => attribute.attr.as_str(),
        _ => return None,
    };
    if !matches!(last, "dedent" | "cleandoc") {
        return None;
    }
    let callee = checker.resolve_expr(&call.func)?;
    if !matches!(callee.segments(), ["textwrap", "dedent"] | ["inspect", "cleandoc"]) {
        return None;
    }
    match &*call.arguments.args {
        [argument] => Some(argument),
        _ => None,
    }
}

/// `push_import` records the callee. For a call, the whole call is what's shown when
/// the text can't be found, so point the piece at it.
fn fix_import_expr<'a>(out: &mut [Piece<'a>], call: &'a Expr) -> bool {
    if let Some(Piece::Import { expr, .. }) = out.last_mut() {
        *expr = call;
    }
    true
}

/// The path a file is opened from: `open(path)`, `Path(...).open()` or `io.open(path)`.
pub fn opened_path<'a>(checker: &Checker<'a>, call: &'a ast::ExprCall) -> Option<SourcePath<'a>> {
    match call.func.as_ref() {
        Expr::Name(name) if name.id.as_str() == "open" => {
            // Only the builtin: a module may define its own `open`.
            if checker.semantic.lookup_symbol("open").is_some() {
                return None;
            }
            path_of(checker, call.arguments.find_argument_value("file", 0)?)
        }
        Expr::Attribute(attribute) if attribute.attr.as_str() == "open" => {
            if let Some(callee) = checker.resolve_expr(&call.func) {
                if matches!(callee.segments(), ["io", "open"] | ["codecs", "open"]) {
                    return path_of(checker, call.arguments.find_argument_value("file", 0)?);
                }
            }
            path_of(checker, &attribute.value)
        }
        _ => None,
    }
}

/// The file path `expr` builds, if it's built from literals and `__file__`.
pub fn path_of<'a>(checker: &Checker<'a>, expr: &'a Expr) -> Option<SourcePath<'a>> {
    match expr {
        Expr::StringLiteral(literal) => Some(SourcePath {
            anchor: Anchor::WorkingDirectory,
            steps: vec![PathStep::Join(literal.value.to_str())],
        }),
        Expr::Name(name) => {
            if name.id.as_str() == "__file__" {
                return Some(SourcePath {
                    anchor: Anchor::File,
                    steps: Vec::new(),
                });
            }
            let binding = checker.semantic.lookup_symbol(name.id.as_str())?;
            if let Some(path) = checker.binding_path(binding) {
                return Some(path.clone());
            }
            match checker.binding_text(binding)? {
                [Piece::Text(text, _)] => Some(SourcePath {
                    anchor: Anchor::WorkingDirectory,
                    steps: vec![PathStep::Join(text)],
                }),
                _ => None,
            }
        }
        Expr::Attribute(attribute) if attribute.attr.as_str() == "parent" => {
            let mut path = path_of(checker, &attribute.value)?;
            path.steps.push(PathStep::Parent);
            Some(path)
        }
        // `Path(__file__).parents[1]`
        Expr::Subscript(subscript) => {
            let Expr::Attribute(attribute) = subscript.value.as_ref() else {
                return None;
            };
            let Expr::NumberLiteral(ast::ExprNumberLiteral {
                value: ast::Number::Int(index),
                ..
            }) = subscript.slice.as_ref()
            else {
                return None;
            };
            if attribute.attr.as_str() != "parents" {
                return None;
            }
            let mut path = path_of(checker, &attribute.value)?;
            for _ in 0..=index.as_u64()? {
                path.steps.push(PathStep::Parent);
            }
            Some(path)
        }
        Expr::BinOp(ast::ExprBinOp {
            left,
            op: Operator::Div,
            right,
            ..
        }) => {
            let mut path = path_of(checker, left)?;
            push_join(checker, &mut path, right)?;
            Some(path)
        }
        Expr::Call(call) => {
            let (last, receiver) = match call.func.as_ref() {
                Expr::Name(name) => (name.id.as_str(), None),
                Expr::Attribute(attribute) => (attribute.attr.as_str(), Some(&attribute.value)),
                _ => return None,
            };
            match last {
                "resolve" | "absolute" | "expanduser" if call.arguments.is_empty() => {
                    path_of(checker, receiver?)
                }
                "joinpath" => {
                    let mut path = path_of(checker, receiver?)?;
                    for argument in &*call.arguments.args {
                        push_join(checker, &mut path, argument)?;
                    }
                    Some(path)
                }
                "Path" | "PurePath" | "dirname" | "abspath" | "realpath" | "normpath" | "join" => {
                    let callee = checker.resolve_expr(&call.func)?;
                    let arguments = &*call.arguments.args;
                    match callee.segments() {
                        ["pathlib", "Path" | "PurePath"] => match arguments {
                            [] => Some(SourcePath {
                                anchor: Anchor::WorkingDirectory,
                                steps: Vec::new(),
                            }),
                            [first, rest @ ..] => {
                                let mut path = path_of(checker, first)?;
                                for argument in rest {
                                    push_join(checker, &mut path, argument)?;
                                }
                                Some(path)
                            }
                        },
                        ["os", "path", "dirname"] => {
                            let mut path = path_of(checker, arguments.first()?)?;
                            path.steps.push(PathStep::Parent);
                            Some(path)
                        }
                        ["os", "path", "abspath" | "realpath" | "normpath"] => {
                            path_of(checker, arguments.first()?)
                        }
                        ["os", "path", "join"] => {
                            let (first, rest) = arguments.split_first()?;
                            let mut path = path_of(checker, first)?;
                            for argument in rest {
                                push_join(checker, &mut path, argument)?;
                            }
                            Some(path)
                        }
                        _ => None,
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Appends `/ part` to `path`, where `part` is a relative path.
fn push_join<'a>(checker: &Checker<'a>, path: &mut SourcePath<'a>, part: &'a Expr) -> Option<()> {
    let part = path_of(checker, part)?;
    if part.anchor != Anchor::WorkingDirectory {
        return None;
    }
    path.steps.extend(part.steps);
    Some(())
}

// ---------------------------------------------------------------------------
// Sinks
// ---------------------------------------------------------------------------

impl PromptSink {
    pub fn returns_prompt(&self) -> bool {
        self.kind == SinkKind::Returns
    }
}

/// Finds the prompts passed to `call`, which goes to `sink`.
pub fn prompts_of_call<'a>(
    checker: &Checker<'a>,
    sink: &'static PromptSink,
    via: String,
    call: &'a ast::ExprCall,
) -> Vec<Prompt<'a>> {
    let mut prompts = Vec::new();
    let mut config_prompts = Vec::new();
    if sink.kind != SinkKind::Call {
        return prompts;
    }
    let mut push = |argument: &'static str, role: Role, value: &'a Expr| {
        match value {
            // `system_prompt=None` passes no prompt.
            Expr::NoneLiteral(_) => return,
            // `create_react_agent(prompt=SystemMessage("..."))`: the inner call is a
            // sink itself and reports the text.
            Expr::Call(inner) => {
                let resolved = checker.resolve_expr(&inner.func);
                if checker.prompt_sink(&inner.func, resolved.as_ref()).is_some() {
                    return;
                }
            }
            _ => {}
        }
        prompts.push(Prompt {
            framework: sink.framework,
            role,
            via: Some(via.clone()),
            argument,
            value,
            pieces: callable_text(checker, value).unwrap_or_else(|| text_or_dynamic(checker, value)),
            call: Some(call.range()),
        });
    };

    for (arg, role) in sink.args {
        match *arg {
            Keyword(name) => {
                if let Some(keyword) = call.arguments.find_keyword(name) {
                    for value in elements(&keyword.value) {
                        push(name, *role, value);
                    }
                }
            }
            Positional(index, name) => {
                let value = call
                    .arguments
                    .find_positional(index)
                    .or_else(|| call.arguments.find_keyword(name).map(|keyword| &keyword.value));
                if let Some(value) = value {
                    for value in elements(value) {
                        push(name, *role, value);
                    }
                }
            }
            MessageTuples => {
                let messages = call
                    .arguments
                    .find_positional(0)
                    .or_else(|| call.arguments.find_keyword("messages").map(|keyword| &keyword.value));
                let Some(Expr::List(ast::ExprList { elts, .. }) | Expr::Tuple(ast::ExprTuple { elts, .. })) =
                    messages
                else {
                    continue;
                };
                for message in elts {
                    let Expr::Tuple(ast::ExprTuple { elts: pair, .. }) = message else {
                        continue;
                    };
                    let [Expr::StringLiteral(role), value] = pair.as_slice() else {
                        continue;
                    };
                    let role = match role.value.to_str() {
                        "system" => System,
                        "human" | "user" => User,
                        _ => continue,
                    };
                    push("messages", role, value);
                }
            }
            Config(fields) => {
                // `config=self.agents_config["writer"]`
                let Some(keyword) = call.arguments.find_keyword("config") else {
                    continue;
                };
                let Expr::Subscript(subscript) = &keyword.value else {
                    continue;
                };
                let (Expr::Attribute(attribute), Expr::StringLiteral(key)) =
                    (subscript.value.as_ref(), subscript.slice.as_ref())
                else {
                    continue;
                };
                let Some(file) = checker.crewai_config_file(attribute) else {
                    continue;
                };
                for &field in fields {
                    config_prompts.push(Prompt {
                        framework: sink.framework,
                        role: *role,
                        via: Some(via.clone()),
                        argument: field,
                        value: &keyword.value,
                        pieces: vec![Piece::Config {
                            file,
                            key: key.value.to_str(),
                            field,
                            expr: &keyword.value,
                        }],
                        call: Some(call.range()),
                    });
                }
            }
        }
    }
    prompts.extend(config_prompts);
    prompts
}

/// `instructions=build`: most frameworks also take a function that returns the
/// prompt, so its text is what the function returns.
fn callable_text<'a>(checker: &Checker<'a>, value: &'a Expr) -> Option<Vec<Piece<'a>>> {
    let Expr::Name(name) = value else {
        return None;
    };
    let binding = checker.semantic.lookup_symbol(name.id.as_str())?;
    match checker.semantic.binding(binding).kind {
        // An imported function is found by the cross-file step, which treats a
        // function passed as a prompt as what it returns.
        BindingKind::FunctionDefinition => Some(vec![Piece::Returned {
            function: binding,
            expr: value,
        }]),
        _ => None,
    }
}

/// `("You are...", "Be brief.")` passed where one prompt is expected is several
/// prompts (pydantic-ai takes a sequence); anything else is one.
fn elements(value: &Expr) -> &[Expr] {
    match value {
        Expr::List(ast::ExprList { elts, .. }) | Expr::Tuple(ast::ExprTuple { elts, .. }) => elts,
        _ => std::slice::from_ref(value),
    }
}

/// `{"role": "system", "content": ...}`, the message format shared by the OpenAI,
/// Anthropic and LiteLLM chat APIs and most code that calls them.
pub fn prompt_of_message_dict<'a>(checker: &Checker<'a>, dict: &'a ast::ExprDict) -> Option<Prompt<'a>> {
    let mut role = None;
    let mut content = None;
    for item in &dict.items {
        let Some(Expr::StringLiteral(key)) = &item.key else {
            continue;
        };
        match key.value.to_str() {
            "role" => role = Some(&item.value),
            "content" => content = Some(&item.value),
            _ => {}
        }
    }
    let Expr::StringLiteral(role) = role? else {
        return None;
    };
    let role = match role.value.to_str() {
        "system" | "developer" => System,
        "user" => User,
        _ => return None,
    };
    let content = content?;
    Some(Prompt {
        framework: "messages",
        role,
        via: None,
        argument: "content",
        value: content,
        pieces: text_or_dynamic(checker, content),
        call: None,
    })
}

#[cfg(test)]
mod tests {
    use ruff_python_ast::PySourceType;
    use ruff_python_parser::parse_unchecked_source;

    use std::path::Path;

    use crate::checker::check_module;
    use crate::discovery::{discover, discover_prompts};
    use crate::project::FileContext;

    /// `(role, via, text, agent)` for each prompt in `source`.
    fn prompts(source: &str) -> Vec<(&'static str, String, String, Option<String>)> {
        let parsed = parse_unchecked_source(source, PySourceType::Python);
        assert!(parsed.has_valid_syntax(), "{:?}", parsed.errors());
        let checker = check_module(parsed.syntax(), vec!["app"]);
        let agents = discover(&checker);
        let context = FileContext {
            path: Path::new("app.py"),
            root: Path::new("."),
            module_path: &["app"],
            is_package: false,
            source,
        };
        discover_prompts(&checker, &agents, &context)
            .into_iter()
            .map(|prompt| {
                let via = match prompt.via {
                    Some(via) => format!("{via}({})", prompt.argument),
                    None => "dict".to_string(),
                };
                (prompt.role.as_str(), via, prompt.text, prompt.agent)
            })
            .collect()
    }

    fn texts(source: &str) -> Vec<String> {
        prompts(source).into_iter().map(|(_, _, text, _)| text).collect()
    }

    #[test]
    fn follows_constants_concatenation_and_fstrings() {
        let found = prompts(
            "from pydantic_ai import Agent\n\
             BASE = 'You review PRs.'\n\
             RULES = BASE + ' Be brief.'\n\
             RULES += ' No self-approval.'\n\
             reviewer = Agent('m', instructions=f'{RULES} Today is {date}.')\n",
        );
        assert_eq!(
            found,
            vec![(
                "system",
                "pydantic_ai.Agent(instructions)".to_string(),
                "You review PRs. Be brief. No self-approval. Today is {date}.".to_string(),
                Some("reviewer".to_string()),
            )]
        );
    }

    #[test]
    fn variables_hold_the_text_from_where_they_were_assigned() {
        // `BASE` is rebound after `PROMPT` is built from it, and inside `f` the name
        // is a parameter, so its text isn't known.
        assert_eq!(
            texts(
                "from agents import Agent\n\
                 BASE = 'first'\n\
                 PROMPT = BASE + '!'\n\
                 BASE = 'second'\n\
                 a = Agent(name='a', instructions=PROMPT)\n\
                 def f(PROMPT):\n    return Agent(name='b', instructions=PROMPT)\n",
            ),
            vec!["first!".to_string(), "{PROMPT}".to_string()]
        );
    }

    #[test]
    fn string_helpers_keep_their_text() {
        assert_eq!(
            texts(
                "import textwrap\nfrom crewai import Agent\n\
                 TEMPLATE = 'Score {x}.'\n\
                 Agent(role='r', goal=TEMPLATE.format(x=1), backstory=textwrap.dedent('  Hi'))\n\
                 Agent(role=' | '.join(['a', name]), goal='Hello %s' % who, backstory=1)\n",
            ),
            vec!["r", "Score {x}.", "  Hi", "a | {name}", "Hello %s", "{1}"]
        );
    }

    #[test]
    fn methods_on_client_objects() {
        let found = prompts(
            "import anthropic\nfrom openai import OpenAI\n\
             client = anthropic.Anthropic()\n\
             def ask(q):\n    return client.messages.create(system='Be terse.', messages=[])\n\
             OpenAI().responses.create(instructions='Hi', input=q)\n\
             other.messages.create(system='not a known client')\n",
        );
        let vias: Vec<_> = found.into_iter().map(|(role, via, _, _)| (role, via)).collect();
        assert_eq!(
            vias,
            vec![
                ("system", "anthropic.Anthropic.messages.create(system)".to_string()),
                ("system", "openai.OpenAI.responses.create(instructions)".to_string()),
                ("user", "openai.OpenAI.responses.create(input)".to_string()),
            ]
        );
    }

    #[test]
    fn message_dicts_and_tuples() {
        let found = prompts(
            "from langchain_core.prompts import ChatPromptTemplate\n\
             JUDGE = 'Grade {a}.'\n\
             m = [{'role': 'system', 'content': JUDGE.format(a=1)}, {'role': 'assistant', 'content': 'x'}]\n\
             t = ChatPromptTemplate.from_messages([('system', 'You are {p}.'), ('ai', 'ok'), ('human', '{q}')])\n",
        );
        let found: Vec<_> = found.into_iter().map(|(role, _, text, _)| (role, text)).collect();
        assert_eq!(
            found,
            vec![
                ("system", "Grade {a}.".to_string()),
                ("system", "You are {p}.".to_string()),
                ("user", "{q}".to_string()),
            ]
        );
    }

    #[test]
    fn decorated_functions_return_prompts() {
        let found = prompts(
            "from pydantic_ai import Agent\n\
             agent = Agent('m')\n\
             @agent.instructions\n\
             def who(ctx):\n    def helper():\n        return 'not this'\n    return f'User: {ctx.deps}'\n\
             agent.run_sync('Hello')\n",
        );
        let found: Vec<_> = found.into_iter().map(|(role, via, text, _)| (role, via, text)).collect();
        assert_eq!(
            found,
            vec![
                (
                    "system",
                    "pydantic_ai.Agent.instructions(return)".to_string(),
                    "User: {ctx.deps}".to_string()
                ),
                ("user", "pydantic_ai.Agent.run_sync(user_prompt)".to_string(), "Hello".to_string()),
            ]
        );
    }

    #[test]
    fn nested_sinks_report_once() {
        assert_eq!(
            texts(
                "from langgraph.prebuilt import create_react_agent\n\
                 from langchain_core.messages import SystemMessage\n\
                 a = create_react_agent(model, tools, prompt=SystemMessage('Be kind.'))\n",
            ),
            vec!["Be kind."]
        );
    }

    #[test]
    fn unrelated_strings_are_not_prompts() {
        assert!(
            texts(
                "import platform\nsystem = platform.system()\n\
                 prompt = 'Continue? [y/n]'\ninput(prompt)\n\
                 config = {'role': 'admin', 'content': 'x'}\n",
            )
            .is_empty()
        );
    }
}
