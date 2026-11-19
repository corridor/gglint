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

#[test]
fn the_readme_lists_the_files_gglint_secrets_reads_for_every_agent() {
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    let section = readme
        .split("### Files it reads")
        .nth(1)
        .expect("the README has a Files it reads section");
    let section = section.split("\n#").next().unwrap();
    for harness in HARNESSES {
        let name = harness.display_name();
        let row = section
            .lines()
            .find(|line| line.starts_with(&format!("| {name} ")))
            .unwrap_or_else(|| panic!("no row for {name} in the README's Files it reads table"));
        let directories = harness
            .rule_directories()
            .iter()
            .map(|directory| format!("`{directory}/`"));
        let files = harness
            .other_files()
            .iter()
            .chain(harness.personal_files().iter().map(|(file, _)| file))
            .map(|file| format!("`{file}`"));
        for listed in files.chain(directories) {
            assert!(row.contains(&listed), "{listed} isn't in {name}'s row: {row}");
        }
    }
}
