// Channel panel data types for the streamer about section

export interface ChannelPanel {
  id: string;
  panel_type: string; // "DEFAULT" | "EXTENSION"
  title: string | null;
  description: string | null;
  image_url: string | null;
  link_url: string | null;
}

export interface SocialMediaLink {
  name: string;
  title: string;
  url: string;
}

export interface ChannelAboutData {
  /** The channel owner's Twitch user id (their 7TV cosmetics id). */
  user_id?: string | null;
  display_name: string | null;
  description: string | null;
  profile_image_url: string | null;
  follower_count: number | null;
  panels: ChannelPanel[];
  social_links: SocialMediaLink[];
  stream_title: string | null;
  game_name: string | null;
}
