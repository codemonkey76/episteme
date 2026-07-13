//! Recipe Box tools — model-facing adapter over the user's self-hosted Recipe
//! Box (Laravel) via its Sanctum API (see `integrations::recipes`). Lets the
//! agent find/search recipes, create a new recipe, and produce a public
//! shareable link for one. Recipes are addressed by `slug`; resolve titles to
//! slugs with `recipe_find` first.

use anyhow::{anyhow, Result};
use reqwest::Method;
use serde_json::{json, Value};

use crate::integrations::recipes;
use crate::state::AppState;

pub fn schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "recipe_find",
            "description": "Search the user's Recipe Box, or list everything when no search is given. Matches recipe title, category, subtitle, and tag names. Returns each match's title, slug, category, tags, whether it's publicly shared (plus its share_url if so), and an in-app url. Use this to find a recipe's slug before sharing it, or to answer 'what recipes do I have?' / 'do I have anything with chicken?'.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "search": { "type": "string", "description": "Optional text to match against title, category, subtitle, or tag. Omit to list all recipes." }
                }
            }
        }),
        json!({
            "name": "recipe_create",
            "description": "Create a new recipe in the user's Recipe Box. Only `title` is required; fill in as much structure as you have. Ingredients are organised into named groups (e.g. 'For the sauce'), each with a list of items that have a name and an optional amount. Steps and notes are plain strings. Returns the created recipe including its slug and in-app url.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "The recipe title." },
                    "category": { "type": "string", "description": "Short eyebrow/category, e.g. 'Main Course · Steak'." },
                    "subtitle": { "type": "string", "description": "A one- or two-sentence description." },
                    "source": { "type": "string", "description": "Attribution, e.g. whose kitchen it's from." },
                    "meta": {
                        "type": "array",
                        "description": "Small stat chips like Prep/Cook/Serves. Mark one as highlight:true to feature it.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string", "description": "e.g. 'Prep', 'Cook', 'Serves'." },
                                "value": { "type": "string", "description": "e.g. '20m', '2'." },
                                "highlight": { "type": "boolean" }
                            },
                            "required": ["label", "value"]
                        }
                    },
                    "ingredient_groups": {
                        "type": "array",
                        "description": "Ingredients grouped under optional headings.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "heading": { "type": "string", "description": "Optional group heading, e.g. 'For the steak'." },
                                "items": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "name": { "type": "string", "description": "Ingredient name." },
                                            "amount": { "type": "string", "description": "Optional amount, e.g. '2 tbsp'." }
                                        },
                                        "required": ["name"]
                                    }
                                }
                            }
                        }
                    },
                    "steps": { "type": "array", "items": { "type": "string" }, "description": "Ordered method steps." },
                    "notes": { "type": "array", "items": { "type": "string" }, "description": "Optional tips/notes." },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for grouping, e.g. ['Dinner','Beef']." }
                },
                "required": ["title"]
            }
        }),
        json!({
            "name": "recipe_share",
            "description": "Create (or return the existing) public shareable link for a recipe, so it can be viewed by anyone without logging in. Get the recipe's `slug` from recipe_find first. Returns the share_url.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "slug": { "type": "string", "description": "The recipe's slug (from recipe_find)." }
                },
                "required": ["slug"]
            }
        }),
    ]
}

pub fn handles(name: &str) -> bool {
    matches!(name, "recipe_find" | "recipe_create" | "recipe_share")
}

pub async fn execute(state: &AppState, user_id: &str, name: &str, args: Value) -> Result<Value> {
    // Which Recipe Box instance to use (name); resolved against the registry.
    let instance = args["integration"].as_str();
    match name {
        "recipe_find" => {
            let search = args["search"].as_str().unwrap_or("");
            let path = if search.is_empty() {
                "/recipes".to_string()
            } else {
                format!("/recipes?search={}", urlencoding::encode(search))
            };
            recipes::request(state, user_id, instance, Method::GET, &path, None).await
        }
        "recipe_create" => {
            let title = args["title"].as_str().filter(|s| !s.trim().is_empty());
            if title.is_none() {
                return Err(anyhow!("title is required"));
            }
            // Forward only the recognised recipe fields (never `integration`).
            let mut body = serde_json::Map::new();
            for key in [
                "title",
                "category",
                "subtitle",
                "source",
                "meta",
                "ingredient_groups",
                "steps",
                "notes",
                "tags",
            ] {
                if let Some(v) = args.get(key) {
                    if !v.is_null() {
                        body.insert(key.to_string(), v.clone());
                    }
                }
            }
            let res = recipes::request(
                state,
                user_id,
                instance,
                Method::POST,
                "/recipes",
                Some(&Value::Object(body)),
            )
            .await?;
            state
                .log("recipes", "info", format!(
                    "recipe created: {}",
                    res["recipe"]["title"].as_str().unwrap_or("?")
                ))
                .await;
            Ok(res)
        }
        "recipe_share" => {
            let slug = args["slug"].as_str().ok_or_else(|| anyhow!("slug is required"))?;
            recipes::request(
                state,
                user_id,
                instance,
                Method::POST,
                &format!("/recipes/{}/share", urlencoding::encode(slug)),
                None,
            )
            .await
        }
        other => Err(anyhow!("unknown recipes tool '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_only_recipe_tools() {
        assert!(handles("recipe_find"));
        assert!(handles("recipe_create"));
        assert!(handles("recipe_share"));
        assert!(!handles("phoneus_health"));
    }

    #[test]
    fn every_schema_has_a_name_and_input_schema() {
        for s in schemas() {
            assert!(s["name"].as_str().is_some_and(|n| n.starts_with("recipe_")));
            assert_eq!(s["input_schema"]["type"], "object");
        }
    }
}
