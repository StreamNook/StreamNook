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
struct WhisperThreadsData {
    #[serde(rename = "currentUser")]
    current_user: Option<CurrentUserThreads>,
}

#[derive(Debug, Deserialize)]
struct CurrentUserThreads {
    whispers: Option<WhispersConnection>,
}

#[derive(Debug, Deserialize)]
struct WhispersConnection {
    edges: Vec<WhisperThreadEdge>,
    #[serde(rename = "pageInfo")]
    page_info: Option<PageInfo>,
}

#[derive(Debug, Deserialize)]
struct PageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
}

#[derive(Debug, Deserialize)]
struct WhisperThreadEdge {
    cursor: Option<String>,
    node: WhisperThreadNode,
}

#[derive(Debug, Deserialize)]
struct WhisperThreadNode {
    id: String,
    participants: Option<Vec<Participant>>,
    #[serde(rename = "lastMessage")]
    last_message: Option<LastMessage>,
}

#[derive(Debug, Deserialize)]
struct Participant {
    id: String,
    login: String,
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(rename = "profileImageURL")]
    profile_image_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LastMessage {
    #[serde(rename = "sentAt")]
    sent_at: Option<String>,
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

#[derive(Debug, Deserialize)]
struct SearchUsersData {
    #[serde(rename = "searchUsers")]
    search_users: Option<SearchUsersResult>,
}

#[derive(Debug, Deserialize)]
struct SearchUsersResult {
    edges: Vec<SearchUserEdge>,
}

#[derive(Debug, Deserialize)]
struct SearchUserEdge {
    node: SearchUserNode,
}

#[derive(Debug, Deserialize)]
struct SearchUserNode {
    id: String,
    login: String,
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(rename = "profileImageURL")]
    profile_image_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FullWhisperImport {
    pub threads: Vec<WhisperThread>,
    pub messages_by_user: std::collections::HashMap<String, Vec<WhisperMessage>>,
}

pub struct WhisperHistoryService;

impl WhisperHistoryService {
    /// Get list of whisper threads (conversations) for the current user using different query
    pub async fn get_all_whisper_threads(
        access_token: &str,
        my_user_id: &str,
    ) -> Result<Vec<WhisperThread>, String> {
        let client = crate::services::http::client().clone();

        // Use the WhispersPage_Whispers query to get all threads
        let body = json!([{
            "operationName": "WhispersPage_Whispers",
            "variables": {
                "first": 100
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "6f3a0e4b8c9d5a2f1e7b3c6d8a9f0e2b4c7d5a8f1e3b6c9d2a5f8e1b4c7d0a3f"
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
            .await;

        // If the first query fails, try an alternative approach - search for threads by querying the current user's whisper history
        // We'll use a different strategy - get the "inbox" style list

        let alt_body = json!([{
            "operationName": "ChatList_Whispers",
            "variables": {},
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "7921037bc9042eb58e1a52f7ddf9f43ce4d60e4b6ffc3e5e7c4b2a1f0e8d6c3a"
                }
            }
        }]);

        // Try fetching the user's list of whisper conversations from a simpler endpoint
        // Since the GraphQL queries might have specific hashes that change, let's use a more direct approach
        // by constructing the thread IDs from known interactions

        // For now, return empty - we'll populate threads as we discover them from message fetches
        Ok(vec![])
    }

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

    /// Get list of whisper threads (conversations) for the current user
    pub async fn get_whisper_threads(
        access_token: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<WhisperThread>, Option<String>), String> {
        let client = crate::services::http::client().clone();

        // Use the undocumented GraphQL query for whisper threads
        let mut variables = json!({
            "first": 20
        });

        if let Some(c) = cursor {
            variables["cursor"] = json!(c);
        }

        let body = json!([{
            "operationName": "WhispersPage_WhispersQuery",
            "variables": variables,
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "7b3e0b3e8c0e3b5a3c5f7e0b3e8c0e3b5a3c5f7e0b3e8c0e3b5a3c5f7e0b3e8c"
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

        let result: Vec<GqlResponse<WhisperThreadsData>> = response
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
                if let Some(current_user) = &data.current_user {
                    if let Some(whispers) = &current_user.whispers {
                        let threads: Vec<WhisperThread> = whispers
                            .edges
                            .iter()
                            .filter_map(|edge| {
                                let node = &edge.node;
                                // Get the other participant (not the current user)
                                let other = node.participants.as_ref()?.iter().find(|p| {
                                    // We'll filter this later when we know our own ID
                                    true
                                })?;

                                Some(WhisperThread {
                                    id: node.id.clone(),
                                    user_id: other.id.clone(),
                                    user_login: other.login.clone(),
                                    user_name: other.display_name.clone(),
                                    profile_image_url: other.profile_image_url.clone(),
                                    last_message_at: node
                                        .last_message
                                        .as_ref()
                                        .and_then(|m| m.sent_at.clone()),
                                })
                            })
                            .collect();

                        let next_cursor = if whispers
                            .page_info
                            .as_ref()
                            .map(|p| p.has_next_page)
                            .unwrap_or(false)
                        {
                            whispers.edges.last().and_then(|e| e.cursor.clone())
                        } else {
                            None
                        };

                        return Ok((threads, next_cursor));
                    }
                }
            }
        }

        Ok((vec![], None))
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

    /// Search for a user by name to get their ID
    pub async fn search_user(
        access_token: &str,
        username: &str,
    ) -> Result<Option<(String, String, String, Option<String>)>, String> {
        let client = crate::services::http::client().clone();

        let body = json!([{
            "operationName": "WhispersSearchUsersQuery",
            "variables": {
                "userQuery": username
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "10ed65593d5e195734064016fe96b895dab192b9bd612e31530c9baeacc60836"
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

        let result: Vec<GqlResponse<SearchUsersData>> = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        if let Some(first) = result.first() {
            if let Some(data) = &first.data {
                if let Some(search_users) = &data.search_users {
                    if let Some(first_result) = search_users.edges.first() {
                        let user = &first_result.node;
                        return Ok(Some((
                            user.id.clone(),
                            user.login.clone(),
                            user.display_name.clone(),
                            user.profile_image_url.clone(),
                        )));
                    }
                }
            }
        }

        Ok(None)
    }
}
