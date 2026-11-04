//! Mermaid flowcharts in the style of LangGraph's `draw_mermaid()`, so a diagram drawn
//! from the code looks like the one LangGraph draws at runtime:
//!
//! ```text
//! ---
//! config:
//!   flowchart:
//!     curve: linear
//! ---
//! graph TD;
//!     __start__([<p>__start__</p>]):::first
//!     agent("agent")
//!     __end__([<p>__end__</p>]):::last
//!     __start__ --> agent;
//!     agent -. &nbsp;continue&nbsp; .-> tools;
//!     classDef default fill:#f2f0ff,line-height:1.2
//!     classDef first fill-opacity:0
//!     classDef last fill:#bfb6fc
//! ```
//!
//! Conditional edges are dotted, as in LangGraph. Details under a node's name are
//! written the way LangGraph writes a node's metadata, after a rule in small italics.

use std::fmt::Write;

use crate::flow::{END, START};

pub(crate) struct Flowchart {
    out: String,
    /// `classDef` lines added to LangGraph's.
    classes: String,
}

/// How a node is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    Round,
    Rect,
    Hexagon,
    Subroutine,
    Trapezoid,
    Stadium,
}

impl Shape {
    fn delimiters(self) -> (&'static str, &'static str) {
        match self {
            Shape::Round => ("(", ")"),
            Shape::Rect => ("[", "]"),
            Shape::Hexagon => ("{{", "}}"),
            Shape::Subroutine => ("[[", "]]"),
            Shape::Trapezoid => ("[/", "\\]"),
            Shape::Stadium => ("([", "])"),
        }
    }
}

impl Flowchart {
    pub(crate) fn new() -> Self {
        Self {
            out: String::from("---\nconfig:\n  flowchart:\n    curve: linear\n---\ngraph TD;\n"),
            classes: String::new(),
        }
    }

    /// A node, drawn as LangGraph draws one. `id` must come from [`id`]. `details` are
    /// shown under the name.
    pub(crate) fn node(&mut self, id: &str, name: &str, details: &[String]) {
        self.shaped_node(id, name, details, Shape::Round, None);
    }

    /// A node of the given shape, styled by `class`, from [`Self::class`].
    pub(crate) fn shaped_node(&mut self, id: &str, name: &str, details: &[String], shape: Shape, class: Option<&str>) {
        if name == START {
            let _ = writeln!(self.out, "\t{id}([<p>{START}</p>]):::first");
            return;
        }
        if name == END {
            let _ = writeln!(self.out, "\t{id}([<p>{END}</p>]):::last");
            return;
        }
        let (open, close) = shape.delimiters();
        let _ = write!(self.out, "\t{id}{open}\"{}", escape(name));
        if !details.is_empty() {
            self.out.push_str("<hr/><small><em>");
            for (index, detail) in details.iter().enumerate() {
                if index > 0 {
                    self.out.push_str("<br>");
                }
                self.out.push_str(&escape(detail));
            }
            self.out.push_str("</em></small>");
        }
        self.out.push('"');
        self.out.push_str(close);
        if let Some(class) = class {
            self.out.push_str(":::");
            self.out.push_str(class);
        }
        self.out.push('\n');
    }

    /// A style for nodes, as a `classDef`: `class("router", "fill:#fdf1d6")`.
    pub(crate) fn class(&mut self, name: &str, style: &str) {
        let _ = writeln!(self.classes, "\tclassDef {name} {style}");
    }

    /// An edge between two ids from [`id`], dotted when it's conditional.
    pub(crate) fn edge(&mut self, from: &str, to: &str, label: Option<&str>, conditional: bool) {
        let _ = match (label, conditional) {
            (None, false) => writeln!(self.out, "\t{from} --> {to};"),
            (None, true) => writeln!(self.out, "\t{from} -.-> {to};"),
            (Some(label), false) => writeln!(self.out, "\t{from} -- &nbsp;{}&nbsp; --> {to};", escape(label)),
            (Some(label), true) => writeln!(self.out, "\t{from} -. &nbsp;{}&nbsp; .-> {to};", escape(label)),
        };
    }

    /// Starts a box around the nodes and edges that follow, until [`Self::end_group`].
    pub(crate) fn begin_group(&mut self, id: &str, title: &str) {
        let _ = writeln!(self.out, "\tsubgraph {id}[\"{}\"]", escape(title));
    }

    pub(crate) fn end_group(&mut self) {
        self.out.push_str("\tend\n");
    }

    pub(crate) fn finish(mut self) -> String {
        self.out.push_str(
            "\tclassDef default fill:#f2f0ff,line-height:1.2\n\
             \tclassDef first fill-opacity:0\n\
             \tclassDef last fill:#bfb6fc\n",
        );
        self.out.push_str(&self.classes);
        self.out
    }
}

/// A Mermaid id for `name`, with `prefix` in front to keep the ids of different graphs
/// apart: letters, digits and `_` are kept, anything else is written as `_` and its
/// code point. `end` alone would end a subgraph, so it gets a `_` after it.
pub(crate) fn id(prefix: &str, name: &str) -> String {
    let mut id = String::with_capacity(prefix.len() + name.len());
    id.push_str(prefix);
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            id.push(c);
        } else {
            let _ = write!(id, "_{:x}_", c as u32);
        }
    }
    if id.eq_ignore_ascii_case("end") {
        id.push('_');
    }
    id
}

/// Text that is safe inside a quoted Mermaid label.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => escaped.push_str("#quot;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\n' => escaped.push(' '),
            c => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_safe() {
        assert_eq!(id("", "triage_decision"), "triage_decision");
        assert_eq!(id("g1_", "a b"), "g1_a_20_b");
        assert_eq!(id("", "end"), "end_");
        assert_eq!(id("", "End"), "End_");
    }

    #[test]
    fn writes_langgraph_style() {
        let mut chart = Flowchart::new();
        chart.node("__start__", START, &[]);
        chart.node("agent", "agent", &["model: gpt-4o".to_string()]);
        chart.node("__end__", END, &[]);
        chart.edge("__start__", "agent", None, false);
        chart.edge("agent", "__end__", Some("done"), true);
        assert_eq!(
            chart.finish(),
            "---\nconfig:\n  flowchart:\n    curve: linear\n---\ngraph TD;\n\
             \t__start__([<p>__start__</p>]):::first\n\
             \tagent(\"agent<hr/><small><em>model: gpt-4o</em></small>\")\n\
             \t__end__([<p>__end__</p>]):::last\n\
             \t__start__ --> agent;\n\
             \tagent -. &nbsp;done&nbsp; .-> __end__;\n\
             \tclassDef default fill:#f2f0ff,line-height:1.2\n\
             \tclassDef first fill-opacity:0\n\
             \tclassDef last fill:#bfb6fc\n"
        );
    }
}
