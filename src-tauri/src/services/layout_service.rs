use crate::models::chat_layout::{LayoutResult, MessageSegment};
use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight,
};
use std::sync::Mutex;

pub struct LayoutService {
    font_system: Mutex<FontSystem>,
    swash_cache: Mutex<SwashCache>,
    config: Mutex<LayoutConfig>,
}

#[derive(Clone)]
pub struct LayoutConfig {
    pub width: f32,
    pub font_size: f32,
    pub message_spacing: f32,
    pub show_timestamps: bool,
}

impl LayoutService {
    pub fn new() -> Self {
        let mut font_system = FontSystem::new();

        // Load bundled Satoshi variable fonts - these contain all weights (300-900)
        // Variable fonts are used for both regular and italic, covering all weight needs
        let fonts = vec![
            include_bytes!("../../../src/assets/fonts/Satoshi-Variable.woff2") as &[u8],
            include_bytes!("../../../src/assets/fonts/Satoshi-VariableItalic.woff2") as &[u8],
        ];

        let db = font_system.db_mut();
        for font_data in fonts {
            db.load_font_data(font_data.to_vec());
        }

        Self {
            font_system: Mutex::new(font_system),
            swash_cache: Mutex::new(SwashCache::new()),
            config: Mutex::new(LayoutConfig {
                width: 300.0,
                font_size: 13.0,
                message_spacing: 8.0,
                show_timestamps: false,
            }),
        }
    }

    /// Reconstruct text from segments, replacing emotes/emoji with placeholder spaces
    /// This ensures the backend measures the same width the frontend will render
    fn reconstruct_text_from_segments(
        &self,
        segments: &[MessageSegment],
        font_size: f32,
    ) -> (String, bool) {
        // Emote is rendered as h-7 (28px) image in frontend
        // Emoji is rendered as h-5 (20px) image in frontend - sized to align with text
        // We approximate their widths with space characters
        // At 13px font size, each space is roughly ~4px wide
        // So 28px emote ≈ 7 spaces, 20px emoji ≈ 5 spaces
        let emote_placeholder = "       "; // 7 spaces ≈ 28px
        let emoji_placeholder = "     "; // 5 spaces ≈ 20px

        let mut text = String::new();
        let mut has_links = false;

        for segment in segments {
            match segment {
                MessageSegment::Text { content } => {
                    text.push_str(content);
                }
                MessageSegment::Link { content, .. } => {
                    has_links = true;
                    // URLs are long strings without spaces that browsers break mid-character
                    // due to CSS `break-words` / `overflow-wrap: break-word`
                    // cosmic-text doesn't break URLs the same way, so we insert artificial
                    // break points (spaces) every ~20 characters to simulate browser behavior
                    // This ensures the backend calculates similar line wrapping as the frontend
                    let url = content;
                    let break_interval = 20;
                    let mut char_count = 0;
                    for c in url.chars() {
                        text.push(c);
                        char_count += 1;
                        // Insert a space every N characters to allow cosmic-text to wrap
                        if char_count >= break_interval && c != ' ' {
                            text.push(' ');
                            char_count = 0;
                        }
                    }
                }
                MessageSegment::Emote { .. } => {
                    text.push_str(emote_placeholder);
                }
                MessageSegment::Emoji { .. } => {
                    text.push_str(emoji_placeholder);
                }
                MessageSegment::Cheermote { .. } => {
                    // Cheermotes render as GIF + number, approximate width ~40px
                    text.push_str("          "); // 10 spaces ≈ 40px
                }
                MessageSegment::Gif { .. } => {
                    // A Twitch chat GIF renders 80px tall with its own aspect;
                    // GIPHY assets run about 1.5:1, so ~120px wide (30 spaces).
                    text.push_str("                              ");
                }
            }
        }
        (text, has_links)
    }

    /// Calculate precise message height with all factors considered
    /// This is THE source of truth for heights - no ResizeObserver needed
    pub fn layout_message(
        &self,
        text: &str,
        width: f32,
        font_size: f32,
        has_reply: bool,
        is_first_message: bool,
    ) -> LayoutResult {
        self.layout_message_extended(
            text,
            width,
            font_size,
            has_reply,
            is_first_message,
            0,     // badge_count
            0,     // emote_count
            false, // has_timestamp
            false, // is_shared_chat
            &[],   // segments
            "",    // display_name (unknown in simple version)
            false, // is_action
        )
    }

    /// Extended layout calculation with all message context for precise height prediction
    ///
    /// This function calculates the EXACT pixel height the message will occupy in the UI.
    /// The goal is "layout precognition" - predict the future height before React renders.
    ///
    /// IMPORTANT: This assumes the frontend uses:
    /// - Satoshi font family
    /// - leading-relaxed (1.625 line-height) in Tailwind CSS
    /// - Username displayed inline on the first line with bold weight
    ///
    /// If the frontend changes these settings, this function must be updated to match.
    pub fn layout_message_extended(
        &self,
        text: &str,
        width: f32,
        font_size: f32,
        has_reply: bool,
        is_first_message: bool,
        badge_count: usize,
        emote_count: usize,
        has_timestamp: bool,
        is_shared_chat: bool,
        segments: &[MessageSegment],
        display_name: &str,
        is_action: bool,
    ) -> LayoutResult {
        let mut font_system = self.font_system.lock().unwrap();

        // === CONSTANTS matching frontend CSS ===
        // leading-relaxed in Tailwind is 1.625 line-height (NOT leading-tight which is 1.25)
        // ChatMessage.tsx uses: className="flex-1 min-w-0 leading-relaxed pb-1"
        let line_height = font_size * 1.625;

        // Emote height is fixed at h-7 (28px) in ChatMessage.tsx
        let emote_height = 28.0_f32;

        // Emoji height is h-5 (20px) - sized to align with text
        let emoji_height = 20.0_f32;

        // Badge width/height is w-4 h-4 (16px)
        let badge_size = 16.0_f32;
        let badge_gap = 4.0_f32; // gap-1

        // Timestamp width estimate (varies by format, ~40-60px)
        let timestamp_width = if has_timestamp { 55.0 } else { 0.0 };
        let timestamp_margin = if has_timestamp { 6.0 } else { 0.0 }; // mr-1.5

        // === Calculate effective text width ===
        // NOTE: The frontend LayoutUpdater already subtracts px-3 padding (24px) from the width
        // before sending it to us, so we should NOT subtract it again here.
        // We receive the content area width directly.
        let mut effective_width = width;

        // Subtract timestamp space
        effective_width -= timestamp_width + timestamp_margin;

        // Subtract badge container width
        // gap-2 between badges container and message content = 8px
        if badge_count > 0 {
            let badges_width = (badge_count as f32 * badge_size)
                + ((badge_count.saturating_sub(1)) as f32 * badge_gap)
                + 8.0; // gap-2 after badges
            effective_width -= badges_width;
        }

        // Shared chat adds border-l-2 (2px) and bg-accent/5
        if is_shared_chat {
            effective_width -= 2.0;
        }

        // === SAFETY BUFFER ===
        // Subtract a small safety buffer to ensure we wrap sooner than the frontend
        // This prevents overlapping by erring on the side of calculating a taller height
        // (extra padding is better than content being cut off or overlapping)
        effective_width -= 5.0;

        effective_width = effective_width.max(100.0);

        // === Calculate content height ===

        // Count emotes, emojis and GIFs in segments for height calculation
        let (segment_emote_count, segment_emoji_count, segment_gif_count) =
            segments.iter().fold((0, 0, 0), |(e, j, g), seg| match seg {
                MessageSegment::Emote { .. } => (e + 1, j, g),
                MessageSegment::Emoji { .. } => (e, j + 1, g),
                MessageSegment::Cheermote { .. } => (e + 1, j, g), // Cheermotes count as emotes for height
                MessageSegment::Gif { .. } => (e, j, g + 1),
                _ => (e, j, g),
            });

        let total_emotes = if segment_emote_count > 0 {
            segment_emote_count
        } else {
            emote_count
        };

        // === Reconstruct text from segments for accurate width measurement ===
        // This is critical: the backend must measure what the frontend actually renders
        // Emotes/emoji are fixed-width images, not their text codes
        let (reconstructed_text, has_links) = if !segments.is_empty() {
            self.reconstruct_text_from_segments(segments, font_size)
        } else {
            (text.to_string(), false)
        };

        // Calculate text layout using cosmic-text
        // IMPORTANT: Include username in the text to account for its width on the first line
        // Frontend displays: "Username message content" inline, so we must measure both together
        // This ensures proper line wrapping calculation when the first line includes username
        let full_text = if !display_name.is_empty() {
            // Frontend format: "Username message content" (username takes first line space)
            // The username is displayed with fontWeight: 600 (semi-bold), but for measurement
            // purposes using regular weight is close enough - the width difference is minimal
            format!("{} {}", display_name, reconstructed_text)
        } else {
            reconstructed_text
        };

        let metrics = Metrics::new(font_size, line_height);
        let mut buffer = Buffer::new(&mut font_system, metrics);

        // cosmic-text 0.19 defers layout, so set_size/set_text no longer take a
        // font system; only shape_until_scroll still needs it.
        buffer.set_size(Some(effective_width), Some(f32::MAX));

        // Use appropriate font style for measurement
        // Action messages (/me) are rendered in italics in the frontend
        let attrs = if is_action {
            Attrs::new()
                .family(Family::Name("Satoshi"))
                .style(Style::Italic)
        } else {
            Attrs::new().family(Family::Name("Satoshi"))
        };

        buffer.set_text(&full_text, &attrs, Shaping::Advanced, None);

        buffer.shape_until_scroll(&mut font_system, false);

        // Count visual lines by iterating through BufferLine layouts
        // Each BufferLine can have multiple layout lines after wrapping
        let mut line_count = 0;
        for line in buffer.lines.iter() {
            // Each line's layout_opt contains the layout runs for that line
            if let Some(layout) = line.layout_opt() {
                // Count the number of LayoutLine entries (each represents a visual line after wrapping)
                line_count += layout.len().max(1);
            } else {
                // Line not laid out yet, count as 1
                line_count += 1;
            }
        }

        // Ensure at least 1 line
        line_count = line_count.max(1);

        // Determine effective line height considering inline content
        // If line has emotes/emojis, they set minimum line height
        let has_emotes = total_emotes > 0;
        let has_emojis = segment_emoji_count > 0;

        let effective_line_height = if has_emotes {
            line_height.max(emote_height)
        } else if has_emojis {
            line_height.max(emoji_height)
        } else {
            line_height
        };

        // Content height = lines * effective line height
        let mut content_height = (line_count as f32) * effective_line_height;

        // A Twitch chat GIF is a fixed 80px inline-block (ChatMessage.tsx `h-20`
        // plus the line box's descent slack), far taller than any emote row.
        // Only the line holding it grows, and messages carry one in practice,
        // so grow the message by the difference once rather than per line.
        const GIF_ROW_HEIGHT: f32 = 86.0;
        if segment_gif_count > 0 {
            content_height += (GIF_ROW_HEIGHT - effective_line_height).max(0.0);
        }

        // === Add vertical padding ===
        // Frontend ChatMessage.tsx uses:
        //   const messageSpacing = chatDesign?.message_spacing ?? 8;
        //   paddingTop: `${Math.max(4, messageSpacing / 2)}px`,
        //   paddingBottom: `${Math.max(4, messageSpacing / 2)}px`,
        // So total vertical padding = Math.max(4, messageSpacing/2) * 2
        let message_spacing = self.config.lock().unwrap().message_spacing;
        let half_spacing = message_spacing / 2.0;
        let clamped_half = half_spacing.max(4.0);
        let vertical_padding = clamped_half * 2.0; // Total top + bottom

        // Also add pb-1 (4px) from the content div: className="flex-1 min-w-0 leading-relaxed pb-1"
        let content_padding_bottom = 4.0;

        let mut height = content_height + vertical_padding + content_padding_bottom;

        // === Add height for special elements ===

        // Reply indicator height
        // CSS: mb-1.5 (6px) + pl-2 border-l-2 content (~22px) = ~28px total
        if has_reply {
            height += 28.0;
        }

        // First message indicator
        // CSS: mt-1.5 (6px) + text line (~14px) + mb-3 (12px) = ~32px
        if is_first_message {
            height += 32.0;
        }

        // === Add small buffer for browser rendering variance ===
        // This accounts for subpixel rendering differences
        height += 2.0;

        // Add extra buffer for messages with links
        // URLs can cause additional unpredictable wrapping behavior
        // even with artificial break points, due to browser rendering differences
        if has_links {
            height += line_height * 0.5; // Add half a line of extra buffer
        }

        LayoutResult {
            height,
            width,
            has_reply,
            is_first_message,
        }
    }

    pub fn update_config(&self, width: f32, font_size: f32) {
        let mut config = self.config.lock().unwrap();
        config.width = width;
        config.font_size = font_size;
    }

    pub fn update_config_extended(
        &self,
        width: f32,
        font_size: f32,
        message_spacing: f32,
        show_timestamps: bool,
    ) {
        let mut config = self.config.lock().unwrap();
        config.width = width;
        config.font_size = font_size;
        config.message_spacing = message_spacing;
        config.show_timestamps = show_timestamps;
    }

    pub fn get_current_config(&self) -> (f32, f32) {
        let config = self.config.lock().unwrap();
        (config.width, config.font_size)
    }

    pub fn get_current_config_extended(&self) -> LayoutConfig {
        self.config.lock().unwrap().clone()
    }
}
