import Foundation

/// The leading symbol of every settings row, by the setting it writes (its first path), or by its
/// title for a row that writes nothing itself. One table, so a page cannot ship a row without one
/// and two rows for the same setting always match.
enum SettingIcons {
    static func symbol(paths: [String], title: String) -> String {
        for path in paths {
            if let symbol = byPath[path] { return symbol }
            // An event highlight rule writes `<rule>.enabled` and `<rule>.color`.
            if let symbol = byPath[path.components(separatedBy: ".").dropLast().joined(separator: ".")] { return symbol }
        }
        let bare = title.components(separatedBy: ":").first ?? title
        return byTitle[bare] ?? "slider.horizontal.3"
    }

    private static let byPath: [String: String] = [
        // Player
        "auto_switch.enabled": "arrow.triangle.2.circlepath",
        "auto_switch.auto_redirect_on_raid": "person.3",
        "auto_switch.stay_in_offline_chat": "bubble.left",
        "streamlink.enhanced_codecs": "film.stack",
        "streamlink.stream_timeout": "timer",
        "streamlink.retry_streams": "arrow.clockwise",
        "video_player.autoplay": "play.circle",
        "video_player.background_mode": "pip",
        "quality": "sparkles.tv",
        "video_player.muted": "speaker.slash",
        "video_player.volume": "speaker.wave.2",
        "video_player.resume_vod_playback": "arrow.counterclockwise.circle",
        "video_player.sync_vod_progress": "arrow.triangle.2.circlepath.circle",
        "chat_input.hide_points_balance": "creditcard",
        "video_player.experimental_low_latency": "bolt",
        "video_player.ll_target_latency": "gauge.with.dots.needle.33percent",
        "discovery_languages": "globe",
        // Ad-free
        "video_player.ad_bypass_enabled": "checkmark.shield",
        "video_player.ad_bypass_proxies": "point.3.connected.trianglepath.dotted",
        // Chat: layout and events
        "fullscreen_chat.phone_layout": "rectangle.landscape.rotate",
        "show_polls": "chart.bar",
        "show_predictions": "chart.line.uptrend.xyaxis",
        "show_channel_point_redemptions": "gift",
        "chat_events.hidden_provider_events": "bell.badge",
        "chat_design.pinned_start_collapsed": "pin",
        "chat_design.show_dividers": "line.3.horizontal",
        "chat_design.alternating_backgrounds": "rectangle.split.1x2",
        "chat_design.message_spacing": "arrow.up.and.down.text.horizontal",
        "chat_design.font_size": "textformat.size",
        "chat_design.font_weight": "bold",
        "chat_design.message_entrance": "wand.and.stars",
        "chat_design.backfill_opacity": "clock.arrow.circlepath",
        "chat_design.show_timestamps": "clock",
        // Names, badges, mentions
        "chat_design.username_style": "person.text.rectangle",
        "chat_design.name_color_adjustment": "eye",
        "chat_design.show_badges": "checkmark.seal",
        "chat_design.badge_scale": "arrow.up.left.and.arrow.down.right",
        "chat_design.show_third_party_badges": "rosette",
        "cosmetics.profile_picture_source": "person.crop.circle",
        "cosmetics.profile_picture_animation": "person.crop.circle",
        "chat_design.show_at_sign": "at",
        "cosmetics.paint_shadows": "shadow",
        "chat_design.mention_color": "highlighter",
        "chat_design.paint_mentions_in_body": "paintbrush",
        "chat_design.reply_color": "arrowshape.turn.up.left",
        "chat_design.reply_style": "text.quote",
        // Emotes
        "chat_design.emoji_style": "face.smiling",
        "chat_design.show_personal_emotes": "person.crop.square",
        "chat_design.animate_emotes": "play.rectangle",
        "chat_design.show_chat_gifs": "photo.on.rectangle",
        "chat_design.emote_scale": "plus.magnifyingglass",
        "chat_design.emote_margin": "arrow.left.and.right",
        "chat_design.ffz_emote_effects": "sparkles",
        "chat_design.bttv_emote_modifiers": "wand.and.rays",
        "chat_design.giant_emotes": "square.resize.up",
        // Input and behavior
        "chat_input.bypass_duplicate": "doc.on.doc",
        "chat_input.spellcheck_enabled": "textformat.abc.dottedunderline",
        "chat_input.show_room_modes": "lock.shield",
        "chat_input.hide_placeholder": "text.cursor",
        "chat_input.hide_emote_button": "face.dashed",
        "chat_input.inbox_button_side": "tray",
        "chat_input.emote_tab_complete_enabled": "text.badge.plus",
        "chat_input.emote_tab_complete_include_chatters": "person.2",
        "auto_claim_points_watching": "shippingbox",
        "chat_design.deleted_message_style": "trash",
        "chat_design.hide_shared_chat": "rectangle.2.swap",
        "chat_render.smooth_scroll_on_resume": "arrow.down.to.line",
        "chat_render.message_buffer_cap": "scroll",
        "chat_burst.vary_repeats": "shuffle",
        "chat_burst.delay_ms": "speedometer",
        // Links and hidden messages
        "chat_design.link_previews": "text.below.photo",
        "chat_design.shorten_links": "link",
        "chat_design.link_color": "paintpalette",
        "chat_design.link_underline": "underline",
        "chat_filters.hide_bots": "cpu",
        "chat_filters.hidden_users": "person.crop.circle.badge.xmark",
        "chat_filters.per_channel": "number",
        "chat_filters.hide_commands": "exclamationmark.bubble",
        "chat_filters.ignored_phrases": "text.badge.xmark",
        // Highlights, commands, moderation, nicknames
        "chat_highlights.built_in.first_time_chatter": "hand.wave",
        "chat_highlights.built_in.returning_chatter": "arrow.uturn.backward.circle",
        "chat_highlights.built_in.self_message": "person",
        "chat_highlights.built_in.raider": "person.3.sequence",
        "chat_highlights.appearance.display_style": "circle.lefthalf.filled",
        "chat_highlights.appearance.opacity": "circle.lefthalf.filled",
        "chat_highlights.phrases": "text.magnifyingglass",
        "chat_highlights.users": "person.crop.circle.badge.checkmark",
        "chat_highlights.badges": "checkmark.seal.fill",
        "chat_commands.user_commands": "terminal",
        "chat_design.mod_slider": "hand.draw",
        "chat_design.mod_hold_buttons": "hand.tap",
        "chat_design.mod_slider_delete": "list.number",
        "moderation.timeout_presets": "timer",
        "moderation.card_actions": "hand.raised",
        "chat_customization.user_overrides": "tag",
        // Notifications: one per alert type the core can list, and the mention rules
        "live_notifications.show_live_notifications": "dot.radiowaves.left.and.right",
        "live_notifications.show_favorite_live_notifications": "star",
        "live_notifications.show_mention_notifications": "at",
        "live_notifications.show_drops_notifications": "shippingbox",
        "live_notifications.show_badge_notifications": "medal",
        "live_notifications.muted_live_channels": "line.3.horizontal.decrease.circle",
        "mentions.match_bare_login": "at",
        "mentions.match_display_name": "person.crop.circle.badge.checkmark",
        "mentions.highlight_words": "highlighter",
        "mentions.muted_channels": "bell.slash",
        // Cache, support, theme
        "cache.enabled": "internaldrive",
        "cache.expiry_days": "calendar.badge.clock",
        "error_reporting_enabled": "doc.text.magnifyingglass",
        "theme": "paintpalette",
        "glass_transparency": "drop",
        "font": "textformat",
        "oled_accent": "circle.hexagongrid",
    ]

    /// Rows that write nothing themselves (a group of nested choices, a button, a note).
    private static let byTitle: [String: String] = [
        "@name style": "at.circle",
        "Vibration": "iphone.radiowaves.left.and.right",
        "Storage": "externaldrive",
        "Followed channels": "arrow.down.circle",
        "Log export": "square.and.arrow.up",
        "Channels and emotes": "chart.bar.xaxis",
        "Version and platform": "info.circle",
        "Linked accounts": "link.circle",
        "VOD positions": "play.rectangle.on.rectangle",
        "Export": "square.and.arrow.up",
        "Import": "square.and.arrow.down",
        "Languages": "globe",
        "Counts as a mention": "checklist",
    ]
}
