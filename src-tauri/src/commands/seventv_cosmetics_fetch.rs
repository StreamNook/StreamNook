// 7TV Global Cosmetics Fetch - Badges and Paints
// Fetches all available badges and paints from 7TV v4 GraphQL API

use log::debug;
use serde::{Deserialize, Serialize};

const SEVENTV_GQL_URL: &str = "https://7tv.io/v4/gql";

// ============================================================================
// DATA MODELS - Badges
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVGlobalBadge {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub images: Vec<SevenTVImage>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVImage {
    pub url: String,
    pub mime: Option<String>,
    pub scale: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    #[serde(rename = "frameCount")]
    pub frame_count: Option<i32>,
}

// ============================================================================
// DATA MODELS - Paints
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVGlobalPaint {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub data: Option<SevenTVPaintData>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVPaintData {
    pub layers: Vec<SevenTVPaintLayer>,
    pub shadows: Vec<SevenTVPaintShadow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVPaintLayer {
    pub id: String,
    pub opacity: f64,
    #[serde(rename = "ty")]
    pub layer_type: serde_json::Value, // Complex union type, keep as JSON
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVPaintShadow {
    pub color: SevenTVColor,
    #[serde(rename = "offsetX")]
    pub offset_x: f64,
    #[serde(rename = "offsetY")]
    pub offset_y: f64,
    pub blur: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVColor {
    pub hex: String,
    pub r: i32,
    pub g: i32,
    pub b: i32,
    pub a: i32,
}

// ============================================================================
// GRAPHQL QUERIES
// ============================================================================

const ALL_BADGES_QUERY: &str = r#"
query AllBadges {
  badges {
    badges {
      id
      name
      description
      tags
      images {
        url
        mime
        scale
        width
        height
        frameCount
      }
      updatedAt
    }
  }
}
"#;

const ALL_PAINTS_QUERY: &str = r#"
query AllPaints {
  paints {
    paints {
      id
      name
      description
      tags
      data {
        layers {
          id
          opacity
          ty {
            ... on PaintLayerTypeLinearGradient {
              __typename
              angle
              repeating
              stops { at color { hex r g b a } }
            }
            ... on PaintLayerTypeRadialGradient {
              __typename
              shape
              repeating
              stops { at color { hex r g b a } }
            }
            ... on PaintLayerTypeSingleColor {
              __typename
              color { hex r g b a }
            }
            ... on PaintLayerTypeImage {
              __typename
              images { url mime scale width height frameCount }
            }
          }
        }
        shadows {
          color { hex r g b a }
          offsetX
          offsetY
          blur
        }
      }
      updatedAt
    }
  }
}
"#;

// ============================================================================
// API FUNCTIONS
// ============================================================================

async fn execute_gql_query(query: &str) -> Result<serde_json::Value, String> {
    let client = crate::services::http::client().clone();

    let response = client
        .post(SEVENTV_GQL_URL)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "query": query }))
        .send()
        .await
        .map_err(|e| format!("7TV API request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("7TV API error: {}", response.status()));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse 7TV response: {}", e))?;

    // Check for GraphQL errors
    if let Some(errors) = json.get("errors") {
        if let Some(arr) = errors.as_array() {
            if !arr.is_empty() {
                return Err(format!("7TV GraphQL error: {:?}", errors));
            }
        }
    }

    Ok(json)
}

// ============================================================================
// TAURI COMMANDS
// ============================================================================

/// Fetch all 7TV global badges
#[tauri::command]
pub async fn get_all_seventv_badges() -> Result<Vec<SevenTVGlobalBadge>, String> {
    debug!("[7TV] Fetching all badges...");

    let json = execute_gql_query(ALL_BADGES_QUERY).await?;

    let badges: Vec<SevenTVGlobalBadge> = json
        .get("data")
        .and_then(|d| d.get("badges"))
        .and_then(|b| b.get("badges"))
        .and_then(|arr| arr.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|badge| serde_json::from_value(badge.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    debug!("[7TV] Fetched {} badges", badges.len());
    Ok(badges)
}

/// Fetch all 7TV global paints
#[tauri::command]
pub async fn get_all_seventv_paints() -> Result<Vec<SevenTVGlobalPaint>, String> {
    debug!("[7TV] Fetching all paints...");

    let json = execute_gql_query(ALL_PAINTS_QUERY).await?;

    let paints: Vec<SevenTVGlobalPaint> = json
        .get("data")
        .and_then(|d| d.get("paints"))
        .and_then(|p| p.get("paints"))
        .and_then(|arr| arr.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|paint| serde_json::from_value(paint.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    debug!("[7TV] Fetched {} paints", paints.len());
    Ok(paints)
}

// ============================================================================
// DATA MODELS - Paint usage (PotatBotat)
// ============================================================================

/// Global usage figures for a single paint, sourced from the PotatBotat API.
/// `id` matches the 7TV paint id so the frontend can join it against the paint
/// catalog. 7TV itself publishes no per-paint usage data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVPaintUsage {
    pub id: String,
    /// Users currently wearing the paint.
    pub user_count: u64,
    /// Users who own (have been granted) the paint.
    pub owners: u64,
}

const PAINT_USAGE_URL: &str = "https://api.potat.app/paints";

/// Fetch global paint usage counts.
///
/// Only paints the source has observed are returned (a subset of all 7TV
/// paints); paints with no entry have no known usage and are sorted last by the
/// UI. A failure here is non-fatal — the paints tab still works without it.
#[tauri::command]
pub async fn get_seventv_paint_usage() -> Result<Vec<SevenTVPaintUsage>, String> {
    debug!("[7TV] Fetching paint usage counts...");

    let client = crate::services::http::client().clone();
    let response = client
        .get(PAINT_USAGE_URL)
        .send()
        .await
        .map_err(|e| format!("Paint usage request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Paint usage API error: {}", response.status()));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse paint usage response: {}", e))?;

    let usage: Vec<SevenTVPaintUsage> = json
        .get("data")
        .and_then(|arr| arr.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| {
                    let id = entry.get("id")?.as_str()?.to_string();
                    let user_count = entry
                        .get("user_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let owners = entry.get("owners").and_then(|v| v.as_u64()).unwrap_or(0);
                    Some(SevenTVPaintUsage {
                        id,
                        user_count,
                        owners,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    debug!("[7TV] Fetched usage for {} paints", usage.len());
    Ok(usage)
}
