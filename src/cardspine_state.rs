use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{CardSpine, DocPosition, content_digest::content_digest};

const STATE_SCHEMA: &str = "sim.cardspine-state.v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CardSpineState {
    pub(crate) content_ids: BTreeMap<String, String>,
    pub(crate) lane_digests: BTreeMap<String, String>,
}

impl CardSpineState {
    pub(crate) fn from_parts(
        content_ids: BTreeMap<String, String>,
        lane_digests: BTreeMap<String, String>,
    ) -> Self {
        Self {
            content_ids,
            lane_digests,
        }
    }

    pub(crate) fn read(root: &Path) -> Result<Option<Self>, String> {
        let path = state_path(root);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(format!("read {}: {err}", path.display())),
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return Ok(None);
        };
        if value.get("schema").and_then(Value::as_str) != Some(STATE_SCHEMA) {
            return Ok(None);
        }
        let Some(content_ids) = string_map(&value, "content_ids") else {
            return Ok(None);
        };
        let Some(lane_digests) = string_map(&value, "lane_digests") else {
            return Ok(None);
        };
        Ok(Some(Self {
            content_ids,
            lane_digests,
        }))
    }

    pub(crate) fn write(&self, root: &Path) -> Result<(), String> {
        let path = state_path(root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create {}: {err}", parent.display()))?;
        }
        let value = serde_json::json!({
            "schema": STATE_SCHEMA,
            "content_ids": self.content_ids,
            "lane_digests": self.lane_digests,
        });
        let mut text = serde_json::to_string_pretty(&value)
            .map_err(|err| format!("serialize cardspine state: {err}"))?;
        text.push('\n');
        fs::write(&path, text).map_err(|err| format!("write {}: {err}", path.display()))
    }
}

pub(crate) fn lanes_to_reencode(spine: &CardSpine, state: &CardSpineState) -> Vec<DocPosition> {
    affected_positions(&spine.content_ids(), &state.content_ids)
}

pub(crate) fn lane_digest(contents: &str) -> String {
    format!("sha256:{}", content_digest(contents.as_bytes()))
}

pub(crate) fn file_lane_digest(root: &Path, lane: &str) -> Option<String> {
    fs::read(root.join(lane))
        .ok()
        .map(|bytes| format!("sha256:{}", content_digest(&bytes)))
}

pub(crate) fn state_path(root: &Path) -> PathBuf {
    root.join(".sim").join("cardspine-state.json")
}

fn affected_positions(
    current: &BTreeMap<String, String>,
    previous: &BTreeMap<String, String>,
) -> Vec<DocPosition> {
    let mut recipe_changed = false;
    let mut contract_changed = false;
    let keys = current
        .keys()
        .chain(previous.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    for key in keys {
        if current.get(&key) == previous.get(&key) {
            continue;
        }
        if key.starts_with("recipe/") {
            recipe_changed = true;
        } else if key.starts_with("repo-contract/") {
            contract_changed = true;
        } else {
            recipe_changed = true;
            contract_changed = true;
        }
    }

    let mut positions = Vec::new();
    if recipe_changed {
        positions.extend([
            DocPosition::AgentCards,
            DocPosition::CardIndex,
            DocPosition::HumanReadme,
        ]);
    }
    if contract_changed {
        positions.push(DocPosition::RepoContract);
    }
    positions
}

fn string_map(value: &Value, key: &str) -> Option<BTreeMap<String, String>> {
    value
        .get(key)?
        .as_object()?
        .iter()
        .map(|(map_key, map_value)| Some((map_key.clone(), map_value.as_str()?.to_owned())))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{Card, card_content_id};

    use super::*;

    fn card(id: &str, kind: &str, path: &str) -> Card {
        let mut data = BTreeMap::new();
        data.insert("path".to_owned(), path.to_owned());
        Card {
            id: id.to_owned(),
            kind: kind.to_owned(),
            data,
        }
    }

    fn contract(repo: &str) -> Card {
        let mut data = BTreeMap::new();
        data.insert("repo".to_owned(), repo.to_owned());
        Card {
            id: format!("repo-contract/{repo}"),
            kind: "repo-contract".to_owned(),
            data,
        }
    }

    #[test]
    fn unchanged_spine_reencodes_nothing() {
        let spine = CardSpine {
            cards: vec![
                card(
                    "recipe/sim-fixture/recipes/open/recipe.toml",
                    "recipe",
                    "recipes/open/recipe.toml",
                ),
                contract("sim-fixture"),
            ],
        };
        let state = CardSpineState::from_parts(spine.content_ids(), BTreeMap::new());

        assert!(lanes_to_reencode(&spine, &state).is_empty());
    }

    #[test]
    fn single_recipe_change_reencodes_only_recipe_positions() {
        let previous = Card {
            id: "recipe/sim-fixture/recipes/open/recipe.toml".to_owned(),
            kind: "recipe".to_owned(),
            data: [("path".to_owned(), "recipes/open/recipe.toml".to_owned())]
                .into_iter()
                .collect(),
        };
        let changed = Card {
            id: previous.id.clone(),
            kind: previous.kind.clone(),
            data: [("path".to_owned(), "recipes/changed/recipe.toml".to_owned())]
                .into_iter()
                .collect(),
        };
        let old_spine = CardSpine {
            cards: vec![previous, contract("sim-fixture")],
        };
        let new_spine = CardSpine {
            cards: vec![changed, contract("sim-fixture")],
        };
        assert_ne!(
            card_content_id(&old_spine.cards[0]),
            card_content_id(&new_spine.cards[0])
        );
        let state = CardSpineState::from_parts(old_spine.content_ids(), BTreeMap::new());

        assert_eq!(
            lanes_to_reencode(&new_spine, &state),
            vec![
                DocPosition::AgentCards,
                DocPosition::CardIndex,
                DocPosition::HumanReadme
            ]
        );
    }

    #[test]
    fn lane_digest_keeps_the_card_state_spelling() {
        assert_eq!(
            lane_digest("SIM\n"),
            "sha256:7040c16de1e23dddf77df8ff8043c2bee23b42b47a0f326e5e124ae9bc2178e0"
        );
    }
}
