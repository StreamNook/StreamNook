//! Edits to the signed-in viewer's own Twitch profile: bio, display name,
//! profile picture and banner.
//!
//! These go through the twitch.tv web session the app's sign-in already holds
//! (Twitch's own GQL, as twitch.tv's settings page sends it), so they need no
//! Helix scope and never make anyone sign in again. Each operation's document
//! is inline: Twitch can change its private API, and a persisted hash would
//! fail with no word of what changed. Every refusal is logged with Twitch's
//! own error code.

use base64::Engine;
use serde::Serialize;

use crate::services::twitch_auth_service::TwitchAuthService;
use crate::services::twitch_web_gql;

/// The longest bio Twitch keeps.
pub const BIO_MAX_CHARS: usize = 300;

/// The largest picture or banner StreamNook sends (Twitch's own limit is 10 MB).
pub const IMAGE_MAX_BYTES: usize = 10 * 1024 * 1024;

const LOG_TAG: &str = "TwitchProfileEdit";

const UPDATE_USER: &str = "mutation UpdateUserProfile($input: UpdateUserInput!) { updateUser(input: $input) { user { id login displayName description } error { code } } }";

const CREATE_PICTURE_UPLOAD: &str = "mutation EditProfile_CreateProfileImageUploadURL($input: CreateProfileImageUploadURLInput!) { createProfileImageUploadURL(input: $input) { uploadID uploadURL error { code } } }";

const CREATE_BANNER_UPLOAD: &str = "mutation EditProfile_CreateProfileBannerImageUploadURL($input: CreateProfileBannerImageUploadURLInput!) { createProfileBannerImageUploadURL(input: $input) { uploadID uploadURL error { code } } }";

/// Which profile image an upload replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileImageKind {
    Picture,
    Banner,
}

impl ProfileImageKind {
    fn document(self) -> (&'static str, &'static str) {
        match self {
            Self::Picture => (CREATE_PICTURE_UPLOAD, "createProfileImageUploadURL"),
            Self::Banner => (CREATE_BANNER_UPLOAD, "createProfileBannerImageUploadURL"),
        }
    }
}

/// What Twitch stored after a bio or display-name edit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ProfileEditResult {
    pub display_name: String,
    pub description: String,
}

/// The bio as Twitch would take it: trimmed, at most `BIO_MAX_CHARS`.
pub fn checked_bio(text: &str) -> Result<String, String> {
    let bio = text.trim();
    if bio.chars().count() > BIO_MAX_CHARS {
        return Err(format!("A bio can be at most {BIO_MAX_CHARS} characters."));
    }
    Ok(bio.to_string())
}

/// A display name may only change the capitalization of the login.
pub fn checked_display_name(login: &str, name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.to_lowercase() != login.to_lowercase() {
        return Err("Only the capitalization of your name can change.".to_string());
    }
    Ok(name.to_string())
}

/// The upload format Twitch takes for these bytes, read from the file itself.
pub fn image_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("GIF")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("PNG")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("JPG")
    } else {
        None
    }
}

/// The sentence a viewer reads for one of Twitch's `updateUser` / upload error codes.
pub fn refusal_text(code: &str) -> String {
    match code {
        "DISPLAY_NAME_ONLY_CAP" => "Only the capitalization of your name can change.",
        "DISPLAY_NAME_CHANGE_AGAIN" => "Twitch lets you change your name again later.",
        "DISPLAY_NAME_TOO_LONG" | "DISPLAY_NAME_TOO_SHORT" => "That name is not the right length.",
        "DISPLAY_NAME_NOT_AVAILABLE" => "That name is not available.",
        "INVALID_CHARS_IN_DISPLAY_NAME" => "That name has characters Twitch does not allow.",
        "DESCRIPTION_TOO_LONG" => "That bio is too long for Twitch.",
        "DESCRIPTION_FAILED_MODERATION" | "DESCRIPTION_AUTOMOD_FAILED" => "Twitch did not accept that bio.",
        "REAUTH_NEEDED" => "Twitch wants you to confirm your password on twitch.tv before this change.",
        "TOO_MANY_REQUESTS" | "REQUESTS_THROTTLED" => "Too many changes at once. Try again in a moment.",
        _ => "Twitch did not save that change. Try again in a moment.",
    }
    .to_string()
}

/// The `updateUser` variables, with only the fields being changed.
pub fn update_user_body(user_id: &str, display_name: Option<&str>, description: Option<&str>) -> serde_json::Value {
    let mut input = serde_json::json!({ "userID": user_id });
    if let Some(name) = display_name {
        input["displayName"] = name.into();
    }
    if let Some(bio) = description {
        input["description"] = bio.into();
    }
    serde_json::json!({
        "operationName": "UpdateUserProfile",
        "query": UPDATE_USER,
        "variables": { "input": input },
    })
}

/// Reads an `updateUser` answer: Twitch's error code, or what it stored.
pub fn read_update_answer(json: &serde_json::Value) -> Result<ProfileEditResult, String> {
    let update = json.pointer("/data/updateUser");
    if let Some(code) = update.and_then(|u| u.pointer("/error/code")).and_then(|c| c.as_str()) {
        log::warn!("[{LOG_TAG}] updateUser refused: {code}");
        return Err(refusal_text(code));
    }
    let Some(user) = update.and_then(|u| u.get("user")).filter(|u| !u.is_null()) else {
        log::warn!("[{LOG_TAG}] updateUser gave no user: {json}");
        return Err(refusal_text(""));
    };
    Ok(ProfileEditResult {
        display_name: user["displayName"].as_str().unwrap_or_default().to_string(),
        description: user["description"].as_str().unwrap_or_default().to_string(),
    })
}

async fn update_user(
    auth: &TwitchAuthService,
    user_id: &str,
    display_name: Option<&str>,
    description: Option<&str>,
) -> Result<ProfileEditResult, String> {
    let token = twitch_web_gql::web_session_token(auth).await?;
    let json = twitch_web_gql::post(Some(&token), update_user_body(user_id, display_name, description), LOG_TAG).await?;
    read_update_answer(&json)
}

/// Sets the bio; answers what Twitch stored.
pub async fn update_bio(auth: &TwitchAuthService, user_id: &str, text: &str) -> Result<ProfileEditResult, String> {
    let bio = checked_bio(text)?;
    update_user(auth, user_id, None, Some(&bio)).await
}

/// Changes the capitalization of the display name; answers what Twitch stored.
pub async fn update_display_name(
    auth: &TwitchAuthService,
    user_id: &str,
    login: &str,
    name: &str,
) -> Result<ProfileEditResult, String> {
    let name = checked_display_name(login, name)?;
    update_user(auth, user_id, Some(&name), None).await
}

/// Uploads a new profile picture or banner. Twitch processes it for a few
/// seconds before the new image shows.
pub async fn upload_image(
    auth: &TwitchAuthService,
    user_id: &str,
    kind: ProfileImageKind,
    data_base64: &str,
) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|_| "That image could not be read.".to_string())?;
    if bytes.len() > IMAGE_MAX_BYTES {
        return Err("That image is larger than 10 MB.".to_string());
    }
    let format = image_format(&bytes).ok_or_else(|| "Use a PNG, JPG or GIF image.".to_string())?;
    let (query, field) = kind.document();
    let token = twitch_web_gql::web_session_token(auth).await?;
    let body = serde_json::json!({
        "query": query,
        "variables": { "input": { "userID": user_id, "format": format } },
    });
    let json = twitch_web_gql::post(Some(&token), body, LOG_TAG).await?;
    let answer = json.pointer(&format!("/data/{field}")).cloned().unwrap_or_default();
    if let Some(code) = answer.pointer("/error/code").and_then(|c| c.as_str()) {
        log::warn!("[{LOG_TAG}] {field} refused: {code}");
        return Err(refusal_text(code));
    }
    let Some(url) = answer["uploadURL"].as_str().filter(|u| !u.is_empty()) else {
        log::warn!("[{LOG_TAG}] {field} gave no upload URL: {json}");
        return Err(refusal_text(""));
    };
    let url = urlencoding::decode(url).map(|u| u.into_owned()).unwrap_or_else(|_| url.to_string());
    let resp = crate::services::http::client()
        .put(&url)
        .body(bytes)
        .send()
        .await
        .map_err(|e| {
            log::warn!("[{LOG_TAG}] image upload failed: {e}");
            "The image did not upload. Try again in a moment.".to_string()
        })?;
    if !resp.status().is_success() {
        log::warn!("[{LOG_TAG}] image upload answered {}", resp.status());
        return Err("The image did not upload. Try again in a moment.".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bio_is_trimmed_and_capped() {
        assert_eq!(checked_bio("  hi  ").unwrap(), "hi");
        assert!(checked_bio(&"a".repeat(300)).is_ok());
        assert!(checked_bio(&"a".repeat(301)).is_err());
        // Characters, not bytes.
        assert!(checked_bio(&"é".repeat(300)).is_ok());
    }

    #[test]
    fn display_name_changes_capitalization_only() {
        assert_eq!(checked_display_name("nookviewer", " NookViewer ").unwrap(), "NookViewer");
        assert!(checked_display_name("nookviewer", "NookViewer2").is_err());
        assert!(checked_display_name("nookviewer", "").is_err());
    }

    #[test]
    fn image_format_reads_the_file() {
        assert_eq!(image_format(b"GIF89a...."), Some("GIF"));
        assert_eq!(image_format(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0]), Some("PNG"));
        assert_eq!(image_format(&[0xff, 0xd8, 0xff, 0xe0]), Some("JPG"));
        assert_eq!(image_format(b"RIFF....WEBP"), None);
    }

    #[test]
    fn update_body_sends_only_the_changed_fields() {
        let bio = update_user_body("42", None, Some("hello"));
        assert_eq!(bio["variables"]["input"], serde_json::json!({"userID": "42", "description": "hello"}));
        assert!(bio["query"].as_str().unwrap().contains("updateUser(input: $input)"));
        let name = update_user_body("42", Some("Nook"), None);
        assert_eq!(name["variables"]["input"], serde_json::json!({"userID": "42", "displayName": "Nook"}));
    }

    #[test]
    fn update_answer_reads_codes_and_the_stored_user() {
        let refused = serde_json::json!({"data": {"updateUser": {"user": null, "error": {"code": "DISPLAY_NAME_ONLY_CAP"}}}});
        assert_eq!(read_update_answer(&refused).unwrap_err(), "Only the capitalization of your name can change.");
        let ok = serde_json::json!({"data": {"updateUser": {"user": {"id": "42", "login": "nook", "displayName": "Nook", "description": "hi"}, "error": null}}});
        assert_eq!(
            read_update_answer(&ok).unwrap(),
            ProfileEditResult { display_name: "Nook".into(), description: "hi".into() }
        );
        let empty = serde_json::json!({"errors": [{"message": "service error"}]});
        assert!(read_update_answer(&empty).is_err());
    }
}
