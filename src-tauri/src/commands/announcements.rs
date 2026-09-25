use serde::{Deserialize, Serialize};

const ANNOUNCEMENTS_URL: &str =
    "https://raw.githubusercontent.com/StreamNook/StreamNook/main/announcements.json";

/// Live announcement payload served from the repo root. Edits to announcements.json
/// land in users' apps on the next poll without a release — used for situations
/// where users need to be told something but we cannot ship a new build to reach
/// them (e.g. broadcasting a recovery procedure to clients running an old binary).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnouncementsFile {
    pub version: u32,
    pub announcements: Vec<Announcement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announcement {
    pub id: String,
    pub severity: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub min_version: Option<String>,
    #[serde(default)]
    pub max_version: Option<String>,
    #[serde(default)]
    pub dismissible: Option<bool>,
    #[serde(default)]
    pub action: Option<AnnouncementAction>,
    // Announcements are often authored with flat action_label / action_url keys.
    // These are folded into `action` after fetch so the UI only reads one shape.
    #[serde(default)]
    pub action_label: Option<String>,
    #[serde(default)]
    pub action_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnouncementAction {
    pub label: String,
    pub url: String,
}

#[tauri::command]
pub async fn fetch_announcements() -> Result<AnnouncementsFile, String> {
    let mut builder = reqwest::Client::builder()
        .user_agent("StreamNook")
        .timeout(std::time::Duration::from_secs(10));

    if let Ok(token) = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
        builder = builder.default_headers(
            std::iter::once((
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
            ))
            .collect(),
        );
    }

    let client = builder.build().map_err(|e| e.to_string())?;

    let resp = client
        .get(ANNOUNCEMENTS_URL)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch announcements: {}", e))?;

    // 404 is the steady state when the repo has no announcements to broadcast.
    // Treat it as an empty list rather than an error so the UI stays clean.
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(AnnouncementsFile {
            version: 1,
            announcements: vec![],
        });
    }

    let mut file: AnnouncementsFile = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse announcements: {}", e))?;

    // Fold flat action_label / action_url authoring into the structured action
    // the frontend reads. Only fills in when a nested action wasn't provided.
    for a in &mut file.announcements {
        if a.action.is_none() {
            if let (Some(label), Some(url)) = (a.action_label.take(), a.action_url.take()) {
                a.action = Some(AnnouncementAction { label, url });
            }
        }
    }

    Ok(file)
}
