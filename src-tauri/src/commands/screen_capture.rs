// Screen-region capture for the Profile share feature.
//
// Two commands:
//
//   capture_screen_region   — single PNG frame via xcap. Used for the
//                             static-paint path AND as a building block
//                             for the GIF fallback assembled in JS via
//                             gifenc.
//
//   capture_animated_webp   — full animated capture pipeline driven by
//                             DXGI Output Duplication directly from the
//                             windows crate. No third-party binary, no
//                             extra Rust dependencies, no cross-process
//                             IPC. The GPU compositor's framebuffer is
//                             copied into a staging ID3D11Texture2D
//                             sized exactly to our crop region via
//                             CopySubresourceRegion — so the readback
//                             is a few hundred KB per frame instead of
//                             the entire monitor.
//
// The frontend computes screen-physical (desktop-global) coordinates
// from innerPosition + rect * DPR. DXGI exposes each monitor as an
// IDXGIOutput whose DesktopCoordinates rectangle is in that same space,
// so we find the right output by point-in-rect and translate to
// monitor-local coords for the GPU-side crop box.

use image::RgbaImage;
use std::io::Cursor;
use std::time::Duration;
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter};
use webp_animation::{Encoder, EncoderOptions, EncodingConfig, EncodingType};
// Windows-only: see the xcap note in Cargo.toml.
#[cfg(windows)]
use xcap::Monitor;

#[cfg(windows)]
use windows::{
    core::Interface,
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL},
            Direct3D11::{
                D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BOX,
                D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
                D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
            },
            Dxgi::{
                Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
                IDXGIAdapter, IDXGIDevice, IDXGIOutput, IDXGIOutput1, IDXGIOutputDuplication,
                IDXGIResource, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
            },
        },
    },
};

// Upper bound on the whole animated-capture operation so a hang in DXGI
// can't silently freeze the share UI. duration_ms itself is normally
// 2-4s; the timeout covers device setup + capture loop + encode with
// comfortable margin.
const CAPTURE_TIMEOUT_SECS: u64 = 30;

// Static PNG capture (full monitor + crop) — unchanged, used by the
// static-paint path and the GIF fallback.

#[tauri::command]
pub async fn capture_screen_region(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<Response, String> {
    if width == 0 || height == 0 {
        return Err("capture region has zero size".into());
    }

    let bytes = tokio::task::spawn_blocking(move || capture_png_blocking(x, y, width, height))
        .await
        .map_err(|e| format!("capture task panicked: {e}"))??;

    Ok(Response::new(bytes))
}

#[cfg(not(windows))]
fn capture_png_blocking(x: i32, y: i32, width: u32, height: u32) -> Result<Vec<u8>, String> {
    // Off Windows this goes through `platform::capture`, which shells out to
    // macOS's own `screencapture`. xcap is not an option here: the pinned
    // 0.0.14 does not compile for macOS, and 0.9.x is a breaking refactor of a
    // shipping Windows feature.
    //
    // The command stays registered (and stays in the ACL) either way, so the
    // frontend gets a real error rather than a silent invoke denial — and a
    // missing Screen Recording permission is reported AS a permission problem
    // instead of a generic failure.
    crate::platform::capture::capture_region_png(x, y, width, height).map_err(|e| e.to_string())
}

#[cfg(windows)]
fn capture_png_blocking(x: i32, y: i32, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let monitors = Monitor::all().map_err(|e| format!("enumerate monitors: {e}"))?;
    if monitors.is_empty() {
        return Err("no monitors detected".into());
    }
    let idx = pick_monitor_idx(&monitors, x, y);
    let cropped = capture_monitor_region(&monitors[idx], x, y, width, height)?;

    let mut buf = Vec::with_capacity((cropped.width() * cropped.height() * 4) as usize);
    cropped
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| format!("PNG encode: {e}"))?;
    Ok(buf)
}

// Animated WebP via DXGI Output Duplication. Coords are screen-physical
// (desktop-global).

#[tauri::command]
pub async fn capture_animated_webp(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    radius_px: u32,
    target_frame_count: u32,
    duration_ms: u32,
) -> Result<Response, String> {
    if width == 0 || height == 0 {
        return Err("capture region has zero size".into());
    }
    if target_frame_count < 2 {
        return Err("target_frame_count must be >= 2 for animated WebP".into());
    }
    if duration_ms == 0 {
        return Err("duration_ms must be > 0".into());
    }

    #[cfg(windows)]
    {
        let fps = ((target_frame_count as f64 * 1000.0 / duration_ms as f64).round() as u32)
            .clamp(10, 144);

        // Two-stage: capture loop fills Vec<Vec<u8>> at DXGI's native
        // refresh rate, then a second spawn_blocking encodes all frames.
        // libwebp's lossless add_frame is too slow at high DPI (~500ms/
        // frame for a 3200×600 cropped card) to live inside the capture
        // hot loop without crashing the effective frame rate. Memory cost
        // is ~one frame buffer per captured frame held until encode
        // completes — bounded by duration_ms × fps × frame_size.
        let capture_start = std::time::Instant::now();
        let capture_result = tokio::time::timeout(
            Duration::from_secs(CAPTURE_TIMEOUT_SECS),
            tokio::task::spawn_blocking(move || {
                capture_via_dxgi(x, y, width, height, fps, duration_ms)
            }),
        )
        .await
        .map_err(|_| format!("DXGI capture timed out after {CAPTURE_TIMEOUT_SECS}s"))?
        .map_err(|e| format!("DXGI capture task panicked: {e}"))??;
        let capture_ms = capture_start.elapsed().as_millis() as i64;

        let (frames, out_w, out_h) = capture_result;
        if frames.is_empty() {
            return Err("DXGI returned zero frames".into());
        }
        let frame_count = frames.len() as u32;

        let encode_start = std::time::Instant::now();
        let webp_bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
            let mut encoder = Encoder::new_with_options(
                (out_w, out_h),
                EncoderOptions {
                    kmin: 9,
                    kmax: 17,
                    encoding_config: Some(EncodingConfig {
                        encoding_type: EncodingType::Lossless,
                        quality: 75.0,
                        method: 4,
                    }),
                    ..Default::default()
                },
            )
            .map_err(|e| format!("webp encoder init: {e:?}"))?;

            let interval_ms_f = 1000.0 / fps as f64;
            for (i, bytes) in frames.into_iter().enumerate() {
                let ts = (i as f64 * interval_ms_f).round() as i32;
                let mut img = RgbaImage::from_raw(out_w, out_h, bytes)
                    .ok_or_else(|| format!("frame {i}: RgbaImage::from_raw failed"))?;
                if radius_px > 0 {
                    apply_rounded_corner_alpha(&mut img, radius_px);
                }
                encoder
                    .add_frame(img.as_raw(), ts)
                    .map_err(|e| format!("webp add_frame[{i}]: {e:?}"))?;
            }

            let webp = encoder
                .finalize(duration_ms as i32)
                .map_err(|e| format!("webp finalize: {e:?}"))?;
            Ok(webp.to_vec())
        })
        .await
        .map_err(|e| format!("encode task panicked: {e}"))??;
        let encode_ms = encode_start.elapsed().as_millis() as i64;

        let _ = app.emit(
            "profile-capture-stats",
            serde_json::json!({
                "frame_count": frame_count,
                "capture_ms": capture_ms,
                "encode_ms": encode_ms,
                "duration_ms": duration_ms,
            }),
        );

        Ok(Response::new(webp_bytes))
    }

    #[cfg(not(windows))]
    {
        let _ = (
            app,
            x,
            y,
            width,
            height,
            radius_px,
            target_frame_count,
            duration_ms,
        );
        Err("animated WebP capture requires Windows (uses DXGI Output Duplication)".into())
    }
}

#[cfg(windows)]
fn capture_via_dxgi(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u32,
) -> Result<(Vec<Vec<u8>>, u32, u32), String> {
    unsafe {
        // 1. Create D3D11 device. BGRA_SUPPORT is required for
        // CopySubresourceRegion across BGRA staging textures, which is
        // the only format DXGI desktop duplication outputs.
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let mut feature_level = D3D_FEATURE_LEVEL::default();
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut feature_level),
            Some(&mut context),
        )
        .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
        let device = device.ok_or("D3D11 device handle null")?;
        let context = context.ok_or("D3D11 context handle null")?;

        // 2. Walk to the DXGI adapter so we can enumerate outputs.
        let dxgi_device: IDXGIDevice = device
            .cast()
            .map_err(|e| format!("cast IDXGIDevice: {e}"))?;
        let adapter: IDXGIAdapter = dxgi_device
            .GetAdapter()
            .map_err(|e| format!("GetAdapter: {e}"))?;

        // 3. Find the output whose DesktopCoordinates rect contains the
        // capture point. Tauri's innerPosition is in the same coordinate
        // space (virtual desktop pixels), so the math is direct.
        let mut output_idx = 0u32;
        let (output, bounds) = loop {
            let o: IDXGIOutput = match adapter.EnumOutputs(output_idx) {
                Ok(o) => o,
                Err(_) => return Err(format!("no DXGI output contains screen point ({x},{y})")),
            };
            let desc = o.GetDesc().map_err(|e| format!("output GetDesc: {e}"))?;
            let b = desc.DesktopCoordinates;
            if x >= b.left && x < b.right && y >= b.top && y < b.bottom {
                break (o, b);
            }
            output_idx += 1;
        };

        // 4. Translate to monitor-local coords and clamp to monitor bounds.
        // The crop box for CopySubresourceRegion is in source-texture
        // coords, and the source is exactly this output's framebuffer.
        let monitor_x = (x - bounds.left).max(0) as u32;
        let monitor_y = (y - bounds.top).max(0) as u32;
        let monitor_w = (bounds.right - bounds.left).max(0) as u32;
        let monitor_h = (bounds.bottom - bounds.top).max(0) as u32;
        let crop_w = width.min(monitor_w.saturating_sub(monitor_x));
        let crop_h = height.min(monitor_h.saturating_sub(monitor_y));
        if crop_w == 0 || crop_h == 0 {
            return Err("capture rect falls outside monitor bounds".into());
        }

        // 5. Duplicate the output. This is the live framebuffer feed.
        let output1: IDXGIOutput1 = output
            .cast()
            .map_err(|e| format!("cast IDXGIOutput1: {e}"))?;
        let duplication: IDXGIOutputDuplication = output1
            .DuplicateOutput(&device)
            .map_err(|e| format!("DuplicateOutput: {e}"))?;

        // 6. Staging texture sized exactly to the crop region. This is
        // where each frame's pixels land for CPU readback.
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: crop_w,
            Height: crop_h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging: Option<ID3D11Texture2D> = None;
        device
            .CreateTexture2D(&staging_desc, None, Some(&mut staging))
            .map_err(|e| format!("CreateTexture2D staging: {e}"))?;
        let staging = staging.ok_or("staging texture handle null")?;

        let src_box = D3D11_BOX {
            left: monitor_x,
            top: monitor_y,
            right: monitor_x + crop_w,
            bottom: monitor_y + crop_h,
            front: 0,
            back: 1,
        };

        // 7. Capture loop. Target a uniform `interval_ms` between samples.
        // DXGI delivers new frames at the monitor's refresh rate; if no
        // new frame arrives within the per-iteration timeout we duplicate
        // the previous frame's pixels so the output cadence is uniform.
        // 7. Capture loop. Target a uniform `interval_ms` between
        // samples. DXGI delivers new frames at the monitor's refresh
        // rate; if no new frame arrives within the per-iteration timeout
        // we duplicate the previous frame's pixels so the output cadence
        // is uniform. Encode happens AFTER the loop, in a separate
        // spawn_blocking — libwebp's lossless add_frame is too expensive
        // at high DPI (~500ms/frame for a 3200×600 card) to live inside
        // the hot path without crashing the capture rate.
        let interval_ms_f = 1000.0 / fps as f64;
        let max_frames = ((duration_ms as f64 / 1000.0 * fps as f64).ceil() as u32).max(2);
        let start = std::time::Instant::now();

        let mut frames: Vec<Vec<u8>> = Vec::with_capacity(max_frames as usize);
        let mut frame_idx: u32 = 0;

        while frame_idx < max_frames {
            let target_ms = (frame_idx as f64 * interval_ms_f) as u64;
            let elapsed_ms = start.elapsed().as_millis() as u64;
            if elapsed_ms < target_ms {
                std::thread::sleep(Duration::from_millis(target_ms - elapsed_ms));
            }
            if start.elapsed().as_millis() as u64 >= duration_ms as u64 {
                break;
            }

            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            let timeout_ms = (interval_ms_f.ceil() as u32).clamp(5, 50);
            let acquire = duplication.AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource);

            match acquire {
                Ok(()) => {
                    let resource = resource.ok_or("AcquireNextFrame yielded null")?;
                    let desktop: ID3D11Texture2D = resource
                        .cast()
                        .map_err(|e| format!("cast desktop texture: {e}"))?;

                    // GPU-side region crop into the staging texture.
                    context.CopySubresourceRegion(
                        &staging,
                        0,
                        0,
                        0,
                        0,
                        &desktop,
                        0,
                        Some(&src_box),
                    );

                    // Map staging → CPU pointer. RowPitch can be larger
                    // than width*4 (alignment); copy row-by-row.
                    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                    context
                        .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                        .map_err(|e| format!("Map staging: {e}"))?;

                    let mut rgba = vec![0u8; (crop_w * crop_h * 4) as usize];
                    let row_bytes = (crop_w * 4) as usize;
                    for row in 0..crop_h {
                        let src_off = (row * mapped.RowPitch) as usize;
                        let dst_off = (row * crop_w * 4) as usize;
                        std::ptr::copy_nonoverlapping(
                            (mapped.pData as *const u8).add(src_off),
                            rgba.as_mut_ptr().add(dst_off),
                            row_bytes,
                        );
                    }
                    context.Unmap(&staging, 0);

                    // DXGI gives us BGRA; swap to RGBA in place for the
                    // WebP encoder.
                    for chunk in rgba.chunks_exact_mut(4) {
                        chunk.swap(0, 2);
                    }

                    frames.push(rgba);

                    // Must release before next AcquireNextFrame.
                    let _ = duplication.ReleaseFrame();
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    // No fresh frame; duplicate previous to keep cadence.
                    if let Some(last) = frames.last() {
                        frames.push(last.clone());
                    }
                    // No ReleaseFrame call needed — we didn't acquire.
                }
                Err(e) => {
                    return Err(format!("AcquireNextFrame: {e}"));
                }
            }

            frame_idx += 1;
        }

        Ok((frames, crop_w, crop_h))
    }
}

// Mask the four corner regions to a rounded-rectangle shape. Pixels
// outside the curve get alpha 0; pixels straddling get a 1-px
// anti-aliased falloff.
fn apply_rounded_corner_alpha(img: &mut RgbaImage, radius: u32) {
    let w = img.width();
    let h = img.height();
    let r = radius as f32;
    if r < 1.0 || w < radius * 2 || h < radius * 2 {
        return;
    }

    for y in 0..h {
        let yf = y as f32 + 0.5;
        let dy = if yf < r {
            yf - r
        } else if yf > h as f32 - r {
            yf - (h as f32 - r)
        } else {
            continue;
        };

        for x in 0..w {
            let xf = x as f32 + 0.5;
            let dx = if xf < r {
                xf - r
            } else if xf > w as f32 - r {
                xf - (w as f32 - r)
            } else {
                continue;
            };

            let dist = (dx * dx + dy * dy).sqrt();
            let alpha_mult = (r + 0.5 - dist).clamp(0.0, 1.0);
            if alpha_mult >= 1.0 {
                continue;
            }

            let pixel = img.get_pixel_mut(x, y);
            pixel[3] = (pixel[3] as f32 * alpha_mult) as u8;
        }
    }
}

// xcap helpers for the static path.

#[cfg(windows)]
fn capture_monitor_region(
    monitor: &Monitor,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<RgbaImage, String> {
    let captured: RgbaImage = monitor
        .capture_image()
        .map_err(|e| format!("capture_image: {e}"))?;

    let mx = monitor.x();
    let my = monitor.y();
    let local_x = (x - mx).max(0) as u32;
    let local_y = (y - my).max(0) as u32;
    let max_w = captured.width().saturating_sub(local_x);
    let max_h = captured.height().saturating_sub(local_y);
    let crop_w = width.min(max_w);
    let crop_h = height.min(max_h);

    if crop_w == 0 || crop_h == 0 {
        return Err("requested region falls outside monitor bounds".into());
    }

    Ok(image::imageops::crop_imm(&captured, local_x, local_y, crop_w, crop_h).to_image())
}

#[cfg(windows)]
fn pick_monitor_idx(monitors: &[Monitor], x: i32, y: i32) -> usize {
    monitors
        .iter()
        .position(|m| contains_point(m, x, y))
        .or_else(|| monitors.iter().position(|m| m.is_primary()))
        .unwrap_or(0)
}

#[cfg(windows)]
fn contains_point(monitor: &Monitor, x: i32, y: i32) -> bool {
    let mx = monitor.x();
    let my = monitor.y();
    let mw = monitor.width() as i32;
    let mh = monitor.height() as i32;
    x >= mx && x < mx + mw && y >= my && y < my + mh
}
