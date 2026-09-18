use log::{error};
use serde::{Deserialize, Serialize};
use serde_json::json;

const GQL_URL: &str = "https://gql.twitch.tv/gql";
const CLIENT_ID: &str = env!("TWITCH_WEB_CLIENT_ID"); // Twitch's first-party client ID

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WhisperThread {
    pub id: String,
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    pub profile_image_url: Option<String>,
    pub last_message_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WhisperMessage {
    pub id: String,
    pub from_user_id: String,
    pub from_user_name: String,
    pub content: String,
    pub sent_at: String,
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct WhisperMessagesData {
    #[serde(rename = "whisperThread")]
    whisper_thread: Option<WhisperThreadMessages>,
}

#[derive(Debug, Deserialize)]
struct WhisperThreadMessages {
    messages: Option<MessagesConnection>,
}

#[derive(Debug, Deserialize)]
struct MessagesConnection {
    edges: Vec<MessageEdge>,
}

#[derive(Debug, Deserialize)]
struct MessageEdge {
    cursor: String,
    node: MessageNode,
}

#[derive(Debug, Deserialize)]
struct MessageNode {
    id: String,
    from: MessageFrom,
    content: MessageContent,
    #[serde(rename = "sentAt")]
    sent_at: String,
}

#[derive(Debug, Deserialize)]
struct MessageFrom {
    id: String,
    #[serde(rename = "displayName")]
    display_name: String,
}

#[derive(Debug, Deserialize)]
struct MessageContent {
    content: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FullWhisperImport {
    pub threads: Vec<WhisperThread>,
    pub messages_by_user: std::collections::HashMap<String, Vec<WhisperMessage>>,
}

pub struct WhisperHistoryService;

impl WhisperHistoryService {

    /// Import all whisper history by iterating through known thread IDs
    /// This tries to fetch threads by constructing thread IDs from a list of potential users
    pub async fn import_full_history(
        access_token: &str,
        my_user_id: &str,
        known_user_ids: Vec<String>,
    ) -> Result<FullWhisperImport, String> {
        let mut threads = Vec::new();
        let mut messages_by_user: std::collections::HashMap<String, Vec<WhisperMessage>> =
            std::collections::HashMap::new();

        for other_user_id in known_user_ids {
            // Fetch all messages for this thread with pagination
            let mut all_messages = Vec::new();
            let mut cursor: Option<String> = None;
            let mut attempts = 0;
            const MAX_PAGES: i32 = 50; // Limit to prevent infinite loops

            loop {
                if attempts >= MAX_PAGES {
                    break;
                }
                attempts += 1;

                match Self::get_whisper_messages(
                    access_token,
                    my_user_id,
                    &other_user_id,
                    cursor.as_deref(),
                )
                .await
                {
                    Ok((messages, next_cursor)) => {
                        if messages.is_empty() {
                            break;
                        }
                        all_messages.extend(messages);

                        if next_cursor.is_none() {
                            break;
                        }
                        cursor = next_cursor;
                    }
                    Err(e) => {
                        // Log error but continue with other users
                        error!(
                            "[WhisperHistory] Failed to fetch messages for user {}: {}",
                            other_user_id, e
                        );
                        break;
                    }
                }
            }

            if !all_messages.is_empty() {
                // Create a thread entry
                let first_msg = &all_messages[0];
                let other_name = if first_msg.from_user_id == my_user_id {
                    // Message was sent by us, so the other user is the recipient
                    // We don't have their name from this message, use ID as fallback
                    other_user_id.clone()
                } else {
                    first_msg.from_user_name.clone()
                };

                threads.push(WhisperThread {
                    id: format!(
                        "{}_{}",
                        if my_user_id < other_user_id.as_str() {
                            my_user_id
                        } else {
                            &other_user_id
                        },
                        if my_user_id > other_user_id.as_str() {
                            my_user_id
                        } else {
                            &other_user_id
                        }
                    ),
                    user_id: other_user_id.clone(),
                    user_login: other_name.to_lowercase(),
                    user_name: other_name,
                    profile_image_url: None,
                    last_message_at: all_messages.last().map(|m| m.sent_at.clone()),
                });

                messages_by_user.insert(other_user_id, all_messages);
            }
        }

        Ok(FullWhisperImport {
            threads,
            messages_by_user,
        })
    }

    /// Get whisper messages for a specific thread between current user and another user
    pub async fn get_whisper_messages(
        access_token: &str,
        my_user_id: &str,
        other_user_id: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<WhisperMessage>, Option<String>), String> {
        let client = crate::services::http::client().clone();

        // Thread ID is formatted as "{smaller_id}_{larger_id}"
        let thread_id = if my_user_id < other_user_id {
            format!("{}_{}", my_user_id, other_user_id)
        } else {
            format!("{}_{}", other_user_id, my_user_id)
        };

        let mut variables = json!({
            "id": thread_id
        });

        if let Some(c) = cursor {
            variables["cursor"] = json!(c);
        }

        let body = json!([{
            "operationName": "Whispers_Thread_WhisperThread",
            "variables": variables,
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "c11d356f7e2d8a2b7da3f90c11487414b7fb188649bafe331e93937a5da2310d"
                }
            }
        }]);

        let response = client
            .post(GQL_URL)
            .header("Client-ID", CLIENT_ID)
            .header("Authorization", format!("OAuth {}", access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(format!("Request failed ({}): {}", status, text));
        }

        let result: Vec<GqlResponse<WhisperMessagesData>> = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        if let Some(first) = result.first() {
            if let Some(errors) = &first.errors {
                if !errors.is_empty() {
                    return Err(format!("GraphQL error: {}", errors[0].message));
                }
            }

            if let Some(data) = &first.data {
                if let Some(thread) = &data.whisper_thread {
                    if let Some(messages) = &thread.messages {
                        let msgs: Vec<WhisperMessage> = messages
                            .edges
                            .iter()
                            .map(|edge| WhisperMessage {
                                id: edge.node.id.clone(),
                                from_user_id: edge.node.from.id.clone(),
                                from_user_name: edge.node.from.display_name.clone(),
                                content: edge.node.content.content.clone().unwrap_or_default(),
                                sent_at: edge.node.sent_at.clone(),
                                cursor: Some(edge.cursor.clone()),
                            })
                            .collect();

                        let next_cursor = messages.edges.last().map(|e| e.cursor.clone());

                        return Ok((msgs, next_cursor));
                    }
                }
            }
        }

        Ok((vec![], None))
    }

}
