//! The README describes every agent gglint supports.

use gglint::harness::HARNESSES;

#[test]
fn the_readme_lists_every_agent_with_its_config_file() {
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    let section = readme
        .split("## Supported agents")
        .nth(1)
        .expect("the README has a Supported agents section");
    let section = section.split("\n## ").next().unwrap();
    for harness in HARNESSES {
        let name = harness.display_name();
        let row = section
            .lines()
            .find(|line| line.starts_with(&format!("| {name} ")))
            .unwrap_or_else(|| panic!("no row for {name} in the README's Supported agents table"));
        assert!(row.contains(&format!("`{}`", harness.mcp_file())), "{row}");
    }
}
