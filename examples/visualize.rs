//! A throwaway viewer for the lineage graph. It isn't part of the library, which does
//! no UI: it writes one HTML page that draws the graph with Cytoscape.js.
//!
//! ```sh
//! cargo run --release --example visualize -- path/to/repo [out.html]
//! ```

use std::path::PathBuf;
use std::time::Instant;

use gglint::discovery::scan;
use gglint::graph;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().unwrap_or_else(|| ".".to_string()));
    let out = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("gglint-graph.html"));

    let start = Instant::now();
    let graph = graph::build(scan(std::slice::from_ref(&root), false));
    let json = graph.to_json();
    let elapsed = start.elapsed();

    let title = root.canonicalize().unwrap_or(root).display().to_string();
    // `</script>` in a prompt would end the script element early; `<\/` is the same
    // string in JSON.
    let html = PAGE
        .replace("__TITLE__", &title.replace('<', "&lt;"))
        .replace("__STATS__", &format!("scan + graph in {:.1} ms", elapsed.as_secs_f64() * 1000.0))
        .replace("__GRAPH__", &json.replace("</", "<\\/"));
    std::fs::write(&out, html).expect("write the page");
    println!("{}", out.display());
    eprintln!(
        "{} node(s), {} edge(s) in {:.1} ms",
        graph.nodes.len(),
        graph.edges.len(),
        elapsed.as_secs_f64() * 1000.0
    );
}

const PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>gglint lineage</title>
<script src="https://cdnjs.cloudflare.com/ajax/libs/cytoscape/3.30.2/cytoscape.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/dagre@0.8.5/dist/dagre.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/cytoscape-dagre@2.5.0/cytoscape-dagre.js"></script>
<style>
  :root { --bg:#f7f7f5; --panel:#fff; --text:#1d1d1f; --muted:#6e6e73; --line:#d9d9de;
          --agent:#2f6fdf; --prompt:#e08a1e; --source:#2e9d6a; }
  @media (prefers-color-scheme: dark) {
    :root { --bg:#151517; --panel:#1f1f22; --text:#ececef; --muted:#9a9aa1; --line:#36363b; }
  }
  * { box-sizing: border-box; }
  body { margin:0; font:13px/1.45 -apple-system, system-ui, sans-serif; background:var(--bg); color:var(--text);
         display:grid; grid-template-columns: 1fr 360px; grid-template-rows: auto 1fr; height:100vh; }
  header { grid-column: 1 / 3; display:flex; gap:14px; align-items:center; flex-wrap:wrap;
           padding:10px 16px; border-bottom:1px solid var(--line); background:var(--panel); }
  header h1 { font-size:14px; margin:0; font-weight:600; }
  header .path { color:var(--muted); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; max-width:40vw; }
  .legend span { display:inline-flex; align-items:center; gap:5px; margin-right:10px; }
  .dot { width:10px; height:10px; border-radius:50%; display:inline-block; }
  input[type=search] { padding:5px 8px; border:1px solid var(--line); border-radius:6px; background:var(--bg); color:var(--text); width:200px; }
  label { color:var(--muted); user-select:none; }
  #cy { min-height:0; }
  aside { border-left:1px solid var(--line); background:var(--panel); padding:14px 16px; overflow:auto; }
  aside h2 { font-size:14px; margin:0 0 4px; word-break:break-all; }
  aside .kind { text-transform:uppercase; font-size:11px; letter-spacing:.04em; font-weight:600; }
  aside dl { display:grid; grid-template-columns: auto 1fr; gap:3px 10px; margin:10px 0; }
  aside dt { color:var(--muted); } aside dd { margin:0; word-break:break-all; }
  aside pre { white-space:pre-wrap; word-break:break-word; background:var(--bg); border:1px solid var(--line);
              border-radius:6px; padding:8px; font:12px/1.4 ui-monospace, Menlo, monospace; max-height:40vh; overflow:auto; }
  aside ul { padding-left:16px; margin:4px 0; } aside li { cursor:pointer; } aside li:hover { text-decoration:underline; }
  .muted { color:var(--muted); }
  @media (max-width: 760px) { body { grid-template-columns: 1fr; grid-template-rows: auto 60vh auto; } aside { border-left:0; border-top:1px solid var(--line); } }
</style>
</head>
<body>
<header>
  <h1>gglint lineage</h1>
  <span class="path" title="__TITLE__">__TITLE__</span>
  <span class="legend">
    <span><i class="dot" style="background:var(--source)"></i>source</span>
    <span><i class="dot" style="background:var(--prompt)"></i>prompt</span>
    <span><i class="dot" style="background:var(--agent)"></i>agent</span>
  </span>
  <input type="search" id="search" placeholder="Filter by id, name or text">
  <label><input type="checkbox" id="isolated" checked> hide unlinked prompts</label>
  <span class="muted" id="counts"></span>
  <span class="muted">__STATS__</span>
</header>
<div id="cy"></div>
<aside id="details"><p class="muted">Click a node to see its text, location and neighbours.</p></aside>
<script type="application/json" id="graph-data">__GRAPH__</script>
<script>
const data = JSON.parse(document.getElementById('graph-data').textContent);
const css = n => getComputedStyle(document.documentElement).getPropertyValue(n).trim();
const byId = new Map(data.nodes.map(n => [n.id, n]));
const degree = new Map();
for (const e of data.edges) for (const id of [e.source, e.target]) degree.set(id, (degree.get(id) || 0) + 1);

const short = (s, n) => s.length > n ? s.slice(0, n - 1) + '…' : s;
function caption(n) {
  if (n.kind === 'prompt') return `${n.label}\n${short(n.text.replace(/\s+/g, ' '), 38)}`;
  if (n.kind === 'agent') return `${n.label}\n${n.framework}`;
  return `${n.label}\n${n.source_kind}`;
}
const elements = [
  ...data.nodes.map(n => ({ data: { id: n.id, kind: n.kind, caption: caption(n) } })),
  ...data.edges.map((e, i) => ({ data: { id: 'e' + i, source: e.source, target: e.target, kind: e.kind,
      caption: e.kind === 'feeds' ? `#${e.position}` : e.link } })),
];

cytoscape.use(cytoscapeDagre);
const cy = cytoscape({
  container: document.getElementById('cy'),
  elements,
  wheelSensitivity: 0.3,
  style: [
    { selector: 'node', style: { 'label': 'data(caption)', 'text-wrap': 'wrap', 'text-max-width': 190,
        'font-size': 10, 'color': css('--text'), 'text-valign': 'bottom', 'text-margin-y': 4,
        'width': 18, 'height': 18, 'border-width': 0 } },
    { selector: 'node[kind="agent"]', style: { 'background-color': css('--agent'), 'shape': 'round-rectangle', 'width': 26, 'height': 26 } },
    { selector: 'node[kind="prompt"]', style: { 'background-color': css('--prompt'), 'shape': 'diamond' } },
    { selector: 'node[kind="source"]', style: { 'background-color': css('--source'), 'shape': 'ellipse' } },
    { selector: 'edge', style: { 'width': 1.4, 'curve-style': 'bezier', 'target-arrow-shape': 'triangle',
        'arrow-scale': 0.8, 'line-color': css('--line'), 'target-arrow-color': css('--line'),
        'label': 'data(caption)', 'font-size': 8, 'color': css('--muted'), 'text-rotation': 'autorotate',
        'text-background-color': css('--bg'), 'text-background-opacity': 1, 'text-background-padding': 1 } },
    { selector: 'edge[kind="given_to"]', style: { 'line-color': css('--agent'), 'target-arrow-color': css('--agent'), 'width': 2 } },
    { selector: '.faded', style: { 'opacity': 0.12 } },
    { selector: 'node:selected', style: { 'border-width': 3, 'border-color': css('--text') } },
  ],
});

function layout() {
  cy.elements(':visible').layout({ name: 'dagre', rankDir: 'LR', nodeSep: 14, rankSep: 140, edgeSep: 6, animate: false }).run();
  cy.fit(cy.elements(':visible'), 30);
}
function applyFilters() {
  const q = document.getElementById('search').value.trim().toLowerCase();
  const hideIsolated = document.getElementById('isolated').checked;
  cy.batch(() => cy.nodes().forEach(node => {
    const n = byId.get(node.id());
    const isolated = !degree.get(n.id);
    const match = !q || n.id.toLowerCase().includes(q) || (n.text || '').toLowerCase().includes(q) || (n.label || '').toLowerCase().includes(q);
    node.style('display', (hideIsolated && isolated) || !match ? 'none' : 'element');
  }));
  const v = cy.nodes(':visible');
  document.getElementById('counts').textContent =
    `${v.filter('[kind="source"]').length} sources · ${v.filter('[kind="prompt"]').length} prompts · ${v.filter('[kind="agent"]').length} agents · ${cy.edges(':visible').length} edges (of ${data.nodes.length} nodes)`;
  layout();
}

const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
function show(id) {
  const n = byId.get(id), node = cy.getElementById(id);
  cy.elements().addClass('faded');
  node.closedNeighborhood().removeClass('faded');
  node.predecessors().removeClass('faded'); node.successors().removeClass('faded');
  cy.nodes().unselect(); node.select();
  const color = { agent: '--agent', prompt: '--prompt', source: '--source' }[n.kind];
  const skip = new Set(['id', 'kind', 'label', 'text']);
  const rows = Object.entries(n).filter(([k, v]) => !skip.has(k) && v !== null && v !== '')
    .map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join('');
  const list = (title, ids) => ids.length ? `<h3 style="font-size:12px;margin:12px 0 2px">${title}</h3><ul>${
    ids.map(i => `<li data-id="${esc(i)}">${esc(byId.get(i).kind)}: ${esc(byId.get(i).label)} <span class="muted">${esc(i)}</span></li>`).join('')}</ul>` : '';
  const ins = data.edges.filter(e => e.target === id).map(e => e.source);
  const outs = data.edges.filter(e => e.source === id).map(e => e.target);
  document.getElementById('details').innerHTML =
    `<div class="kind" style="color:var(${color})">${esc(n.source_kind || n.agent_kind || n.kind)}</div>
     <h2>${esc(n.label)}</h2><div class="muted">${esc(n.id)}</div>
     <dl>${rows}</dl>${n.text !== undefined ? `<pre>${esc(n.text)}</pre>` : ''}
     ${list('Fed by / given from', ins)}${list('Feeds / given to', outs)}`;
  document.querySelectorAll('#details li').forEach(li => li.onclick = () => show(li.dataset.id));
}
cy.on('tap', 'node', e => show(e.target.id()));
cy.on('tap', e => { if (e.target === cy) { cy.elements().removeClass('faded'); cy.nodes().unselect(); } });
document.getElementById('search').addEventListener('input', applyFilters);
document.getElementById('isolated').addEventListener('change', applyFilters);
applyFilters();
</script>
</body>
</html>
"##;
