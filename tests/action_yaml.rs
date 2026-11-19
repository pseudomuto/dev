//! Checks `action.yaml` gets what it caches from dev (through
//! `dev-action.sh cache-key`), not from paths copied into it. The steps
//! themselves only run in real CI.

use std::collections::BTreeMap;

use dev::Result;
use serde::Deserialize;

#[derive(Deserialize)]
struct Action {
    runs: Runs,
}

#[derive(Deserialize)]
struct Runs {
    steps: Vec<Step>,
}

#[derive(Deserialize)]
struct Step {
    name: String,
    #[serde(rename = "if")]
    condition: Option<String>,
    uses: Option<String>,
    #[serde(default)]
    with: BTreeMap<String, String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    run: Option<String>,
}

fn steps() -> Result<Vec<Step>> {
    let yaml = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/action.yaml"))?;
    Ok(serde_saphyr::from_str::<Action>(&yaml)?.runs.steps)
}

#[test]
fn cache_steps_take_their_paths_from_dev() -> Result<()> {
    let caches: Vec<Step> = steps()?
        .into_iter()
        .filter(|s| s.uses.as_deref().is_some_and(|u| u.starts_with("actions/cache/")))
        .collect();
    assert_eq!(4, caches.len(), "restore and save, for each of the two caches");

    for step in &caches {
        let brew = step.with["key"].starts_with("brew-");
        let want = if brew {
            "${{ steps.key.outputs.brew-paths }}"
        } else {
            "${{ steps.key.outputs.cache-paths }}"
        };
        assert_eq!(want, step.with["path"].trim(), "{}", step.name);
    }

    let brew_restore = caches
        .iter()
        .find(|s| s.uses.as_deref().is_some_and(|u| u.contains("/restore")) && s.with["key"].starts_with("brew-"))
        .expect("a brew restore step");
    let condition = brew_restore.condition.as_deref().unwrap_or_default();
    assert!(
        condition.contains("steps.key.outputs.brew-paths != ''"),
        "no brew paths (macOS) means no brew cache: {condition}"
    );
    Ok(())
}

#[test]
fn no_step_copies_where_dev_keeps_things() -> Result<()> {
    for step in steps()? {
        // Where the Action installs dev is its own choice, given to
        // install.sh; `dev action-paths` then reports wherever dev runs from.
        if step.name.starts_with("Install dev") {
            continue;
        }
        let text = [step.run.as_deref().unwrap_or_default()]
            .into_iter()
            .chain(step.with.values().map(String::as_str))
            .chain(step.env.keys().map(String::as_str))
            .chain(step.env.values().map(String::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        for copy in [
            "~/",
            "$HOME/.local",
            ".local/share",
            "MISE_DEFAULT_CONFIG_FILENAME",
            ".dev/mise.toml",
        ] {
            assert!(!text.contains(copy), "step {:?} copies {copy:?}:\n{text}", step.name);
        }
    }
    Ok(())
}
