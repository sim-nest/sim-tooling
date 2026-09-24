//! Build-side Card spine for documentation lanes.

use std::collections::BTreeMap;
use std::path::Path;

use crate::{
    content_digest::content_digest,
    simdoc::{collect_recipe_files, repo_name},
};

/// Content-id algorithm tag for build-side documentation Cards.
pub const CARD_CONTENT_ID_ALGORITHM: &str = "sim.card-id.sha256-v1";

/// One browsable documentation Card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Card {
    /// Stable subject id, for example `recipe/sim-cli/01-basics/open`.
    pub id: String,
    /// Open kind tag, for example `recipe` or `repo-contract`.
    pub kind: String,
    /// Normalized payload used by documentation lane projections.
    pub data: BTreeMap<String, String>,
}

/// The unified, deterministically ordered Card list for one repo.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CardSpine {
    /// Cards sorted by `(kind, id)` so encode order is stable across runs.
    pub cards: Vec<Card>,
}

impl CardSpine {
    /// Builds the Card spine for `repo_root`.
    pub fn for_repo(repo_root: &Path) -> Result<Self, String> {
        let repo = repo_name(repo_root);
        let recipes = collect_recipe_files(repo_root)?;
        let mut cards = recipes
            .into_iter()
            .map(|recipe| recipe_card(&repo, &recipe))
            .collect::<Vec<_>>();
        cards.push(repo_contract_card(&repo));
        cards.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(Self { cards })
    }

    /// Returns a map of Card id to content id.
    pub fn content_ids(&self) -> BTreeMap<String, String> {
        self.cards
            .iter()
            .map(|card| (card.id.clone(), card_content_id(card)))
            .collect()
    }
}

/// Returns the content-addressed id for one Card.
pub fn card_content_id(card: &Card) -> String {
    let canonical = canonical_card_bytes(card);
    format!(
        "{CARD_CONTENT_ID_ALGORITHM}:{}",
        content_digest(canonical.as_bytes())
    )
}

fn recipe_card(repo: &str, recipe: &str) -> Card {
    let mut data = BTreeMap::new();
    data.insert("path".to_owned(), recipe.to_owned());
    Card {
        id: format!("recipe/{repo}/{recipe}"),
        kind: "recipe".to_owned(),
        data,
    }
}

fn repo_contract_card(repo: &str) -> Card {
    let mut data = BTreeMap::new();
    data.insert("repo".to_owned(), repo.to_owned());
    data.insert("lane:target-doc".to_owned(), "target/doc/".to_owned());
    data.insert(
        "lane:agent-cards".to_owned(),
        "docs/agents/cards.jsonl".to_owned(),
    );
    data.insert("lane:human-docs".to_owned(), "docs/humans/".to_owned());
    data.insert(
        "lane:diagram-src".to_owned(),
        "docs/diagrams/src/".to_owned(),
    );
    data.insert(
        "lane:diagram-generated".to_owned(),
        "docs/diagrams/generated/".to_owned(),
    );
    data.insert(
        "contract:provenance".to_owned(),
        "docs/generated/provenance.json".to_owned(),
    );
    data.insert(
        "contract:repo".to_owned(),
        "docs/generated/repo-contract.json".to_owned(),
    );
    data.insert(
        "contract:rustdoc".to_owned(),
        "docs/generated/rustdoc-index.json".to_owned(),
    );
    data.insert(
        "contract:card-index".to_owned(),
        "docs/generated/card-index.json".to_owned(),
    );
    data.insert(
        "contract:feature-map".to_owned(),
        "docs/generated/feature-map.json".to_owned(),
    );
    Card {
        id: format!("repo-contract/{repo}"),
        kind: "repo-contract".to_owned(),
        data,
    }
}

fn canonical_card_bytes(card: &Card) -> String {
    let mut out = String::new();
    out.push_str("{\"data\":{");
    for (index, (key, value)) in card.data.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&json_string(key));
        out.push(':');
        out.push_str(&json_string(value));
    }
    out.push_str("},\"id\":");
    out.push_str(&json_string(&card.id));
    out.push_str(",\"kind\":");
    out.push_str(&json_string(&card.kind));
    out.push('}');
    out
}

fn json_string(input: &str) -> String {
    serde_json::to_string(input).expect("serializing a string cannot fail")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn card_with(fields: &[(&str, &str)]) -> Card {
        Card {
            id: "recipe/demo/open".to_owned(),
            kind: "recipe".to_owned(),
            data: fields
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn cardspine_identical_spines_have_equal_content_ids() {
        let spine = CardSpine {
            cards: vec![card_with(&[("title", "Open")])],
        };
        let same = CardSpine {
            cards: vec![card_with(&[("title", "Open")])],
        };

        assert_eq!(spine.content_ids(), same.content_ids());
    }

    #[test]
    fn cardspine_field_reorder_does_not_change_content_id() {
        let first = card_with(&[("title", "Open"), ("book", "basics")]);
        let second = card_with(&[("book", "basics"), ("title", "Open")]);

        assert_eq!(card_content_id(&first), card_content_id(&second));
    }

    #[test]
    fn cardspine_content_change_changes_content_id() {
        let first = card_with(&[("title", "Open")]);
        let second = card_with(&[("title", "Close")]);

        assert_ne!(card_content_id(&first), card_content_id(&second));
    }

    #[test]
    fn cardspine_fixture_recipe_has_stable_golden_id() {
        let card = card_with(&[("path", "recipes/demo/open/recipe.toml")]);

        assert_eq!(
            card_content_id(&card),
            "sim.card-id.sha256-v1:c5b6ad423c6ff98719c97608b124ae2590ffeaca2a946286a6ef5dffe48e349f"
        );
    }

    #[test]
    fn cardspine_for_repo_unifies_recipe_and_contract_cards() {
        let root = temp_repo_root();
        let recipe_dir = root.join("recipes").join("demo").join("open");
        fs::create_dir_all(&recipe_dir).unwrap();
        fs::write(recipe_dir.join("recipe.toml"), "name = \"open\"\n").unwrap();

        let spine = CardSpine::for_repo(&root).unwrap();

        assert_eq!(spine.cards.len(), 2);
        assert_eq!(spine.cards[0].kind, "recipe");
        assert_eq!(
            spine.cards[0].data.get("path").map(String::as_str),
            Some("recipes/demo/open/recipe.toml")
        );
        assert_eq!(spine.cards[1].kind, "repo-contract");
        assert!(spine.content_ids().contains_key(&spine.cards[0].id));

        let _ = fs::remove_dir_all(root);
    }

    fn temp_repo_root() -> std::path::PathBuf {
        let mut root = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        root.push(format!("sim-tooling-cardspine-test-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
