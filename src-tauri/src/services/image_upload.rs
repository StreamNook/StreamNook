//! Chat image uploads: one door for every app. The image arrives as bytes
//! (desktop paste and drops), base64 (Android and the iPhone, whose IPC is
//! JSON only) or a file path (desktop's picker and native drops). This picks
//! the host from the settings, checks the file is a picture a chat can show,
//! removes what could identify the sender (EXIF with its GPS position, XMP,
//! text chunks) from JPEG, PNG and WebP, shrinks what is too big for the
//! host, posts it the way the host documents and returns the link. AVIF and
//! BMP pass as they are; GIFs and animated PNGs are never re-encoded.

use std::io::Cursor;

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, Limits};
use imagesize::{Compression, ImageType};
use serde::Serialize;

use crate::models::settings::ImageUploaderSettings;

/// Nothing larger is read, from any app.
pub const MAX_INPUT_BYTES: usize = 25 * 1024 * 1024;
/// Longest edge a picture is sent at.
const MAX_EDGE: u32 = 4096;
/// More pixels than this are refused before decoding (a 108 MP phone photo
/// would need hundreds of MB to decode).
const MAX_PIXELS: u64 = 50_000_000;

#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    /// The name on the Images tab's host tabs.
    pub short: &'static str,
    pub note: &'static str,
    pub url: &'static str,
    pub form_field: &'static str,
    pub extra_fields: &'static [(&'static str, &'static str)],
    /// Where the link is in the reply: empty = the body is the link;
    /// otherwise a template over the JSON reply, such as `{url}` or
    /// `{link}{ext}` (`link_from_reply`).
    pub reply: &'static str,
    /// How to delete an upload: a template over the reply (the URL to POST),
    /// empty where the host has no anonymous delete.
    pub delete: &'static str,
    pub max_mb: u32,
    /// Hours until the host deletes the file; None = kept (or no fixed time).
    pub lifetime_hours: Option<u32>,
}

/// The request each host documents. kappa.lol and imgdrip.com answer with
/// JSON: the link is `{link}{ext}` and `{url}` of it.
pub const PRESETS: &[Preset] = &[
    Preset {
        id: "nuuls",
        label: "i.nuuls.com",
        short: "nuuls",
        note: "A long-running community image host. Public links, kept indefinitely.",
        url: "https://i.nuuls.com/upload",
        form_field: "attachment",
        extra_fields: &[],
        reply: "",
        delete: "",
        max_mb: 10,
        lifetime_hours: None,
    },
    Preset {
        id: "kappa",
        label: "kappa.lol",
        short: "kappa.lol",
        note: "Free, no account, no ads. Public links, kept, files up to 100 MB.",
        url: "https://kappa.lol/api/upload",
        form_field: "file",
        extra_fields: &[],
        reply: "{link}{ext}",
        delete: "https://kappa.lol/api/delete?key={key}",
        max_mb: 100,
        lifetime_hours: None,
    },
    Preset {
        id: "imgdrip",
        label: "imgdrip.com",
        short: "imgdrip",
        note: "Anonymous, no account. Links expire after a while; files up to 20 MB.",
        url: "https://imgdrip.com/api/upload",
        form_field: "file",
        extra_fields: &[],
        reply: "{url}",
        delete: "",
        max_mb: 20,
        lifetime_hours: None,
    },
    Preset {
        id: "catbox",
        label: "catbox.moe",
        short: "catbox",
        note: "Popular, permanent, no account. Public links.",
        url: "https://catbox.moe/user/api.php",
        form_field: "fileToUpload",
        extra_fields: &[("reqtype", "fileupload")],
        reply: "",
        delete: "",
        max_mb: 200,
        lifetime_hours: None,
    },
    Preset {
        id: "litterbox",
        label: "Litterbox (72 h)",
        short: "Litterbox",
        note: "catbox's temporary sibling: the link dies after 72 hours.",
        url: "https://litterbox.catbox.moe/resources/internals/api.php",
        form_field: "fileToUpload",
        extra_fields: &[("reqtype", "fileupload"), ("time", "72h")],
        reply: "",
        delete: "",
        max_mb: 1000,
        lifetime_hours: Some(72),
    },
    Preset {
        id: "uguu",
        label: "uguu.se (3 h)",
        short: "uguu",
        note: "Throwaway host: files vanish after about three hours.",
        url: "https://uguu.se/upload?output=text",
        form_field: "files[]",
        extra_fields: &[],
        reply: "",
        delete: "",
        max_mb: 128,
        lifetime_hours: Some(3),
    },
];

pub const DEFAULT_PRESET: &str = "nuuls";
pub const CUSTOM_ID: &str = "custom";

/// Hosts whose links stop working after a while, so the app keeps a copy of
/// each picture sent there and can send it again.
const EXPIRING: &[&str] = &["imgdrip", "litterbox", "uguu"];

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct UploadHost {
    pub id: String,
    pub label: String,
    /// The name on the Images tab's host tabs.
    pub short: String,
    pub note: String,
    pub max_mb: u32,
    /// Deleting an upload deletes the file there, not only the list entry.
    pub deletes_from_host: bool,
    /// Its links stop working after a while.
    pub expires: bool,
    /// How long its links last, when the host says (Litterbox 72, uguu 3).
    pub lifetime_hours: Option<u32>,
}

/// The request one upload makes.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// What consent and recents are keyed by: the preset id, or
    /// `custom:<host>` so a changed custom address asks again.
    pub key: String,
    pub label: String,
    pub short: String,
    pub url: String,
    pub form_field: String,
    pub extra_fields: Vec<(String, String)>,
    pub response_path: String,
    /// `Preset::delete` (empty: no delete on the host).
    pub delete_template: String,
    pub max_bytes: usize,
    pub lifetime_hours: Option<u32>,
}

impl Target {
    pub fn host(&self) -> UploadHost {
        UploadHost {
            id: self.key.clone(),
            label: self.label.clone(),
            short: self.short.clone(),
            note: PRESETS
                .iter()
                .find(|p| p.id == self.key)
                .map(|p| p.note)
                .unwrap_or("Your own host.")
                .to_string(),
            max_mb: (self.max_bytes / (1024 * 1024)) as u32,
            deletes_from_host: !self.delete_template.is_empty(),
            expires: self.expires(),
            lifetime_hours: self.lifetime_hours,
        }
    }

    pub fn expires(&self) -> bool {
        self.lifetime_hours.is_some() || EXPIRING.contains(&self.key.as_str())
    }
}

fn preset_host(p: &Preset) -> UploadHost {
    UploadHost {
        id: p.id.into(),
        label: p.label.into(),
        short: p.short.into(),
        note: p.note.into(),
        max_mb: p.max_mb,
        deletes_from_host: !p.delete.is_empty(),
        expires: p.lifetime_hours.is_some() || EXPIRING.contains(&p.id),
        lifetime_hours: p.lifetime_hours,
    }
}

pub fn hosts() -> Vec<UploadHost> {
    PRESETS.iter().map(preset_host).collect()
}

/// The host an upload went to, by its key: a preset, or a custom host
/// (`custom:<host>`), shown by its host name.
pub fn host_for_key(key: &str) -> UploadHost {
    if let Some(p) = PRESETS.iter().find(|p| p.id == key) {
        return preset_host(p);
    }
    let name = key.strip_prefix("custom:").filter(|h| !h.is_empty()).unwrap_or("your host").to_string();
    UploadHost {
        id: key.into(),
        label: name.clone(),
        short: name,
        note: "Your own host.".into(),
        max_mb: (MAX_INPUT_BYTES / (1024 * 1024)) as u32,
        deletes_from_host: false,
        expires: false,
        lifetime_hours: None,
    }
}

/// The request for the host an upload went to, for sending it again there.
/// None for a custom host that is no longer the one in Settings.
pub fn target_for_key(key: &str, up: &ImageUploaderSettings) -> Option<Target> {
    if PRESETS.iter().any(|p| p.id == key) {
        let mut forced = up.clone();
        forced.preset = Some(key.to_string());
        return Some(target(&forced));
    }
    let current = target(up);
    (current.key == key).then_some(current)
}

/// The host the settings name. An old file with only `url` set (from before
/// presets) is custom unless it is nuuls' address.
pub fn target(up: &ImageUploaderSettings) -> Target {
    let id = up.preset.clone().unwrap_or_else(|| match up.url.as_deref() {
        Some(u) if !u.is_empty() && u != PRESETS[0].url => CUSTOM_ID.to_string(),
        _ => DEFAULT_PRESET.to_string(),
    });
    if let Some(p) = PRESETS.iter().find(|p| p.id == id) {
        return Target {
            key: p.id.into(),
            label: p.label.into(),
            short: p.short.into(),
            url: p.url.into(),
            form_field: p.form_field.into(),
            extra_fields: p.extra_fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            response_path: p.reply.into(),
            delete_template: p.delete.into(),
            max_bytes: p.max_mb as usize * 1024 * 1024,
            lifetime_hours: p.lifetime_hours,
        };
    }
    let url = up.url.clone().unwrap_or_default();
    let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
    let label = if host.is_empty() { "your host".to_string() } else { host.clone() };
    Target {
        key: format!("custom:{host}"),
        short: label.clone(),
        label,
        url,
        form_field: up.form_field.clone().filter(|f| !f.is_empty()).unwrap_or_else(|| "file".into()),
        extra_fields: parse_extra_fields(up.extra_fields.as_deref().unwrap_or("")),
        response_path: up.response_path.clone().unwrap_or_default(),
        delete_template: String::new(),
        max_bytes: MAX_INPUT_BYTES,
        lifetime_hours: None,
    }
}

/// "reqtype=fileupload&time=72h" -> [("reqtype","fileupload"),("time","72h")]
pub fn parse_extra_fields(raw: &str) -> Vec<(String, String)> {
    raw.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (percent_decode(k.trim()), percent_decode(v.trim())))
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

/// Minimal percent-decoding for the extra form fields (`+` is not a space).
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(h) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(h);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A picture ready to post.
#[derive(Debug)]
pub struct Prepared {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
}

fn unreadable() -> String {
    "That picture could not be read.".into()
}

fn not_prepared() -> String {
    "That picture could not be prepared.".into()
}

fn decode_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(MAX_PIXELS * 4 + 64 * 1024 * 1024);
    limits
}

fn kind(bytes: &[u8]) -> Result<(&'static str, &'static str), String> {
    match imagesize::image_type(bytes) {
        Ok(ImageType::Jpeg) => Ok(("jpg", "image/jpeg")),
        Ok(ImageType::Png) => Ok(("png", "image/png")),
        Ok(ImageType::Gif) => Ok(("gif", "image/gif")),
        Ok(ImageType::Webp) => Ok(("webp", "image/webp")),
        Ok(ImageType::Bmp) => Ok(("bmp", "image/bmp")),
        Ok(ImageType::Heif(Compression::Av1)) => Ok(("avif", "image/avif")),
        Ok(ImageType::Heif(Compression::Hevc)) => Err("HEIC photos can't show in chat. Save it as a JPG first.".into()),
        _ => Err("That file isn't a picture StreamNook can send.".into()),
    }
}

fn mb(bytes: usize) -> String {
    format!("{} MB", (bytes as f64 / (1024.0 * 1024.0)).ceil() as u64)
}

/// Checks and cleans `bytes` for `target`. CPU work: call from
/// `spawn_blocking`.
pub fn prepare(bytes: Vec<u8>, target: &Target) -> Result<Prepared, String> {
    if bytes.is_empty() {
        return Err("That file is empty.".into());
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("That image is larger than 25 MB.".into());
    }
    let (ext, mime) = kind(&bytes)?;
    let size = imagesize::blob_size(&bytes).map_err(|_| unreadable())?;
    let (w, h) = (size.width as u32, size.height as u32);
    let pixels = w as u64 * h as u64;
    if matches!(ext, "jpg" | "png") && pixels > MAX_PIXELS {
        return Err(format!("That photo is too large to send ({} MP). Share a smaller copy.", pixels / 1_000_000));
    }
    let over_limit = |len: usize| -> Result<Prepared, String> {
        Err(format!(
            "That image is {}, and {} takes up to {}. Pick another host in Settings.",
            mb(len),
            target.label,
            mb(target.max_bytes)
        ))
    };
    match ext {
        "jpg" => reencode_jpeg(&bytes, target.max_bytes).or_else(|e| {
            // A JPEG the decoder cannot read (arithmetic coding, 12-bit) goes
            // as it is, minus its metadata segments.
            let stripped = strip_jpeg_metadata(&bytes).ok_or(e)?;
            if stripped.len() > target.max_bytes {
                return over_limit(stripped.len());
            }
            Ok(Prepared { bytes: stripped, ext, mime, width: w, height: h })
        }),
        "png" => {
            let (clean, animated) = clean_png(&bytes).ok_or_else(unreadable)?;
            if !animated && (clean.len() > target.max_bytes || w.max(h) > MAX_EDGE) {
                return shrink_png(&clean, target.max_bytes);
            }
            if clean.len() > target.max_bytes {
                return over_limit(clean.len());
            }
            Ok(Prepared { bytes: clean, ext, mime, width: w, height: h })
        }
        "webp" => {
            let clean = clean_webp(&bytes).ok_or_else(unreadable)?;
            if clean.len() > target.max_bytes {
                return over_limit(clean.len());
            }
            Ok(Prepared { bytes: clean, ext, mime, width: w, height: h })
        }
        _ if bytes.len() > target.max_bytes => over_limit(bytes.len()),
        _ => Ok(Prepared { bytes, ext, mime, width: w, height: h }),
    }
}

/// The longest edge of an upload's small picture.
const THUMB_EDGE: u32 = 320;

/// A small picture of an upload (a GIF's first frame): JPEG, or PNG when it
/// has see-through parts. None for what cannot be decoded here (AVIF, BMP).
/// CPU work: call from `spawn_blocking`.
pub fn thumbnail(bytes: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(decode_limits());
    let img = reader.decode().ok()?;
    let small = if img.width().max(img.height()) > THUMB_EDGE {
        img.thumbnail(THUMB_EDGE, THUMB_EDGE)
    } else {
        img
    };
    if small.color().has_alpha() {
        let mut out = Vec::new();
        small.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).ok()?;
        Some((out, "png"))
    } else {
        jpeg(&small, 82, None).ok().map(|out| (out, "jpg"))
    }
}

/// A kept copy (already cleaned when it was first sent), ready to send again
/// as it is: no second re-encode.
pub fn prepared_copy(bytes: Vec<u8>) -> Result<Prepared, String> {
    let (ext, mime) = kind(&bytes)?;
    let size = imagesize::blob_size(&bytes).map_err(|_| unreadable())?;
    Ok(Prepared { bytes, ext, mime, width: size.width as u32, height: size.height as u32 })
}

/// At most `MAX_EDGE` on the long edge. A big step down uses the cheaper
/// filter: Lanczos on a 50 MP photo takes seconds on a phone.
fn fit(img: DynamicImage) -> DynamicImage {
    let long = img.width().max(img.height());
    if long <= MAX_EDGE {
        return img;
    }
    let filter = if long > MAX_EDGE * 2 {
        image::imageops::FilterType::Triangle
    } else {
        image::imageops::FilterType::Lanczos3
    };
    img.resize(MAX_EDGE, MAX_EDGE, filter)
}

/// A JPEG of `img` with its color profile (`icc`) and nothing else: the
/// encoder writes no EXIF, so a GPS position never leaves.
fn jpeg(img: &DynamicImage, quality: u8, icc: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
        if let Some(icc) = icc {
            let _ = enc.set_icc_profile(icc.to_vec());
        }
        enc.encode_image(&img.to_rgb8()).map_err(|_| not_prepared())?;
    }
    Ok(out)
}

/// Every JPEG: upright, at most `MAX_EDGE`, its color profile kept, and no
/// other metadata.
fn reencode_jpeg(bytes: &[u8], max_bytes: usize) -> Result<Prepared, String> {
    let mut decoder = image::codecs::jpeg::JpegDecoder::new(Cursor::new(bytes)).map_err(|_| unreadable())?;
    decoder.set_limits(decode_limits()).map_err(|_| unreadable())?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let icc = decoder.icc_profile().ok().flatten();
    let mut img = DynamicImage::from_decoder(decoder).map_err(|_| unreadable())?;
    img.apply_orientation(orientation);
    let mut img = fit(img);
    let mut out = jpeg(&img, 90, icc.as_deref())?;
    // Still over the host's limit: step the size down until it fits.
    while out.len() > max_bytes && img.width().max(img.height()) > 512 {
        img = img.resize(img.width() * 3 / 4, img.height() * 3 / 4, image::imageops::FilterType::Triangle);
        out = jpeg(&img, 85, icc.as_deref())?;
    }
    if out.len() > max_bytes {
        return Err("That image is too large for this host. Pick another host in Settings.".into());
    }
    Ok(Prepared { width: img.width(), height: img.height(), bytes: out, ext: "jpg", mime: "image/jpeg" })
}

/// A PNG too big for the host or the edge: smaller PNG, or JPEG if that
/// still does not fit (transparency is lost then).
fn shrink_png(bytes: &[u8], max_bytes: usize) -> Result<Prepared, String> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    reader.limits(decode_limits());
    let img = fit(reader.decode().map_err(|_| unreadable())?);
    let mut png = Vec::new();
    img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).map_err(|_| not_prepared())?;
    if png.len() <= max_bytes {
        return Ok(Prepared { width: img.width(), height: img.height(), bytes: png, ext: "png", mime: "image/png" });
    }
    reencode_jpeg(&jpeg(&img, 90, None)?, max_bytes)
}

/// A PNG without the chunks where a location or a camera can sit (EXIF,
/// text, time), otherwise byte for byte, and whether it is animated (APNG).
/// None when it is not a well-formed PNG.
fn clean_png(bytes: &[u8]) -> Option<(Vec<u8>, bool)> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIG) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(SIG);
    let mut animated = false;
    let mut i = SIG.len();
    while i + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().ok()?) as usize;
        let end = i.checked_add(12)?.checked_add(len)?;
        if end > bytes.len() {
            return None;
        }
        let kind = &bytes[i + 4..i + 8];
        if kind == b"acTL" {
            animated = true;
        }
        if !matches!(kind, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"tIME") {
            out.extend_from_slice(&bytes[i..end]);
        }
        i = end;
        if kind == b"IEND" {
            return Some((out, animated));
        }
    }
    None
}

/// A WebP without its EXIF and XMP chunks (and their VP8X flags). None when
/// it is not a well-formed RIFF WebP.
fn clean_webp(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[0..12]);
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let kind = &bytes[i..i + 4];
        let len = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().ok()?) as usize;
        if i + 8 + len > bytes.len() {
            return None;
        }
        let end = (i + 8 + len + (len & 1)).min(bytes.len());
        if kind != b"EXIF" && kind != b"XMP " {
            let start = out.len();
            out.extend_from_slice(&bytes[i..end]);
            if kind == b"VP8X" && len >= 1 {
                // The "EXIF present" and "XMP present" flags.
                out[start + 8] &= !(0x08 | 0x04);
            }
        }
        i = end;
    }
    let riff = u32::try_from(out.len() - 8).ok()?;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    Some(out)
}

/// A JPEG minus its APP1 (EXIF, XMP) and APP13 (IPTC) segments, for files the
/// decoder cannot read. None when the markers do not parse.
fn strip_jpeg_metadata(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut out = vec![0xFF, 0xD8];
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xDA {
            // Start of scan: the rest is picture data.
            out.extend_from_slice(&bytes[i..]);
            return Some(out);
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let end = i + 2 + len;
        if len < 2 || end > bytes.len() {
            return None;
        }
        if marker != 0xE1 && marker != 0xED {
            out.extend_from_slice(&bytes[i..end]);
        }
        i = end;
    }
    None
}

/// What a host gave back for one upload.
#[derive(Clone, Debug, PartialEq)]
pub struct Posted {
    pub link: String,
    /// The URL that deletes it (POST), where the host offers one.
    pub delete_url: Option<String>,
    /// When the host said the link stops working, in ms since the epoch.
    pub expires_ms: Option<i64>,
}

/// Posts `prepared` to `target`: the link, how to delete it, when it expires.
pub async fn post(prepared: Prepared, target: &Target) -> Result<Posted, String> {
    if !target.url.starts_with("https://") {
        return Err("The upload address in Settings must start with https://.".into());
    }
    let name = format!("streamnook-{:08x}.{}", rand::random::<u32>(), prepared.ext);
    let part = reqwest::multipart::Part::bytes(prepared.bytes)
        .file_name(name)
        .mime_str(prepared.mime)
        .map_err(|e| e.to_string())?;
    let mut form = reqwest::multipart::Form::new().part(target.form_field.clone(), part);
    for (k, v) in &target.extra_fields {
        form = form.text(k.clone(), v.clone());
    }
    let resp = crate::services::http::client()
        .post(&target.url)
        .header("User-Agent", format!("StreamNook/{}", crate::services::client_identity::build_version()))
        .multipart(form)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| format!("Couldn't reach {}. Try again, or pick another host in Settings.", target.label))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "{} answered {}. Try again, or pick another host in Settings.",
            target.label,
            status.as_u16()
        ));
    }
    let link =
        link_from_reply(&text, &target.response_path).ok_or_else(|| format!("{} did not send back a link.", target.label))?;
    let json = serde_json::from_str::<serde_json::Value>(&text).ok();
    let delete_url = json
        .as_ref()
        .filter(|_| !target.delete_template.is_empty())
        .and_then(|j| fill(j, &target.delete_template, true))
        .filter(|u| u.starts_with("https://"));
    let expires_ms = json.as_ref().and_then(expiry_from_reply);
    Ok(Posted { link, delete_url, expires_ms })
}

/// Asks the host to delete an upload (its `Posted::delete_url`). A file that
/// is already gone counts as deleted.
pub async fn delete_on_host(delete_url: &str, label: &str) -> Result<(), String> {
    let resp = crate::services::http::client()
        .post(delete_url)
        .header("User-Agent", format!("StreamNook/{}", crate::services::client_identity::build_version()))
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| format!("Couldn't reach {label}. Try again."))?;
    let status = resp.status();
    if status.is_success() || status.as_u16() == 404 || status.as_u16() == 410 {
        Ok(())
    } else {
        Err(format!("{label} did not delete it ({}). Try again.", status.as_u16()))
    }
}

/// A template's `{field}`s filled from dotted paths into `json`; None when a
/// field is missing. `encode` percent-encodes the values (for a URL's query).
fn fill(json: &serde_json::Value, template: &str, encode: bool) -> Option<String> {
    let at = |path: &str| -> Option<String> {
        let pointer = format!("/{}", path.trim().trim_matches('.').replace('.', "/"));
        match json.pointer(&pointer)? {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    };
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}')? + open;
        let field = &rest[open + 1..close];
        let value = at(field)?;
        if field == "ext" && !value.is_empty() && !value.starts_with('.') {
            out.push('.');
        }
        if encode {
            out.push_str(&urlencoding::encode(&value));
        } else {
            out.push_str(&value);
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// The link in a host's reply. `how` is empty (the body is the link), a
/// dotted JSON path (`data.link`, what the custom-host box always took), or
/// a template whose `{field}`s are dotted paths into the JSON reply (`{url}`,
/// `{link}{ext}`), so a schema copied from another uploader's settings works
/// as is. An `{ext}` without its dot gets one.
pub fn link_from_reply(text: &str, how: &str) -> Option<String> {
    let how = how.trim();
    let link = if how.is_empty() {
        text.trim().to_string()
    } else {
        let json: serde_json::Value = serde_json::from_str(text).ok()?;
        let template = if how.contains('{') { how.to_string() } else { format!("{{{how}}}") };
        fill(&json, &template, false)?
    };
    (link.starts_with("https://") || link.starts_with("http://")).then_some(link)
}

/// When a host's JSON reply says the link expires: an epoch number (seconds
/// or milliseconds) or an RFC 3339 time, under the names hosts use.
fn expiry_from_reply(json: &serde_json::Value) -> Option<i64> {
    let keys = ["expires_at", "expiresAt", "expires", "expiry", "expire", "expiration"];
    let value = keys.iter().find_map(|k| json.get(*k).or_else(|| json.pointer(&format!("/data/{k}"))))?;
    match value {
        serde_json::Value::Number(n) => {
            let n = n.as_i64()?;
            Some(if n > 100_000_000_000 { n } else { n * 1000 })
        }
        serde_json::Value::String(s) => chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp_millis()),
        _ => None,
    }
}

/// How long ago, in words: "just now", "5 minutes ago", "2 hours ago",
/// "3 days ago".
pub fn ago_label(at_ms: i64, now_ms: i64) -> String {
    let minutes = (now_ms - at_ms).max(0) / 60_000;
    let (n, unit) = if minutes < 1 {
        return "just now".into();
    } else if minutes < 60 {
        (minutes, "minute")
    } else if minutes < 48 * 60 {
        (minutes / 60, "hour")
    } else {
        (minutes / (24 * 60), "day")
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

/// A moment on this device's clock: "Oct 21, 3:40 PM".
pub fn clock_label(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%b %-d, %-I:%M %p").to_string())
        .unwrap_or_default()
}

/// What an upload is: "1920 × 1080 · PNG · 1.4 MB" (the size left out when
/// it was not recorded).
pub fn details_label(width: u32, height: u32, ext: &str, bytes: u64) -> String {
    let mut parts = Vec::new();
    if width > 0 && height > 0 {
        parts.push(format!("{width} × {height}"));
    }
    if !ext.is_empty() {
        parts.push(if ext == "jpg" { "JPG".to_string() } else { ext.to_ascii_uppercase() });
    }
    if bytes > 0 {
        parts.push(size_label(bytes));
    }
    parts.join(" · ")
}

/// "1.4 MB", "12 KB", "1.2 GB".
pub fn size_label(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= 1024.0 * MB {
        format!("{:.1} GB", bytes as f64 / (1024.0 * MB))
    } else if bytes as f64 >= MB {
        format!("{:.1} MB", bytes as f64 / MB)
    } else {
        format!("{} KB", (bytes / 1024).max(if bytes == 0 { 0 } else { 1 }))
    }
}

/// The full expiry line for an upload's details.
pub fn expiry_label(expires_ms: Option<i64>, host_expires: bool, host_label: &str, now_ms: i64) -> String {
    match expires_ms {
        Some(e) if e <= now_ms => format!("Expired {}", clock_label(e)),
        Some(e) => format!("Expires {} ({})", clock_label(e), time_left_label(e, now_ms)),
        None if host_expires => format!("Expires after a while; {host_label} doesn't say when"),
        None => "Kept indefinitely".into(),
    }
}

/// How long a link has left, in words: "2 days left", "5 hours left",
/// "40 minutes left", or "Expired".
pub fn time_left_label(expires_ms: i64, now_ms: i64) -> String {
    let left = expires_ms - now_ms;
    if left <= 0 {
        return "Expired".into();
    }
    let minutes = left / 60_000;
    let (n, unit) = if minutes >= 48 * 60 {
        (minutes / (24 * 60), "day")
    } else if minutes >= 120 {
        (minutes / 60, "hour")
    } else {
        (minutes.max(1), "minute")
    };
    format!("{n} {unit}{} left", if n == 1 { "" } else { "s" })
}

/// A 1x1 transparent PNG, enough to prove a host's round trip.
pub fn test_png() -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==")
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(preset: Option<&str>, url: Option<&str>) -> ImageUploaderSettings {
        ImageUploaderSettings { preset: preset.map(String::from), url: url.map(String::from), ..Default::default() }
    }

    fn png_of(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        DynamicImage::new_rgba8(w, h).write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
        out
    }

    /// `png` with a chunk of `kind` holding `data` spliced in after IHDR (the
    /// CRC is not checked by `clean_png`, so zeros do).
    fn with_chunk(png: &[u8], kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let ihdr_end = 8 + 12 + 13;
        let mut out = png[..ihdr_end].to_vec();
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&png[ihdr_end..]);
        out
    }

    #[test]
    fn presets_resolve_and_default_to_nuuls() {
        assert_eq!(target(&settings(None, None)).key, "nuuls");
        let t = target(&settings(Some("catbox"), None));
        assert_eq!(t.form_field, "fileToUpload");
        assert_eq!(t.extra_fields, vec![("reqtype".to_string(), "fileupload".to_string())]);
        assert_eq!(target(&settings(Some("litterbox"), None)).lifetime_hours, Some(72));
    }

    #[test]
    fn kappa_reply_gives_its_delete_url() {
        // The reply kappa.lol gave a real test upload (the file was deleted).
        let reply = r#"{"id":"1lumEe","ext":".png","type":"image/png","checksum":"b357a19c87624c7c4d131aeeb4ae677f","key":"xIGhwZxJlogcMROT","link":"https://kappa.lol/1lumEe","delete":"https://kappa.lol/delete?xIGhwZxJlogcMROT"}"#;
        let t = target(&settings(Some("kappa"), None));
        assert_eq!(link_from_reply(reply, &t.response_path).as_deref(), Some("https://kappa.lol/1lumEe.png"));
        let json: serde_json::Value = serde_json::from_str(reply).unwrap();
        assert_eq!(
            fill(&json, &t.delete_template, true).as_deref(),
            Some("https://kappa.lol/api/delete?key=xIGhwZxJlogcMROT")
        );
        assert!(t.host().deletes_from_host);
        assert!(!target(&settings(Some("nuuls"), None)).host().deletes_from_host);
    }

    #[test]
    fn expiring_hosts_and_reply_expiry() {
        assert!(target(&settings(Some("uguu"), None)).host().expires);
        assert!(target(&settings(Some("imgdrip"), None)).host().expires);
        assert!(!target(&settings(Some("kappa"), None)).host().expires);
        let secs = serde_json::json!({ "url": "x", "expires_at": 1_800_000_000 });
        assert_eq!(expiry_from_reply(&secs), Some(1_800_000_000_000));
        let iso = serde_json::json!({ "data": { "expires": "2026-11-20T00:00:00Z" } });
        assert_eq!(expiry_from_reply(&iso), Some(1_795_132_800_000));
        assert_eq!(expiry_from_reply(&serde_json::json!({ "url": "x" })), None);
    }

    #[test]
    fn time_left_reads_in_words() {
        let h = 3_600_000;
        assert_eq!(time_left_label(3 * 24 * h, 0), "3 days left");
        assert_eq!(time_left_label(47 * h, 0), "47 hours left");
        assert_eq!(time_left_label(90 * 60_000, 0), "90 minutes left");
        assert_eq!(time_left_label(30_000, 0), "1 minute left");
        assert_eq!(time_left_label(0, 5), "Expired");
    }

    #[test]
    fn small_pictures_keep_shape_and_transparency() {
        let (png, ext) = thumbnail(&png_of(1000, 500)).unwrap();
        assert_eq!(ext, "png", "see-through stays PNG");
        let small = image::load_from_memory(&png).unwrap();
        assert_eq!((small.width(), small.height()), (320, 160));
        let opaque = jpeg(&DynamicImage::new_rgb8(200, 100), 90, None).unwrap();
        let (jpg, ext) = thumbnail(&opaque).unwrap();
        assert_eq!(ext, "jpg");
        assert_eq!(image::load_from_memory(&jpg).unwrap().width(), 200, "never made bigger");
        assert!(thumbnail(b"not a picture").is_none());
    }

    #[test]
    fn detail_words() {
        let m = 60_000;
        assert_eq!(ago_label(0, 30_000), "just now");
        assert_eq!(ago_label(0, m), "1 minute ago");
        assert_eq!(ago_label(0, 5 * 60 * m), "5 hours ago");
        assert_eq!(ago_label(0, 3 * 24 * 60 * m), "3 days ago");
        assert_eq!(details_label(1920, 1080, "png", 1_468_006), "1920 × 1080 · PNG · 1.4 MB");
        assert_eq!(details_label(64, 64, "jpg", 900), "64 × 64 · JPG · 1 KB");
        assert_eq!(details_label(10, 10, "gif", 0), "10 × 10 · GIF");
        assert_eq!(expiry_label(None, false, "kappa.lol", 0), "Kept indefinitely");
        assert_eq!(expiry_label(None, true, "imgdrip.com", 0), "Expires after a while; imgdrip.com doesn't say when");
        assert!(expiry_label(Some(10), true, "uguu.se", 20).starts_with("Expired "));
        assert!(expiry_label(Some(3 * 3_600_000), true, "uguu.se", 0).ends_with("(3 hours left)"));
    }

    #[test]
    fn hosts_by_key() {
        assert_eq!(host_for_key("litterbox").short, "Litterbox");
        assert_eq!(host_for_key("custom:up.example.org").label, "up.example.org");
        let up = settings(Some("nuuls"), None);
        assert_eq!(target_for_key("uguu", &up).map(|t| t.key).as_deref(), Some("uguu"));
        assert!(target_for_key("custom:gone.example", &up).is_none());
    }

    #[test]
    fn new_presets_resolve() {
        let k = target(&settings(Some("kappa"), None));
        assert_eq!(
            (k.url.as_str(), k.form_field.as_str(), k.response_path.as_str()),
            ("https://kappa.lol/api/upload", "file", "{link}{ext}")
        );
        let i = target(&settings(Some("imgdrip"), None));
        assert_eq!((i.form_field.as_str(), i.response_path.as_str(), i.max_bytes), ("file", "{url}", 20 * 1024 * 1024));
    }

    #[test]
    fn an_old_custom_url_stays_custom_and_keys_by_host() {
        let t = target(&settings(None, Some("https://up.example.org/api")));
        assert_eq!(t.key, "custom:up.example.org");
        assert_eq!(t.form_field, "file");
        assert_eq!(target(&settings(None, Some("https://i.nuuls.com/upload"))).key, "nuuls");
    }

    #[test]
    fn extra_fields_parse_and_decode() {
        assert_eq!(parse_extra_fields("reqtype=fileupload&time=72h").len(), 2);
        assert_eq!(parse_extra_fields("a%20b=c%26d"), vec![("a b".to_string(), "c&d".to_string())]);
        assert!(parse_extra_fields("=x&&novalue").is_empty());
        // A percent sign before a non-ASCII letter must not split the letter.
        assert_eq!(parse_extra_fields("k=%é"), vec![("k".to_string(), "%é".to_string())]);
    }

    #[test]
    fn reply_links() {
        assert_eq!(link_from_reply("https://i.nuuls.com/a.png\n", "").as_deref(), Some("https://i.nuuls.com/a.png"));
        assert_eq!(
            link_from_reply(r#"{"data":{"link":"https://x.example/a.gif"}}"#, "data.link").as_deref(),
            Some("https://x.example/a.gif")
        );
        assert!(link_from_reply("error: too big", "").is_none());
        assert!(link_from_reply("{}", "data.link").is_none());
    }

    #[test]
    fn reply_templates() {
        assert_eq!(
            link_from_reply(r#"{"link":"https://kappa.lol/AbC","ext":".png","delete":"x"}"#, "{link}{ext}").as_deref(),
            Some("https://kappa.lol/AbC.png")
        );
        assert_eq!(
            link_from_reply(r#"{"link":"https://kappa.lol/AbC","ext":"gif"}"#, "{link}{ext}").as_deref(),
            Some("https://kappa.lol/AbC.gif")
        );
        assert_eq!(
            link_from_reply(r#"{"url":"https://imgdrip.com/i/x.webp"}"#, "{url}").as_deref(),
            Some("https://imgdrip.com/i/x.webp")
        );
        assert!(link_from_reply(r#"{"link":"https://kappa.lol/AbC"}"#, "{link}{ext}").is_none());
        assert!(link_from_reply(r#"{"url":"https://a.example/x.png"}"#, "{url").is_none());
        assert_eq!(
            link_from_reply(r#"{"data":{"id":"7"}}"#, "https://h.example/{data.id}.png").as_deref(),
            Some("https://h.example/7.png")
        );
    }

    #[test]
    fn refuses_what_chat_cannot_show() {
        let t = target(&settings(None, None));
        assert!(prepare(b"not an image at all".to_vec(), &t).is_err());
        assert!(prepare(Vec::new(), &t).is_err());
    }

    #[test]
    fn heic_is_refused_in_words_and_other_ftyp_files_are_not_called_heic() {
        let mut heic = vec![0, 0, 0, 0x18];
        heic.extend_from_slice(b"ftypheic\0\0\0\0mif1heic");
        assert!(kind(&heic).unwrap_err().contains("HEIC"));
        let mut mp4 = vec![0, 0, 0, 0x18];
        mp4.extend_from_slice(b"ftypisom\0\0\0\0isomiso2");
        assert!(!kind(&mp4).unwrap_err().contains("HEIC"));
    }

    #[test]
    fn png_under_the_limit_passes_untouched() {
        let t = target(&settings(None, None));
        let png = test_png();
        let p = prepare(png.clone(), &t).unwrap();
        assert_eq!((p.ext, p.width, p.height), ("png", 1, 1));
        assert_eq!(p.bytes, png);
    }

    #[test]
    fn png_text_and_exif_chunks_are_removed() {
        let t = target(&settings(None, None));
        let tagged = with_chunk(&png_of(2, 2), b"tEXt", b"GPS\0somewhere");
        let p = prepare(tagged, &t).unwrap();
        assert!(!p.bytes.windows(4).any(|w| w == b"tEXt"));
        assert!(image::load_from_memory(&p.bytes).is_ok(), "still a PNG");
    }

    #[test]
    fn apng_is_never_reencoded() {
        let mut t = target(&settings(None, None));
        let apng = with_chunk(&png_of(2, 2), b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]);
        t.max_bytes = apng.len() - 1;
        let err = prepare(apng, &t).unwrap_err();
        assert!(err.contains("takes up to"), "{err}");
    }

    #[test]
    fn webp_exif_is_removed_and_flags_cleared() {
        let mut b = b"RIFF\0\0\0\0WEBP".to_vec();
        b.extend_from_slice(b"VP8X");
        b.extend_from_slice(&10u32.to_le_bytes());
        b.extend_from_slice(&[0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        b.extend_from_slice(b"EXIF");
        b.extend_from_slice(&4u32.to_le_bytes());
        b.extend_from_slice(b"GPS!");
        b.extend_from_slice(b"VP8L");
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&[0x2F, 0]);
        let riff = (b.len() - 8) as u32;
        b[4..8].copy_from_slice(&riff.to_le_bytes());
        let out = clean_webp(&b).unwrap();
        assert!(!out.windows(4).any(|w| w == b"EXIF"));
        assert_eq!(out[20] & 0x08, 0, "EXIF flag cleared");
        assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()) as usize, out.len() - 8);
    }

    #[test]
    fn undecodable_jpeg_loses_app1() {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x08];
        b.extend_from_slice(b"Exif\0\0");
        b.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x04, 0x00, 0x00]);
        b.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0x11, 0x22]);
        let out = strip_jpeg_metadata(&b).unwrap();
        assert!(!out.windows(4).any(|w| w == b"Exif"));
        assert!(out.windows(2).any(|w| w == [0xFF, 0xDB]));
        assert!(out.ends_with(&[0xFF, 0xDA, 0x00, 0x02, 0x11, 0x22]));
    }

    #[test]
    fn huge_photos_are_refused_before_decoding() {
        // SOI, then a baseline SOF0 claiming 10000 x 10000, then EOI.
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        b.extend_from_slice(&10_000u16.to_be_bytes());
        b.extend_from_slice(&10_000u16.to_be_bytes());
        b.extend_from_slice(&[0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xFF, 0xD9]);
        let err = prepare(b, &target(&settings(None, None))).unwrap_err();
        assert!(err.contains("too large to send"), "{err}");
    }

    #[test]
    fn jpeg_comes_out_without_exif_and_keeps_its_color_profile() {
        let img = DynamicImage::new_rgb8(2, 1);
        let mut plain = Vec::new();
        {
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut plain, 90);
            enc.set_icc_profile(vec![0; 128]).unwrap();
            enc.encode_image(&img.to_rgb8()).unwrap();
        }
        // An APP1 Exif segment spliced in after SOI.
        let mut with_exif = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x0A];
        with_exif.extend_from_slice(b"Exif\0\0MM");
        with_exif.extend_from_slice(&plain[2..]);
        let p = prepare(with_exif, &target(&settings(None, None))).unwrap();
        assert_eq!(p.ext, "jpg");
        assert!(!p.bytes.windows(4).any(|w| w == b"Exif"), "EXIF must not survive");
        assert!(p.bytes.windows(11).any(|w| w == b"ICC_PROFILE"), "the color profile stays");
    }
}
