//! Which rendition a Cast receiver gets.
//!
//! Kept apart from `cast_session` (Android-only) so the rule is tested by the
//! desktop `cargo test` too: it is pure, and it decides whether a TV gets a
//! picture at all.

/// The fields of a rendition the pick reads, from either platform's list.
#[derive(Debug, Clone)]
pub struct Rendition<'a> {
    pub name: &'a str,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub codecs: Option<&'a str>,
    pub bandwidth: Option<u64>,
}

fn is_audio(r: &Rendition) -> bool {
    r.name == "audio_only"
        || (r.height.is_none() && r.codecs.is_some_and(|c| c.trim_start().starts_with("mp4a")))
}

/// H.264 is the one video codec every Cast receiver decodes. A rendition with
/// no `CODECS` is H.264 in practice (Kick never sends it; Twitch always does).
fn is_h264(r: &Rendition) -> bool {
    let first = r.codecs.and_then(|c| c.split(',').next()).unwrap_or("").trim();
    first.is_empty() || first.starts_with("avc1") || first.starts_with("avc3")
}

/// The rendition the receiver gets: the audio-only one for a speaker, else the
/// tallest H.264 video at or under `cap` (frame rate, then bitrate, break ties).
/// When nothing fits under the cap, the smallest H.264 video above it.
pub fn pick_rendition(renditions: &[Rendition], audio_only: bool, cap: Option<u32>) -> Option<usize> {
    if audio_only {
        return renditions.iter().position(is_audio);
    }
    let video: Vec<(usize, &Rendition)> = renditions
        .iter()
        .enumerate()
        .filter(|(_, r)| !is_audio(r) && is_h264(r))
        .collect();
    let rank = |r: &Rendition| {
        (
            r.height.unwrap_or(0),
            (r.fps.unwrap_or(0.0) * 1000.0) as u64,
            r.bandwidth.unwrap_or(0),
        )
    };
    video
        .iter()
        .filter(|(_, r)| cap.is_none_or(|c| r.height.unwrap_or(0) <= c))
        .max_by_key(|(_, r)| rank(r))
        .or_else(|| video.iter().min_by_key(|(_, r)| rank(r)))
        .map(|(i, _)| *i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r<'a>(name: &'a str, height: Option<u32>, fps: f64, codecs: Option<&'a str>) -> Rendition<'a> {
        Rendition { name, height, fps: Some(fps), codecs, bandwidth: None }
    }

    #[test]
    fn a_tv_gets_the_tallest_h264_under_the_cap() {
        let list = [
            r("1440p60", Some(1440), 60.0, Some("avc1.64002A,mp4a.40.2")),
            r("1080p60", Some(1080), 60.0, Some("avc1.64002A,mp4a.40.2")),
            r("1080p60", Some(1080), 60.0, Some("hev1.1.6.L120,mp4a.40.2")),
            r("720p60", Some(720), 60.0, Some("avc1.4D401F,mp4a.40.2")),
            r("audio_only", None, 0.0, Some("mp4a.40.2")),
        ];
        assert_eq!(pick_rendition(&list, false, Some(1080)), Some(1));
        assert_eq!(pick_rendition(&list, false, None), Some(0));
    }

    #[test]
    fn hevc_and_av1_are_never_picked() {
        let list = [
            r("1080p60", Some(1080), 60.0, Some("av01.0.08M.08,mp4a.40.2")),
            r("1080p60", Some(1080), 60.0, Some("hvc1.1.6.L120,mp4a.40.2")),
            r("480p30", Some(480), 30.0, Some("avc1.4D401E,mp4a.40.2")),
        ];
        assert_eq!(pick_rendition(&list, false, None), Some(2));
    }

    #[test]
    fn a_speaker_gets_audio_only_or_nothing() {
        let with = [r("720p60", Some(720), 60.0, None), r("audio_only", None, 0.0, Some("mp4a.40.2"))];
        assert_eq!(pick_rendition(&with, true, None), Some(1));
        let without = [r("720p60", Some(720), 60.0, None)];
        assert_eq!(pick_rendition(&without, true, None), None);
    }

    #[test]
    fn kick_renditions_without_codecs_count_as_h264() {
        let list = [r("1080p60", Some(1080), 60.0, None), r("720p60", Some(720), 60.0, None)];
        assert_eq!(pick_rendition(&list, false, None), Some(0));
    }

    #[test]
    fn nothing_under_the_cap_falls_back_to_the_smallest_above_it() {
        let list = [
            r("1440p60", Some(1440), 60.0, Some("avc1.64002A")),
            r("1200p60", Some(1200), 60.0, Some("avc1.64002A")),
        ];
        assert_eq!(pick_rendition(&list, false, Some(1080)), Some(1));
    }
}
