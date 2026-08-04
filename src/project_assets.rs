//! Agent-facing files that `project init` places into a project.
//!
//! The published binary carries these files, so a user who downloaded only the
//! binary still gets the half of the kit that agents read.
//!
//! `templates/` holds only what a project actually receives. The contract and
//! evidence templates that shipped with the retired Python CLI are gone: they
//! used a different record shape, and placing them left a project whose
//! `contracts/` directory could not load.

use std::fmt;

include!(concat!(env!("OUT_DIR"), "/embedded_assets.rs"));

const SKILL_SOURCE_PREFIX: &str = "skill-src/";
const SKILL_TARGET_PREFIX: &str = ".agents/skills/";
const AGENTS_BLOCK_ASSET: &str = "templates/AGENTS.block";
const GUIDE_ASSET: &str = "templates/docs/adf/README.md";
const GUIDE_TARGET: &str = "docs/adf/README.md";

pub const AGENTS_FILE: &str = "AGENTS.md";
pub const AGENTS_BLOCK_MARKER: &str = "<!-- adf:start -->";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingAsset(pub String);

impl fmt::Display for MissingAsset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the binary does not carry {}", self.0)
    }
}

/// A file to place, as a project-relative path and its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAsset {
    pub relative_path: String,
    pub contents: String,
}

/// Every file `project init` places, sorted by path.
///
/// `project_name` fills the placeholder that the guide uses to name the project.
pub fn planned_assets(project_name: &str) -> Result<Vec<PlannedAsset>, MissingAsset> {
    let mut planned = Vec::new();

    for (source_path, contents) in EMBEDDED_ASSETS {
        if let Some(rest) = source_path.strip_prefix(SKILL_SOURCE_PREFIX) {
            planned.push(PlannedAsset {
                relative_path: format!("{SKILL_TARGET_PREFIX}{rest}"),
                contents: render(contents, project_name),
            });
        }
    }
    if planned.is_empty() {
        return Err(MissingAsset(SKILL_SOURCE_PREFIX.to_owned()));
    }

    planned.push(PlannedAsset {
        relative_path: GUIDE_TARGET.to_owned(),
        contents: render(embedded(GUIDE_ASSET)?, project_name),
    });

    planned.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(planned)
}

/// The block appended to `AGENTS.md`.
pub fn agents_block(project_name: &str) -> Result<String, MissingAsset> {
    Ok(render(embedded(AGENTS_BLOCK_ASSET)?, project_name))
}

fn embedded(path: &str) -> Result<&'static str, MissingAsset> {
    EMBEDDED_ASSETS
        .iter()
        .find(|(candidate, _)| *candidate == path)
        .map(|(_, contents)| *contents)
        .ok_or_else(|| MissingAsset(path.to_owned()))
}

fn render(contents: &str, project_name: &str) -> String {
    contents.replace("{{PROJECT_NAME}}", project_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<String> {
        planned_assets("example")
            .unwrap()
            .into_iter()
            .map(|asset| asset.relative_path)
            .collect()
    }

    #[test]
    fn the_three_role_skills_are_placed() {
        let placed = paths();
        for skill in ["adf-analyst", "adf-builder", "adf-challenger"] {
            assert!(
                placed.contains(&format!(".agents/skills/{skill}/SKILL.md")),
                "{skill} was not placed"
            );
        }
    }

    #[test]
    fn the_reference_documents_travel_with_their_skills() {
        let placed = paths();
        for reference in [
            ".agents/skills/adf-analyst/references/contract-governance.md",
            ".agents/skills/adf-challenger/references/challenge-method.md",
        ] {
            assert!(
                placed.contains(&reference.to_owned()),
                "{reference} is missing"
            );
        }
    }

    #[test]
    fn the_project_guide_is_placed() {
        assert!(paths().contains(&GUIDE_TARGET.to_owned()));
    }

    /// Records shaped for the retired CLI would leave a project that cannot load.
    #[test]
    fn templates_of_the_retired_python_cli_are_not_placed() {
        for placed in paths() {
            assert!(
                !placed.starts_with(".adf/") && !placed.starts_with("contracts/"),
                "{placed} belongs to the retired CLI"
            );
        }
    }

    #[test]
    fn placed_files_carry_no_template_suffix_and_no_placeholder() {
        for asset in planned_assets("example").unwrap() {
            assert!(!asset.relative_path.ends_with(".tmpl"));
            assert!(
                !asset.contents.contains("{{PROJECT_NAME}}"),
                "{} keeps an unrendered project name",
                asset.relative_path
            );
        }
    }

    #[test]
    fn the_agents_block_carries_its_marker() {
        let block = agents_block("example").expect("the block is embedded");
        assert!(block.contains(AGENTS_BLOCK_MARKER));
        assert!(block.contains("$adf-analyst"));
    }
}
