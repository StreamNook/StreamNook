use serde::{Deserialize, Serialize};
use tauri::command;

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphQLResponse {
    pub data: Option<serde_json::Value>,
    pub errors: Option<Vec<serde_json::Value>>,
    pub message: Option<String>,
}

/// Proxy GraphQL requests to 7TV API to bypass CORS restrictions
#[command]
pub async fn seventv_graphql(query: String) -> Result<GraphQLResponse, String> {
    let client = crate::services::http::client().clone();

    let response = client
        .post("https://7tv.io/v4/gql")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "query": query }))
        .send()
        .await
        .map_err(|e| format!("Failed to send request to 7TV: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("7TV API returned status: {}", response.status()));
    }

    let json: GraphQLResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse 7TV response: {}", e))?;

    // Cosmetics lookups are the one query whose FAILURE is silent: a bad platform
    // value or a rejected batch returns 200 with an errors array, and every user
    // in it simply renders unpainted. Report what was asked for and what came
    // back so that is visible instead of inferred.
    if query.contains("userByConnection") {
        let asked = query.matches("userByConnection").count();
        let kick = query.matches("platform: KICK").count();
        let errs = json
            .errors
            .as_ref()
            .map(|e| {
                e.iter()
                    .filter_map(|v| v.get("message").and_then(|m| m.as_str()))
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .unwrap_or_default();
        let resolved = json
            .data
            .as_ref()
            .and_then(|d| d.as_object())
            .map(|o| {
                o.values()
                    .filter(|v| !v.pointer("/userByConnection").map(|u| u.is_null()).unwrap_or(true))
                    .count()
            })
            .unwrap_or(0);
        if !errs.is_empty() {
            log::warn!(
                "[7TV] cosmetics query FAILED ({} asked, {} kick): {}",
                asked,
                kick,
                errs
            );
        } else {
            log::debug!(
                "[7TV] cosmetics {}/{} resolved ({} kick)",
                resolved,
                asked,
                kick
            );
        }
    }

    Ok(json)
}
