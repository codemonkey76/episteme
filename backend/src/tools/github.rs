//! GitHub tools — read-only, model-facing adapter over the user's repos via
//! the GitHub API (see `integrations::github`). Lets the agent look at commits,
//! files, and pull requests so it can reference them (e.g. in a helpdesk
//! reply). `repo` is `owner/name`, or a bare name when a default owner is set.

use anyhow::{anyhow, Result};
use reqwest::Method;
use serde_json::{json, Value};

use crate::integrations::github;
use crate::state::AppState;

/// Per-file diff cap so one big patch can't blow the context budget.
const PATCH_MAX: usize = 6_000;
/// File-content cap for github_get_file.
const FILE_MAX: usize = 100_000;

pub fn schemas() -> Vec<Value> {
    let repo = json!({
        "type": "string",
        "description": "Repository as \"owner/name\" (or a bare name if a default owner is configured)."
    });
    vec![
        json!({
            "name": "github_get_commit",
            "description": "Fetch a commit in full: author, date, message, stats, and the per-file diff. Use the sha (full or short) from github_list_commits or given by the user. Returns the commit's web URL for citing.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "repo": repo,
                    "sha": { "type": "string", "description": "Commit SHA (full or abbreviated) or a branch/tag name." }
                },
                "required": ["repo", "sha"]
            }
        }),
        json!({
            "name": "github_list_commits",
            "description": "List recent commits on a repo, newest first. Filter by branch/tag (ref), file path, or author. Use this to find a commit before reading it.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "repo": repo,
                    "ref": { "type": "string", "description": "Branch, tag, or SHA to list from (default branch if omitted)." },
                    "path": { "type": "string", "description": "Only commits touching this file/directory path." },
                    "author": { "type": "string", "description": "Filter by author (GitHub login or email)." },
                    "limit": { "type": "integer", "description": "Max commits, default 15, max 50." }
                },
                "required": ["repo"]
            }
        }),
        json!({
            "name": "github_get_file",
            "description": "Read a text file from a repo at a given ref (branch/tag/commit). Returns the file contents and its web URL.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "repo": repo,
                    "path": { "type": "string", "description": "Path to the file within the repo." },
                    "ref": { "type": "string", "description": "Branch, tag, or commit SHA (default branch if omitted)." }
                },
                "required": ["repo", "path"]
            }
        }),
        json!({
            "name": "github_get_pull_request",
            "description": "Fetch a pull request: title, state, author, branches, body, and stats. Get its number from the user or a commit reference.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "repo": repo,
                    "number": { "type": "integer", "description": "Pull request number." }
                },
                "required": ["repo", "number"]
            }
        }),
        json!({
            "name": "github_search_commits",
            "description": "Search a repo's commit messages for a phrase (e.g. a ticket reference or bug keyword). Returns matching commits newest first.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "repo": repo,
                    "query": { "type": "string", "description": "Text to match in commit messages." }
                },
                "required": ["repo", "query"]
            }
        }),
    ]
}

pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "github_get_commit"
            | "github_list_commits"
            | "github_get_file"
            | "github_get_pull_request"
            | "github_search_commits"
    )
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("\n…[truncated]");
    out
}

/// Slim a commit object for the model: identity, message, stats, and per-file
/// diffs (patches clipped). Works for both the detail and list-item shapes.
fn slim_commit(c: &Value) -> Value {
    let commit = &c["commit"];
    let files: Vec<Value> = c["files"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|f| {
                    json!({
                        "filename": f["filename"],
                        "status": f["status"],
                        "additions": f["additions"],
                        "deletions": f["deletions"],
                        "patch": f["patch"].as_str().map(|p| clip(p, PATCH_MAX)),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "sha": c["sha"],
        "html_url": c["html_url"],
        "author": commit["author"]["name"],
        "date": commit["author"]["date"],
        "message": commit["message"],
        "stats": c["stats"],
        "files": files,
    })
}

pub async fn execute(state: &AppState, user_id: &str, name: &str, args: Value) -> Result<Value> {
    let repo = github::resolve_repo(state, user_id, args["repo"].as_str().unwrap_or("")).await?;
    match name {
        "github_get_commit" => {
            let sha = args["sha"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("sha is required"))?;
            let res = github::request(
                state,
                user_id,
                Method::GET,
                &format!("/repos/{repo}/commits/{}", urlencoding::encode(sha)),
                None,
            )
            .await?;
            Ok(json!({ "commit": slim_commit(&res) }))
        }
        "github_list_commits" => {
            let limit = args["limit"].as_u64().unwrap_or(15).clamp(1, 50);
            let mut path = format!("/repos/{repo}/commits?per_page={limit}");
            if let Some(r) = args["ref"].as_str().filter(|s| !s.is_empty()) {
                path.push_str(&format!("&sha={}", urlencoding::encode(r)));
            }
            if let Some(p) = args["path"].as_str().filter(|s| !s.is_empty()) {
                path.push_str(&format!("&path={}", urlencoding::encode(p)));
            }
            if let Some(a) = args["author"].as_str().filter(|s| !s.is_empty()) {
                path.push_str(&format!("&author={}", urlencoding::encode(a)));
            }
            let res = github::request(state, user_id, Method::GET, &path, None).await?;
            // List items omit per-file diffs; summarize each.
            let commits: Vec<Value> = res
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|c| {
                            json!({
                                "sha": c["sha"],
                                "html_url": c["html_url"],
                                "author": c["commit"]["author"]["name"],
                                "date": c["commit"]["author"]["date"],
                                "message": c["commit"]["message"],
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(json!({ "commits": commits }))
        }
        "github_get_file" => {
            let file_path = args["path"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("path is required"))?;
            let mut path = format!("/repos/{repo}/contents/{}", file_path.trim_start_matches('/'));
            if let Some(r) = args["ref"].as_str().filter(|s| !s.is_empty()) {
                path.push_str(&format!("?ref={}", urlencoding::encode(r)));
            }
            let res = github::request(state, user_id, Method::GET, &path, None).await?;
            if res["type"].as_str() != Some("file") {
                return Err(anyhow!("'{file_path}' is not a file (a directory or symlink?)"));
            }
            let size = res["size"].as_u64().unwrap_or(0);
            if size as usize > FILE_MAX {
                return Err(anyhow!("file is too large to read inline ({size} bytes)"));
            }
            // GitHub returns base64 with embedded newlines.
            let content = match res["encoding"].as_str() {
                Some("base64") => {
                    use base64::Engine;
                    let raw = res["content"].as_str().unwrap_or("").replace(['\n', '\r'], "");
                    match base64::engine::general_purpose::STANDARD.decode(raw) {
                        Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                        Err(_) => return Err(anyhow!("could not decode file content")),
                    }
                }
                _ => res["content"].as_str().unwrap_or("").to_string(),
            };
            Ok(json!({
                "path": res["path"],
                "html_url": res["html_url"],
                "size": size,
                "content": clip(&content, FILE_MAX),
            }))
        }
        "github_get_pull_request" => {
            let number = args["number"].as_i64().ok_or_else(|| anyhow!("number is required"))?;
            let res = github::request(state, user_id, Method::GET, &format!("/repos/{repo}/pulls/{number}"), None).await?;
            Ok(json!({
                "pull_request": {
                    "number": res["number"],
                    "title": res["title"],
                    "state": res["state"],
                    "merged": res["merged"],
                    "author": res["user"]["login"],
                    "base": res["base"]["ref"],
                    "head": res["head"]["ref"],
                    "html_url": res["html_url"],
                    "body": res["body"].as_str().map(|b| clip(b, PATCH_MAX)),
                    "commits": res["commits"],
                    "additions": res["additions"],
                    "deletions": res["deletions"],
                    "changed_files": res["changed_files"],
                }
            }))
        }
        "github_search_commits" => {
            let query = args["query"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("query is required"))?;
            let q = format!("repo:{repo} {query}");
            let res = github::request(
                state,
                user_id,
                Method::GET,
                &format!("/search/commits?q={}&per_page=15", urlencoding::encode(&q)),
                None,
            )
            .await?;
            let commits: Vec<Value> = res["items"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|c| {
                            json!({
                                "sha": c["sha"],
                                "html_url": c["html_url"],
                                "author": c["commit"]["author"]["name"],
                                "date": c["commit"]["author"]["date"],
                                "message": c["commit"]["message"],
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(json!({ "commits": commits }))
        }
        other => Err(anyhow!("unknown github tool '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_and_handles_line_up() {
        let names: Vec<String> =
            schemas().iter().map(|s| s["name"].as_str().unwrap().to_string()).collect();
        assert!(names.contains(&"github_get_commit".to_string()));
        for n in &names {
            assert!(handles(n), "{n} not handled");
        }
        assert!(!handles("github_push"));
    }

    #[test]
    fn slim_commit_keeps_identity_and_clips_patches() {
        let big = "x".repeat(PATCH_MAX + 500);
        let c = json!({
            "sha": "abc123",
            "html_url": "https://github.com/o/r/commit/abc123",
            "commit": { "author": { "name": "Shane", "date": "2026-06-08T00:00:00Z" }, "message": "Fix it" },
            "stats": { "total": 3 },
            "files": [ { "filename": "a.rs", "status": "modified", "additions": 2, "deletions": 1, "patch": big } ],
        });
        let s = slim_commit(&c);
        assert_eq!(s["sha"], "abc123");
        assert_eq!(s["author"], "Shane");
        assert_eq!(s["message"], "Fix it");
        let patch = s["files"][0]["patch"].as_str().unwrap();
        assert!(patch.len() < PATCH_MAX + 100 && patch.ends_with("[truncated]"));
    }
}
