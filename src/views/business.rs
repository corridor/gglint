//! Tier 3, business: what each agent system is for, who it serves, what it decides,
//! what it relies on and what keeps it in check, built from the workflow tier and the
//! project's own words. No language model is involved, and nothing is guessed: every
//! element says where in the code it comes from and by which rule, and what the code
//! doesn't say is left out.
//!
//! | Element      | Where it comes from                                                         |
//! | ------------ | --------------------------------------------------------------------------- |
//! | system       | the GGX declaration of the function building the graph, else the graph      |
//! | actors       | role words ([`ACTORS`]) in the prompts and in the schemas' field descriptions |
//! | capabilities | the workflow's parts, with the values its router sends to each               |
//! | decisions    | enums named like a state key a part writes, and "One of: A, B" fields        |
//! | resources    | the models called, GGX-declared RAGs, search tools, and systems ([`SYSTEMS`]) named in state keys |
//! | controls     | guardrails, GGX-declared guardrail models, retries, and human review tiers   |
//!
//! The person who asks is the actor the prompts mention most. A field written for an
//! actor, `"for the reviewing nurse"` or `"the patient-facing message"`, sends that field
//! to them; one written `"not for the patient"` doesn't.

use rustc_hash::FxHashMap;

use crate::discovery::{FileResult, Location};
use crate::flow::{self, Step};
use crate::json::Json;
use crate::vocabulary::{self, Declaration, Enum};

use super::mermaid::{Flowchart, Shape, id};
use super::workflow::{Role, Workflow, workflows};
use super::{last_segment, qualify, relative_path};

/// Words that name the people an agent system serves, by the role they're shown as. A
/// word in capitals only matches in capitals: `NP`, not `np`.
pub const ACTORS: &[(&str, &[&str])] = &[
    ("Patient", &["patient", "patients"]),
    (
        "Nurse",
        &[
            "nurse",
            "nurses",
            "nurse practitioner",
            "nurse practitioners",
            "NP",
            "NPs",
        ],
    ),
    ("Physician", &["physician", "physicians", "doctor", "doctors"]),
    ("Specialist", &["specialist", "specialists"]),
    ("Care team", &["care team", "clinical team"]),
    ("Customer", &["customer", "customers"]),
    ("Applicant", &["applicant", "applicants", "borrower", "borrowers"]),
    ("Underwriter", &["underwriter", "underwriters"]),
    ("Support agent", &["human agent", "support agent", "live agent"]),
    ("Reviewer", &["reviewer", "reviewers"]),
];

/// Systems a state key can name, by a word of the key: `epic_patient_summary`.
pub const SYSTEMS: &[(&str, &str)] = &[
    ("epic", "Epic"),
    ("mychart", "MyChart"),
    ("salesforce", "Salesforce"),
    ("hubspot", "HubSpot"),
    ("zendesk", "Zendesk"),
    ("servicenow", "ServiceNow"),
    ("jira", "Jira"),
    ("slack", "Slack"),
    ("stripe", "Stripe"),
    ("sap", "SAP"),
];

/// Who serves a chat model, by its package.
const PROVIDERS: &[(&str, &str)] = &[
    ("langchain_aws", "AWS Bedrock"),
    ("langchain_openai", "OpenAI"),
    ("langchain_anthropic", "Anthropic"),
    ("langchain_google_genai", "Google"),
    ("langchain_google_vertexai", "Google Vertex AI"),
    ("langchain_groq", "Groq"),
    ("langchain_mistralai", "Mistral"),
    ("langchain_ollama", "Ollama"),
];

/// Words in a function or tool name that say it looks something up.
const KNOWLEDGE_WORDS: &[&str] = &[
    "rag",
    "retrieve",
    "retriever",
    "search",
    "knowledge",
    "lookup",
    "document",
    "docs",
];

/// Values of a decision that mean a person looks at the outcome.
const REVIEW_WORDS: &[&str] = &["REVIEW", "APPROVAL", "TAKEOVER", "ESCALAT"];

/// The GGX declaration fields that describe a system, and how they're shown.
const PROFILE: &[(&str, &str)] = &[
    ("usecase_type", "Use case"),
    ("task_type", "Task"),
    ("impact", "Impact"),
    ("data_usage", "Data"),
    ("pipeline_type", "Interface"),
];

/// Where an element comes from.
#[derive(Clone, Debug)]
pub struct Evidence {
    pub path: String,
    pub line: usize,
    /// The rule that made it: `ggx-declaration`, `enum`, `field-audience`, ...
    pub rule: &'static str,
    /// What was read there.
    pub text: String,
}

pub struct Business<'w> {
    pub workflow: &'w Workflow<'w>,
    pub system: System,
    pub actors: Vec<Actor>,
    pub capabilities: Vec<Capability>,
    pub resources: Vec<Resource>,
    pub controls: Vec<Control>,
    pub flows: Vec<Flow>,
}

pub struct System {
    pub name: String,
    /// `("Use case", "Question Answering")`, ...
    pub profile: Vec<(&'static str, String)>,
    pub evidence: Vec<Evidence>,
}

pub struct Actor {
    pub name: String,
    /// Whether they ask, and whether they review what the system decides.
    pub asks: bool,
    pub reviews: bool,
    /// The schema fields written for them.
    pub receives: Vec<String>,
    pub evidence: Vec<Evidence>,
}

/// What a part of the workflow does for the business.
pub struct Capability {
    pub name: String,
    /// The workflow part, as an index into its parts.
    pub part: usize,
    /// What the router sends it, readable: `New case`, `Follow-up case`.
    pub handles: Vec<String>,
    pub decisions: Vec<Decision>,
    /// Indices into [`Business::resources`].
    pub uses: Vec<usize>,
    pub evidence: Vec<Evidence>,
}

pub struct Decision {
    pub name: String,
    pub values: Vec<String>,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Model,
    Knowledge,
    System,
}

impl ResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceKind::Model => "model",
            ResourceKind::Knowledge => "knowledge",
            ResourceKind::System => "system",
        }
    }
}

pub struct Resource {
    pub name: String,
    pub kind: ResourceKind,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    /// Checks what goes in or comes out.
    Guardrail,
    /// Sends work back with what was wrong.
    Retry,
    /// A person looks at the outcome.
    HumanReview,
}

impl ControlKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ControlKind::Guardrail => "guardrail",
            ControlKind::Retry => "retry",
            ControlKind::HumanReview => "human_review",
        }
    }
}

pub struct Control {
    pub name: String,
    pub kind: ControlKind,
    /// The workflow part it belongs to, when it's one.
    pub part: Option<usize>,
    pub evidence: Vec<Evidence>,
}

/// Something that moves between the elements: a request, a reply, work for review.
pub struct Flow {
    pub from: End,
    pub to: End,
    pub label: Option<String>,
    /// A resource used, rather than work handed on.
    pub uses: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Actor(usize),
    Capability(usize),
    Control(usize),
    Resource(usize),
}

/// The business view of every graph in `results`. `called` is what the functions the
/// graphs call in other modules declare.
pub(crate) fn businesses<'r>(
    results: &'r [FileResult],
    workflows: &'r [(&'r FileResult, Workflow<'r>)],
    called: &'r [Declaration],
) -> Vec<(&'r FileResult, Business<'r>)> {
    let declarations = Declarations::new(results, called);
    workflows
        .iter()
        .map(|(result, workflow)| (*result, build(result, workflow, &declarations)))
        .collect()
}

/// The declarations of a scan, by every suffix of their function's name.
struct Declarations<'r> {
    by_name: FxHashMap<&'r str, &'r Declaration>,
}

impl<'r> Declarations<'r> {
    fn new(results: &'r [FileResult], called: &'r [Declaration]) -> Self {
        let mut by_name = FxHashMap::default();
        let declared = results.iter().flat_map(|result| &result.vocabulary.declarations);
        for declaration in declared.chain(called) {
            for suffix in flow::suffixes(&declaration.function).filter(|suffix| suffix.contains('.')) {
                by_name.entry(suffix).or_insert(declaration);
            }
        }
        Self { by_name }
    }

    fn get(&self, function: &str) -> Option<&'r Declaration> {
        flow::suffixes(function)
            .filter(|suffix| suffix.contains('.'))
            .find_map(|suffix| self.by_name.get(suffix).copied())
    }
}

/// What a business view is built from, for one graph.
struct Builder<'w, 'r> {
    result: &'r FileResult,
    workflow: &'w Workflow<'w>,
    declarations: &'w Declarations<'r>,
    path: String,
    actors: Vec<Actor>,
    resources: Vec<Resource>,
    controls: Vec<Control>,
}

fn build<'w>(result: &FileResult, workflow: &'w Workflow<'w>, declarations: &Declarations<'_>) -> Business<'w> {
    let mut builder = Builder {
        result,
        workflow,
        declarations,
        path: relative_path(result),
        actors: Vec::new(),
        resources: Vec::new(),
        controls: Vec::new(),
    };
    let system = builder.system();
    builder.find_actors();
    let mut capabilities = builder.capabilities();
    builder.find_controls(&capabilities);
    builder.find_systems(&mut capabilities);
    let flows = builder.flows(&capabilities);
    Business {
        workflow,
        system,
        actors: builder.actors,
        capabilities,
        resources: builder.resources,
        controls: builder.controls,
        flows,
    }
}

impl Builder<'_, '_> {
    fn evidence(&self, location: Location, rule: &'static str, text: impl Into<String>) -> Evidence {
        Evidence {
            path: self.path.clone(),
            line: location.line,
            rule,
            text: text.into(),
        }
    }

    /// The system: what the function the graph is built in declares, or the graph.
    fn system(&self) -> System {
        let graph = self.workflow.graph;
        let declaration = self
            .result
            .vocabulary
            .declarations
            .iter()
            .filter(|declaration| {
                declaration.location.line <= graph.location.line && graph.location.line <= declaration.end.line
            })
            .max_by_key(|declaration| declaration.location.line);
        let Some(declaration) = declaration.filter(|declaration| declaration.kind.is_some()) else {
            return System {
                name: humanize(graph.name()),
                profile: Vec::new(),
                evidence: vec![self.evidence(graph.location, "graph", format!("StateGraph in {}", graph.name()))],
            };
        };
        let name = declaration
            .field("name")
            .map_or_else(|| humanize(last_segment(&declaration.function)), str::to_string);
        let profile = PROFILE
            .iter()
            .filter_map(|(key, label)| {
                let (_, values) = declaration.fields.iter().find(|(field, _)| field == key)?;
                Some((*label, values.join(", ")))
            })
            .collect();
        let kind = declaration.kind.as_deref().unwrap_or("");
        System {
            name,
            profile,
            evidence: vec![self.evidence(
                declaration.location,
                "ggx-declaration",
                format!("@ggx.{kind}.declare on {}", last_segment(&declaration.function)),
            )],
        }
    }

    /// The actor who asks, from the prompts, and the actors fields are written for.
    fn find_actors(&mut self) {
        let mut asked: Vec<(usize, Location)> = vec![(0, Location { line: 0, column: 0 }); ACTORS.len()];
        for (location, prompt) in &self.result.prompts {
            for (actor, count) in mentions(&prompt.text) {
                if asked[actor].0 == 0 {
                    asked[actor].1 = *location;
                }
                asked[actor].0 += count;
            }
        }
        let asker = (0..ACTORS.len())
            .filter(|&actor| asked[actor].0 > 0)
            .max_by_key(|&actor| (asked[actor].0, std::cmp::Reverse(actor)));
        if let Some(actor) = asker {
            let (count, location) = asked[actor];
            let evidence = self.evidence(
                location,
                "prompt-mentions",
                format!("{count} mention(s) in the prompts"),
            );
            self.actor(ACTORS[actor].0, evidence).asks = true;
        }
        // Without prompts that name anyone, the system answers whoever a field is written
        // facing.
        let mut told = None;
        for schema in &self.result.vocabulary.schemas {
            for field in &schema.fields {
                let Some(description) = &field.description else {
                    continue;
                };
                for audience in audiences(description) {
                    let text = format!("{}.{}: {}", schema.name, field.name, audience.phrase);
                    let evidence = self.evidence(field.location, "field-audience", text);
                    let reviews = audience.reviews || field.name.contains("review");
                    if audience.facing && told.is_none() {
                        told = Some(ACTORS[audience.actor].0);
                    }
                    let actor = self.actor(ACTORS[audience.actor].0, evidence);
                    actor.reviews |= reviews;
                    push_new(&mut actor.receives, field.name.clone());
                }
            }
        }
        if self.actors.iter().any(|actor| actor.asks) {
            return;
        }
        match told.and_then(|name| self.actors.iter_mut().find(|actor| actor.name == name)) {
            Some(actor) => actor.asks = true,
            None => {
                let graph = self.workflow.graph;
                let evidence = self.evidence(graph.location, "graph-start", "whoever invokes the graph");
                self.actor("User", evidence).asks = true;
            }
        }
    }

    fn actor(&mut self, name: &str, evidence: Evidence) -> &mut Actor {
        let index = match self.actors.iter().position(|actor| actor.name == name) {
            Some(index) => index,
            None => {
                self.actors.push(Actor {
                    name: name.to_string(),
                    asks: false,
                    reviews: false,
                    receives: Vec::new(),
                    evidence: Vec::new(),
                });
                self.actors.len() - 1
            }
        };
        let actor = &mut self.actors[index];
        actor.evidence.push(evidence);
        actor
    }

    fn capabilities(&mut self) -> Vec<Capability> {
        let workflow = self.workflow;
        let mut capabilities = Vec::new();
        // What a router sends work to does something for the business, even a step.
        let routed = |part: usize| {
            workflow.handoffs.iter().any(|handoff| {
                handoff.to == part && matches!(workflow.parts[handoff.from].role, Role::Router | Role::Supervisor)
            })
        };
        for (index, part) in workflow.parts.iter().enumerate() {
            let serves = matches!(
                part.role,
                Role::Router | Role::Supervisor | Role::Agent | Role::Responder | Role::Subgraph
            );
            if !serves && !(part.role == Role::Step && routed(index)) {
                continue;
            }
            let decisions = self.decisions(index);
            let name = match part.role {
                Role::Router => match decisions.first() {
                    Some(decision) => format!("Classify {}", decision.name.to_lowercase()),
                    None => "Route requests".to_string(),
                },
                _ => part.name.strip_suffix(" agent").unwrap_or(&part.name).to_string(),
            };
            let node = workflow.graph.nodes.iter().find(|node| node.name == part.members[0]);
            let evidence = node.map(|node| {
                let text = format!("{} part: {}", part.role.as_str(), part.members.join(", "));
                self.evidence(node.location, "workflow-part", text)
            });
            let capability = Capability {
                name,
                part: index,
                handles: self.handles(index),
                decisions,
                uses: self.uses(index),
                evidence: evidence.into_iter().collect(),
            };
            capabilities.push(capability);
        }
        capabilities
    }

    /// What the router sends to part `part`, in the words of the enums it tests.
    fn handles(&self, part: usize) -> Vec<String> {
        let workflow = self.workflow;
        let mut handles = Vec::new();
        for handoff in workflow.handoffs.iter().filter(|handoff| handoff.to == part) {
            if !matches!(workflow.parts[handoff.from].role, Role::Router | Role::Supervisor) {
                continue;
            }
            for value in &handoff.when {
                push_new(&mut handles, self.readable_member(value));
            }
        }
        handles
    }

    /// An enum member as people read it: its value when that's written in words, or its
    /// name, humanized.
    fn readable_member(&self, member: &str) -> String {
        let value = self
            .result
            .vocabulary
            .enums
            .iter()
            .flat_map(|enumeration| &enumeration.members)
            .find(|(name, _)| name == member)
            .and_then(|(_, value)| value.as_deref());
        readable(value.unwrap_or(member))
    }

    /// The decisions a part makes: enums named like a state key it writes, and schema
    /// fields listing their values ("One of: A, B") that it writes. A router that writes
    /// the key makes the decision; the parts that also write it only carry it on.
    fn decisions(&self, part: usize) -> Vec<Decision> {
        let writes = self.writes(part);
        let decided_by_a_router = |key: &str| {
            self.workflow.parts.iter().enumerate().any(|(other, router)| {
                other != part
                    && matches!(router.role, Role::Router | Role::Supervisor)
                    && self.writes(other).iter().any(|write| writes_key(write, key))
            })
        };
        let mut decisions: Vec<Decision> = Vec::new();
        for enumeration in &self.result.vocabulary.enums {
            let key = snake_case(&enumeration.name);
            let written = writes.iter().find(|write| writes_key(write, &key));
            if let Some(written) = written {
                if !decided_by_a_router(&key) {
                    decisions.push(self.enum_decision(enumeration, written));
                }
            }
        }
        for schema in &self.result.vocabulary.schemas {
            for field in &schema.fields {
                let Some(description) = &field.description else {
                    continue;
                };
                let values = listed_values(description);
                let written = writes.iter().any(|write| writes_key(write, &field.name));
                let repeated = decisions.iter().any(|decision| decision.name == humanize(&field.name));
                if values.len() < 2 || !written || repeated || decided_by_a_router(&field.name) {
                    continue;
                }
                decisions.push(Decision {
                    name: humanize(&field.name),
                    values: values.iter().map(|value| readable(value)).collect(),
                    evidence: vec![self.evidence(
                        field.location,
                        "field-values",
                        format!("{}.{}", schema.name, field.name),
                    )],
                });
            }
        }
        decisions
    }

    fn enum_decision(&self, enumeration: &Enum, written: &str) -> Decision {
        let values = enumeration
            .members
            .iter()
            .map(|(name, value)| readable(value.as_deref().unwrap_or(name)))
            .collect();
        Decision {
            name: humanize(&snake_case(&enumeration.name)),
            values,
            evidence: vec![self.evidence(
                enumeration.location,
                "enum-written",
                format!("enum {} for the state key {written}", enumeration.name),
            )],
        }
    }

    /// The state keys the nodes of part `part` write.
    fn writes(&self, part: usize) -> Vec<&str> {
        let graph = self.workflow.graph;
        self.workflow.parts[part]
            .members
            .iter()
            .filter_map(|member| graph.nodes.iter().find(|node| &node.name == member))
            .flat_map(|node| node.writes.iter().map(String::as_str))
            .collect()
    }

    /// The resources part `part` uses: its models, and what it looks things up in.
    fn uses(&mut self, part: usize) -> Vec<usize> {
        let workflow = self.workflow;
        let graph = workflow.graph;
        let mut uses = Vec::new();
        for member in &workflow.parts[part].members {
            let Some(node) = graph.nodes.iter().find(|node| &node.name == member) else {
                continue;
            };
            for step in &node.steps {
                match step {
                    Step::Model(call) => {
                        let name = super::code::model_name(&call.model).to_string();
                        let provider = call.model.client.as_deref().and_then(|client| {
                            PROVIDERS
                                .iter()
                                .find(|(package, _)| client.split('.').next() == Some(*package))
                                .map(|(_, provider)| *provider)
                        });
                        let name = match provider {
                            Some(provider) => format!("{name} on {provider}"),
                            None => name,
                        };
                        let evidence = self.evidence(call.location, "model-call", super::code::model_name(&call.model));
                        push_new_index(&mut uses, self.resource(name, ResourceKind::Model, evidence));
                    }
                    Step::Call { name, location } => {
                        if let Some(resource) = self.knowledge(name, *location) {
                            push_new_index(&mut uses, resource);
                        }
                    }
                    _ => {}
                }
            }
        }
        let first = graph
            .nodes
            .iter()
            .find(|node| node.name == workflow.parts[part].members[0]);
        for tool in workflow.parts[part]
            .tools
            .iter()
            .filter(|tool| has_word(tool, KNOWLEDGE_WORDS))
        {
            let Some(node) = first else { break };
            let evidence = self.evidence(node.location, "search-tool", format!("{} can call {tool}", node.name));
            push_new_index(
                &mut uses,
                self.resource(humanize(tool), ResourceKind::Knowledge, evidence),
            );
        }
        uses
    }

    /// A function a node calls, as knowledge: a GGX-declared RAG, or a function whose
    /// name says it looks something up.
    fn knowledge(&mut self, function: &str, location: Location) -> Option<usize> {
        match self.declarations.get(function) {
            Some(declaration) if declaration.kind.as_deref() == Some("Rag") => {
                let name = declaration
                    .field("name")
                    .map_or_else(|| humanize(last_segment(function)), str::to_string);
                let text = declaration
                    .doc
                    .clone()
                    .unwrap_or_else(|| format!("@ggx.Rag.declare on {}", last_segment(function)));
                let evidence = declared(declaration, "ggx-rag", text);
                Some(self.resource(name, ResourceKind::Knowledge, evidence))
            }
            Some(declaration) if declaration.kind.is_some() => None,
            _ if has_word(last_segment(function), KNOWLEDGE_WORDS) => {
                let evidence = self.evidence(location, "knowledge-call", format!("calls {}", last_segment(function)));
                Some(self.resource(humanize(last_segment(function)), ResourceKind::Knowledge, evidence))
            }
            _ => None,
        }
    }

    fn resource(&mut self, name: String, kind: ResourceKind, evidence: Evidence) -> usize {
        match self
            .resources
            .iter()
            .position(|resource| resource.name == name && resource.kind == kind)
        {
            Some(index) => {
                if self.resources[index].evidence.len() < 3 {
                    self.resources[index].evidence.push(evidence);
                }
                index
            }
            None => {
                self.resources.push(Resource {
                    name,
                    kind,
                    evidence: vec![evidence],
                });
                self.resources.len() - 1
            }
        }
    }

    /// Systems named by the state keys the graph reads or writes, `epic_patient_summary` →
    /// Epic, used by the capabilities whose nodes do.
    fn find_systems(&mut self, capabilities: &mut [Capability]) {
        let workflow = self.workflow;
        for node in &workflow.graph.nodes {
            for key in node.reads.iter().chain(&node.writes) {
                for word in key.split('_') {
                    let word = word.to_ascii_lowercase();
                    let Some((_, system)) = SYSTEMS.iter().find(|(known, _)| *known == word) else {
                        continue;
                    };
                    let evidence = self.evidence(node.location, "state-key", format!("{} uses {key}", node.name));
                    let resource = self.resource(system.to_string(), ResourceKind::System, evidence);
                    let user = capabilities
                        .iter_mut()
                        .find(|capability| workflow.parts[capability.part].members.contains(&node.name));
                    if let Some(capability) = user {
                        push_new_index(&mut capability.uses, resource);
                    }
                }
            }
        }
    }

    /// Guardrails, the guardrail models parts call, retries, and human review.
    fn find_controls(&mut self, capabilities: &[Capability]) {
        let workflow = self.workflow;
        let graph = workflow.graph;
        for (index, part) in workflow.parts.iter().enumerate() {
            let first = graph.nodes.iter().find(|node| node.name == part.members[0]);
            // GGX-declared models named as guardrails, wherever they're called.
            let mut named = None;
            for name in &part.uses {
                let Some(declaration) = self.declarations.get(name) else {
                    continue;
                };
                let title = declaration.field("name").unwrap_or("");
                if declaration.kind.as_deref() == Some("Model") && title.to_lowercase().contains("guardrail") {
                    let evidence = declared(
                        declaration,
                        "ggx-guardrail",
                        format!("@ggx.Model.declare(name={title:?})"),
                    );
                    self.control(title.to_string(), ControlKind::Guardrail, Some(index), evidence);
                    named = Some(title);
                }
            }
            if part.role == Role::Guardrail && named.is_none() {
                if let Some(node) = first {
                    let evidence = self.evidence(
                        node.location,
                        "guardrail-role",
                        format!("{} checks the work of other parts", node.name),
                    );
                    self.control(part.name.clone(), ControlKind::Guardrail, Some(index), evidence);
                }
            }
        }
        // Retries, and when they stop.
        for handoff in workflow.handoffs.iter().filter(|handoff| handoff.retry) {
            let guardrail = handoff.from;
            if self
                .controls
                .iter()
                .any(|control| control.kind == ControlKind::Retry && control.part == Some(guardrail))
            {
                continue;
            }
            // A count compared on the way out, `failures >= 3`, or on the way back,
            // `failures < 3`.
            let conditions = |ends: bool| {
                workflow
                    .handoffs
                    .iter()
                    .filter(move |handoff| {
                        handoff.from == guardrail
                            && (workflow.parts[handoff.to].role == Role::End) == ends
                            && (ends || handoff.retry)
                    })
                    .flat_map(|handoff| &handoff.edges)
                    .flat_map(|&edge| graph.edges[edge].conditions.iter().flatten())
                    .map(move |condition| (condition, ends))
            };
            let limit = conditions(true)
                .chain(conditions(false))
                .find_map(|(condition, ends)| limit(condition, ends));
            let name = match limit {
                Some(limit) => format!("Retry with feedback, gives up after {limit} attempts"),
                None => "Retry with feedback".to_string(),
            };
            let location = graph.edges[handoff.edges[0]].location;
            let evidence = self.evidence(
                location,
                "retry",
                format!("{} sends work back", workflow.parts[guardrail].name),
            );
            self.control(name, ControlKind::Retry, Some(guardrail), evidence);
        }
        // Decisions whose values put a person in the loop.
        for capability in capabilities {
            for decision in &capability.decisions {
                if !decision.values.iter().any(|value| has_review_word(value)) {
                    continue;
                }
                let name = format!("Human review by {}", decision.name.to_lowercase());
                let evidence = decision.evidence[0].clone();
                self.control(name, ControlKind::HumanReview, Some(capability.part), evidence);
            }
        }
    }

    fn control(&mut self, name: String, kind: ControlKind, part: Option<usize>, evidence: Evidence) {
        if self
            .controls
            .iter()
            .any(|control| control.name == name && control.part == part)
        {
            return;
        }
        self.controls.push(Control {
            name,
            kind,
            part,
            evidence: vec![evidence],
        });
    }

    /// The request, the work handed between capabilities through the controls, the
    /// reply, what goes to reviewers, and the resources each capability uses.
    fn flows(&self, capabilities: &[Capability]) -> Vec<Flow> {
        let workflow = self.workflow;
        let capability_of = |part: usize| capabilities.iter().position(|capability| capability.part == part);
        let guardrail_of = |part: usize| {
            self.controls
                .iter()
                .position(|control| control.part == Some(part) && control.kind == ControlKind::Guardrail)
        };
        let end = |part: usize| -> Option<End> {
            capability_of(part)
                .map(End::Capability)
                .or_else(|| guardrail_of(part).map(End::Control))
        };
        let asker = self.actors.iter().position(|actor| actor.asks).map(End::Actor);
        let mut flows: Vec<Flow> = Vec::new();
        for handoff in &workflow.handoffs {
            let (from, to) = (&workflow.parts[handoff.from], &workflow.parts[handoff.to]);
            let flow = match (from.role, to.role) {
                (Role::Start, _) => asker
                    .zip(end(handoff.to))
                    .map(|(from, to)| (from, to, Some("request".to_string()))),
                (_, Role::End) => end(handoff.from)
                    .zip(asker)
                    .map(|(from, to)| (from, to, Some("reply".to_string()))),
                _ if handoff.retry => None,
                _ => end(handoff.from).zip(end(handoff.to)).map(|(from, to)| {
                    let handles = capability_of(handoff.to).map(|capability| &capabilities[capability].handles);
                    let label = handles.filter(|_| !handoff.when.is_empty()).map(|handles| {
                        let shown: Vec<&str> = handles.iter().take(3).map(String::as_str).collect();
                        let more = if handles.len() > 3 { ", …" } else { "" };
                        format!("{}{more}", shown.join(", "))
                    });
                    (from, to, label)
                }),
            };
            if let Some((from, to, label)) = flow {
                if !flows.iter().any(|flow| flow.from == from && flow.to == to) {
                    flows.push(Flow {
                        from,
                        to,
                        label,
                        uses: false,
                    });
                }
            }
        }
        // What goes to a reviewer: the fields written for them, from the capability that
        // writes them.
        for (index, actor) in self.actors.iter().enumerate().filter(|(_, actor)| actor.reviews) {
            for (capability_index, capability) in capabilities.iter().enumerate() {
                let writes = self.writes(capability.part);
                let sent: Vec<String> = actor
                    .receives
                    .iter()
                    .filter(|field| writes.iter().any(|write| writes_key(write, field)))
                    .map(|field| humanize(field).to_lowercase())
                    .collect();
                if !sent.is_empty() {
                    flows.push(Flow {
                        from: End::Capability(capability_index),
                        to: End::Actor(index),
                        label: Some(format!("for review: {}", sent.join(", "))),
                        uses: false,
                    });
                }
            }
        }
        // Each control, by what it checks: a capability, or for a retry, its guardrail.
        for (index, control) in self.controls.iter().enumerate() {
            let Some(part) = control.part else { continue };
            let checked = match control.kind {
                ControlKind::Retry => guardrail_of(part)
                    .filter(|&guardrail| guardrail != index)
                    .map(End::Control),
                _ => capability_of(part).map(End::Capability),
            };
            if let Some(checked) = checked {
                flows.push(Flow {
                    from: End::Control(index),
                    to: checked,
                    label: None,
                    uses: true,
                });
            }
        }
        for (index, capability) in capabilities.iter().enumerate() {
            for &resource in &capability.uses {
                flows.push(Flow {
                    from: End::Resource(resource),
                    to: End::Capability(index),
                    label: None,
                    uses: true,
                });
            }
        }
        flows
    }
}

/// Evidence from a declaration, in the file it's written in.
fn declared(declaration: &Declaration, rule: &'static str, text: String) -> Evidence {
    Evidence {
        path: declaration.path.clone(),
        line: declaration.location.line,
        rule,
        text,
    }
}

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

/// How many times each actor of [`ACTORS`] is named in `text`, by index.
fn mentions(text: &str) -> Vec<(usize, usize)> {
    let words = words(text);
    let mut counts = Vec::new();
    for (actor, (_, aliases)) in ACTORS.iter().enumerate() {
        let count: usize = aliases.iter().map(|alias| occurrences(&words, alias).count()).sum();
        if count > 0 {
            counts.push((actor, count));
        }
    }
    counts
}

/// An actor a field is written for, and the words that say so.
struct Audience {
    actor: usize,
    phrase: String,
    reviews: bool,
    /// Written facing them, `the patient-facing message`: what they're told.
    facing: bool,
}

/// The actors a field description says it's for: `the patient-facing message`, `written
/// for the reviewing nurse`, but not `not for the patient`.
fn audiences(description: &str) -> Vec<Audience> {
    let words = words(description);
    let mut found: Vec<Audience> = Vec::new();
    for (actor, (_, aliases)) in ACTORS.iter().enumerate() {
        for alias in *aliases {
            let length = alias.split(' ').count();
            for at in occurrences(&words, alias) {
                let before = &words[at.saturating_sub(3)..at];
                let after = words.get(at + length).copied();
                let facing = after.is_some_and(|word| word.eq_ignore_ascii_case("facing"));
                let lower: Vec<String> = before.iter().map(|word| word.to_ascii_lowercase()).collect();
                let addressed = lower
                    .iter()
                    .any(|word| matches!(word.as_str(), "for" | "to" | "shown" | "sent"));
                if lower.iter().any(|word| word == "not") || !(facing || addressed) {
                    continue;
                }
                let start = at.saturating_sub(3);
                let phrase = words[start..(at + length + usize::from(facing)).min(words.len())].join(" ");
                let reviews = lower.iter().any(|word| word.starts_with("review"));
                if !found.iter().any(|audience| audience.actor == actor) {
                    found.push(Audience {
                        actor,
                        phrase,
                        reviews,
                        facing,
                    });
                }
            }
        }
    }
    found
}

/// The words of `text`: runs of letters and digits.
fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect()
}

/// Where `alias`, one or more words, starts in `words`. Lower-case aliases match any
/// case; others match exactly.
fn occurrences<'w>(words: &'w [&'w str], alias: &'w str) -> impl Iterator<Item = usize> + 'w {
    let parts: Vec<&str> = alias.split(' ').collect();
    let exact = alias.chars().any(|c| c.is_ascii_uppercase());
    (0..words.len().saturating_sub(parts.len() - 1)).filter(move |&at| {
        parts.iter().enumerate().all(|(offset, part)| {
            let word = words[at + offset];
            if exact {
                word == *part
            } else {
                word.eq_ignore_ascii_case(part)
            }
        })
    })
}

/// The constants a description lists after "one of": `One of: NO_REVIEW, ASYNC_REVIEW`.
fn listed_values(description: &str) -> Vec<&str> {
    let lower = description.to_ascii_lowercase();
    let Some(at) = lower.find("one of") else {
        return Vec::new();
    };
    description[at + "one of".len()..]
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|value| value.chars().filter(char::is_ascii_uppercase).count() >= 2)
        .filter(|value| {
            value
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        })
        .collect()
}

/// Whether `write`, a state key, is `key` or ends with it: `general_medical_review_tier`
/// is a `review_tier`.
fn writes_key(write: &str, key: &str) -> bool {
    write == key || write.strip_suffix(key).is_some_and(|prefix| prefix.ends_with('_'))
}

/// After how many attempts a guardrail gives up, from a count compared on the edge out,
/// `failures >= 3` (`ends`), or on the edge back, `failures < 3`.
fn limit(condition: &str, ends: bool) -> Option<usize> {
    let number = |after: &str| -> Option<usize> {
        after
            .trim_start()
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    };
    let (operator, after) = [">=", "<=", ">", "<"]
        .iter()
        .find_map(|operator| condition.split_once(operator).map(|(_, after)| (*operator, after)))?;
    let count = number(after)?;
    match (operator, ends) {
        (">=", true) | ("<", false) => Some(count),
        (">", true) | ("<=", false) => Some(count + 1),
        _ => None,
    }
}

fn has_word(name: &str, words: &[&str]) -> bool {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| words.contains(&part.to_ascii_lowercase().as_str()))
}

fn has_review_word(value: &str) -> bool {
    let upper = value.to_ascii_uppercase();
    REVIEW_WORDS.iter().any(|word| upper.contains(word))
}

/// `MessageIntent` → `message_intent`.
fn snake_case(name: &str) -> String {
    let mut snake = String::with_capacity(name.len() + 4);
    for (index, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && index > 0 {
            snake.push('_');
        }
        snake.push(c.to_ascii_lowercase());
    }
    snake
}

/// Short words that are words, not abbreviations, in a constant: `NO_REVIEW`, `IN_PERSON`.
const SHORT_WORDS: &[&str] = &[
    "a", "an", "as", "at", "by", "in", "is", "it", "no", "of", "on", "or", "to", "up",
];

/// A value as people read it: `NEW_CASE` → `New case`, `911 SYMPTOMS` → `911 symptoms`,
/// `ONGOING_GENERAL_QA` → `Ongoing general QA`; a value already written in words,
/// `MyChart Help`, is kept.
fn readable(value: &str) -> String {
    if value.chars().any(|c| c.is_ascii_lowercase()) {
        return value.to_string();
    }
    let words: Vec<String> = value
        .split(['_', ' '])
        .filter(|word| !word.is_empty())
        .enumerate()
        .map(|(index, word)| {
            let lower = word.to_ascii_lowercase();
            if word.len() <= 2
                && !SHORT_WORDS.contains(&lower.as_str())
                && word.chars().any(|c| c.is_ascii_alphabetic())
            {
                word.to_string()
            } else if index == 0 {
                let mut capitalized = lower;
                capitalized[..1].make_ascii_uppercase();
                capitalized
            } else {
                lower
            }
        })
        .collect();
    words.join(" ")
}

/// `general_medical` → `General medical`.
fn humanize(name: &str) -> String {
    let words: Vec<String> = name
        .split('_')
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let mut text = words.join(" ");
    if let Some(first) = text.get(..1) {
        let upper = first.to_ascii_uppercase();
        text.replace_range(..1, &upper);
    }
    text
}

fn push_new(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

fn push_new_index(list: &mut Vec<usize>, value: usize) {
    if !list.contains(&value) {
        list.push(value);
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// How many values of a decision a capability lists before it's cut short.
const MAX_VALUES: usize = 4;

/// The business views as one Mermaid flowchart, left to right: who asks, the system's
/// capabilities and controls, who gets the outcome, and what the system relies on.
pub fn mermaid(results: &[FileResult]) -> String {
    let workflows = workflows(results);
    let called = vocabulary::called_declarations(results);
    let businesses = businesses(results, &workflows, &called);
    let mut chart = Flowchart::new_left_to_right();
    for (class, style) in [
        ("actor", "fill:#fff,stroke:#1d1d1f,stroke-width:2px"),
        ("capability", "fill:#e3f4ec,stroke:#2e9d6a"),
        ("control", "fill:#fde4ec,stroke:#c2416b"),
        ("resource", "fill:#e6f0fb,stroke:#2f6fdf"),
    ] {
        chart.class(class, style);
    }
    for (index, (_, business)) in businesses.iter().enumerate() {
        let prefix = if businesses.len() > 1 {
            format!("g{}_", index + 1)
        } else {
            String::new()
        };
        draw(&mut chart, business, &prefix);
    }
    chart.finish()
}

/// Draws one business view, left to right: the person who asks on the left, the system
/// in the middle, and on the right that person again, for the reply, and the reviewers.
/// The resources the system relies on are one box, joined to the system by one edge,
/// and each capability lists the ones it uses. Retries and human review are notes on what
/// they belong to. The JSON has every element and flow as it is.
fn draw(chart: &mut Flowchart, business: &Business<'_>, prefix: &str) {
    let reply = |index: usize| id(prefix, &format!("actor_{index}_reply"));
    let ids = |end: End| match end {
        End::Actor(index) => id(prefix, &format!("actor_{index}")),
        End::Capability(index) => id(prefix, &format!("capability_{index}")),
        End::Control(index) => id(prefix, &format!("control_{index}")),
        End::Resource(index) => id(prefix, &format!("resource_{index}")),
    };
    let notes = |kind: ControlKind, on: End| -> Vec<String> {
        business
            .flows
            .iter()
            .filter(|flow| flow.to == on)
            .filter_map(|flow| match flow.from {
                End::Control(control) if business.controls[control].kind == kind => {
                    Some(business.controls[control].name.clone())
                }
                _ => None,
            })
            .collect()
    };
    for (index, actor) in business.actors.iter().enumerate().filter(|(_, actor)| actor.asks) {
        chart.shaped_node(&ids(End::Actor(index)), &actor.name, &[], Shape::Stadium, Some("actor"));
    }
    // The title says what the system is for and who sees it; the JSON has the rest.
    let mut title = business.system.name.clone();
    for (label, value) in &business.system.profile {
        if matches!(*label, "Use case" | "Impact") {
            title.push_str(" · ");
            title.push_str(value);
        }
    }
    let system = id(prefix, "system");
    chart.begin_group(&system, &title);
    for (index, capability) in business.capabilities.iter().enumerate() {
        let mut details = Vec::new();
        for decision in &capability.decisions {
            let mut values: Vec<&str> = decision.values.iter().take(MAX_VALUES).map(String::as_str).collect();
            if decision.values.len() > MAX_VALUES {
                values.push("…");
            }
            details.push(format!(
                "decides {}: {}",
                decision.name.to_lowercase(),
                values.join(", ")
            ));
        }
        details.extend(notes(ControlKind::HumanReview, End::Capability(index)));
        // Models are in the box of resources; what a capability looks things up in is
        // what sets it apart.
        let uses: Vec<&str> = capability
            .uses
            .iter()
            .map(|&resource| &business.resources[resource])
            .filter(|resource| resource.kind != ResourceKind::Model)
            .map(|resource| resource.name.as_str())
            .collect();
        if !uses.is_empty() {
            details.push(format!("relies on: {}", uses.join(", ")));
        }
        let shape = if business.workflow.parts[capability.part].role == Role::Router {
            Shape::Hexagon
        } else {
            Shape::Round
        };
        chart.shaped_node(
            &ids(End::Capability(index)),
            &capability.name,
            &details,
            shape,
            Some("capability"),
        );
    }
    for (index, control) in business.controls.iter().enumerate() {
        if control.kind == ControlKind::Guardrail {
            let details = notes(ControlKind::Retry, End::Control(index));
            chart.shaped_node(
                &ids(End::Control(index)),
                &control.name,
                &details,
                Shape::Trapezoid,
                Some("control"),
            );
        }
    }
    chart.end_group();
    for (index, actor) in business.actors.iter().enumerate() {
        if actor.asks {
            chart.shaped_node(&reply(index), &actor.name, &[], Shape::Stadium, Some("actor"));
        } else if actor.reviews {
            chart.shaped_node(&ids(End::Actor(index)), &actor.name, &[], Shape::Stadium, Some("actor"));
        }
    }
    if !business.resources.is_empty() {
        let resources = id(prefix, "resources");
        chart.begin_group(&resources, "Relies on");
        for (index, resource) in business.resources.iter().enumerate() {
            chart.shaped_node(
                &ids(End::Resource(index)),
                &resource.name,
                &[],
                Shape::Cylinder,
                Some("resource"),
            );
        }
        chart.end_group();
        chart.edge(&resources, &system, None, true);
    }
    for flow in &business.flows {
        let to = match (flow.to, flow.label.as_deref()) {
            (End::Actor(index), Some("reply")) => reply(index),
            (to, _) => ids(to),
        };
        let drawn = match (flow.from, flow.to) {
            // Resources, retries and human review are drawn above.
            (End::Resource(_), _) => false,
            (End::Control(control), _) => business.controls[control].kind == ControlKind::Guardrail,
            _ => true,
        };
        if drawn {
            chart.edge(&ids(flow.from), &to, flow.label.as_deref(), flow.uses);
        }
    }
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

/// The business views as JSON, one per graph, every element with its `evidence`.
pub fn json(results: &[FileResult]) -> String {
    let workflows = workflows(results);
    let called = vocabulary::called_declarations(results);
    let businesses = businesses(results, &workflows, &called);
    let mut out = Json::with_capacity(4096);
    out.0.push_str("{\"schema\":1,\"tier\":\"business\",");
    out.begin_array("systems");
    for (result, business) in &businesses {
        write_business(&mut out, result, business);
    }
    out.end_array();
    out.end_object();
    out.0
}

fn write_evidence(out: &mut Json, evidence: &[Evidence]) {
    out.begin_array("evidence");
    for evidence in evidence {
        out.begin_object();
        out.string("path", &evidence.path);
        out.number("line", evidence.line);
        out.string("rule", evidence.rule);
        out.string("text", &evidence.text);
        out.close();
    }
    out.end_array();
}

fn write_business(out: &mut Json, result: &FileResult, business: &Business<'_>) {
    let graph = business.workflow.graph;
    let path = relative_path(result);
    let graph_id = qualify(result, &path, graph.name());
    let part_id = |part: usize| {
        let part = &business.workflow.parts[part];
        match part.role {
            Role::Start | Role::End => part.name.clone(),
            _ => format!("part:{graph_id}.{}", part.members[0]),
        }
    };
    let end = |end: End| match end {
        End::Actor(index) => format!("actor:{}", business.actors[index].name),
        End::Capability(index) => part_id(business.capabilities[index].part),
        End::Control(index) => format!("control:{}", business.controls[index].name),
        End::Resource(index) => format!("resource:{}", business.resources[index].name),
    };
    out.begin_object();
    out.string("graph", &format!("graph:{graph_id}"));
    out.string("name", &business.system.name);
    out.begin_array("profile");
    for (label, value) in &business.system.profile {
        out.begin_object();
        out.string("label", label);
        out.string("value", value);
        out.close();
    }
    out.end_array();
    write_evidence(out, &business.system.evidence);
    out.begin_array("actors");
    for actor in &business.actors {
        out.begin_object();
        out.string("id", &format!("actor:{}", actor.name));
        out.string("name", &actor.name);
        out.boolean("asks", actor.asks);
        out.boolean("reviews", actor.reviews);
        write_evidence(out, &actor.evidence);
        out.close();
    }
    out.end_array();
    out.begin_array("capabilities");
    for capability in &business.capabilities {
        out.begin_object();
        out.string("id", &part_id(capability.part));
        out.string("name", &capability.name);
        out.string("part", &business.workflow.parts[capability.part].name);
        out.strings("handles", capability.handles.iter().map(String::as_str));
        out.begin_array("decisions");
        for decision in &capability.decisions {
            out.begin_object();
            out.string("name", &decision.name);
            out.strings("values", decision.values.iter().map(String::as_str));
            write_evidence(out, &decision.evidence);
            out.close();
        }
        out.end_array();
        out.begin_array("uses");
        for &resource in &capability.uses {
            out.element(&format!("resource:{}", business.resources[resource].name));
        }
        out.end_array();
        write_evidence(out, &capability.evidence);
        out.close();
    }
    out.end_array();
    out.begin_array("resources");
    for resource in &business.resources {
        out.begin_object();
        out.string("id", &format!("resource:{}", resource.name));
        out.string("name", &resource.name);
        out.string("kind", resource.kind.as_str());
        write_evidence(out, &resource.evidence);
        out.close();
    }
    out.end_array();
    out.begin_array("controls");
    for control in &business.controls {
        out.begin_object();
        out.string("id", &format!("control:{}", control.name));
        out.string("name", &control.name);
        out.string("kind", control.kind.as_str());
        match control.part {
            Some(part) => out.string("part", &part_id(part)),
            None => out.null("part"),
        }
        write_evidence(out, &control.evidence);
        out.close();
    }
    out.end_array();
    out.begin_array("flows");
    for flow in &business.flows {
        out.begin_object();
        out.string("source", &end(flow.from));
        out.string("target", &end(flow.to));
        out.optional("label", flow.label.as_deref());
        out.string("kind", if flow.uses { "uses" } else { "hands_on" });
        out.close();
    }
    out.end_array();
    out.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audiences_are_who_a_field_is_written_for() {
        let names = |description: &str| -> Vec<(&str, bool)> {
            audiences(description)
                .iter()
                .map(|audience| (ACTORS[audience.actor].0, audience.reviews))
                .collect()
        };
        assert_eq!(names("<the patient-facing message, in markdown>"), [("Patient", false)]);
        assert_eq!(
            names("one short sentence, written for the reviewing nurse and not for the patient"),
            [("Nurse", true)]
        );
        assert_eq!(
            names("2-4 sentence summary for the reviewing NP covering what the patient asked"),
            [("Nurse", true)]
        );
        assert_eq!(names("The patient's age"), []);
    }

    #[test]
    fn words_for_values() {
        assert_eq!(readable("NEW_CASE"), "New case");
        assert_eq!(readable("ONGOING_GENERAL_QA"), "Ongoing general QA");
        assert_eq!(readable("NO_REVIEW"), "No review");
        assert!(writes_key("general_medical_review_tier", "review_tier"));
        assert!(!writes_key("tier", "review_tier"));
        assert!(!writes_key("subtier", "tier"));
        assert_eq!(readable("911 SYMPTOMS"), "911 symptoms");
        assert_eq!(readable("MyChart Help"), "MyChart Help");
        assert_eq!(snake_case("MessageIntent"), "message_intent");
        assert_eq!(
            listed_values("One of: NO_REVIEW, ASYNC_REVIEW, APPROVAL_REQUIRED"),
            ["NO_REVIEW", "ASYNC_REVIEW", "APPROVAL_REQUIRED"]
        );
        assert_eq!(limit("state.get(\"consecutive_failures\", 0) >= 3", true), Some(3));
        assert_eq!(limit("state['feedback'] and state['failures'] < 2", false), Some(2));
        assert_eq!(limit("state['tries'] > 4", true), Some(5));
        assert_eq!(limit("state['ok']", true), None);
        assert_eq!(
            mentions("The patient asks; patients wait. Nurse NP np"),
            [(0, 2), (1, 2)]
        );
    }
}
