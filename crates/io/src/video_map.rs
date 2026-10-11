//! Video layers and timeline <-> PSD image resources, frame-stack groups, and Photoshop video/smart layers.
//!
//! When saving video layers to PSD, each video layer is exported as an open layer group
//! containing individual raster frames named `Frame {:03}` (reversed for bottom-to-top
//! PSD record order), and the document timeline is saved in resource [`TIMELINE_INFO`] (PhotoCraft)
//! and [`PHOTOSHOP_TIMELINE_INFO`] (Photoshop native ActionDescriptor).
//!
//! On import:
//! 1. PSD groups whose child layers form a sequence of raster frames (`Frame 001`, `Frame 002`, ...)
//!    are reassembled into native video layers with a [`VideoData`] frame stack.
//! 2. Photoshop layers containing `PxSc` (PixelSource / FrameReader) tagged blocks are linked to their
//!    source video footage (MP4, MOV, etc.) and decoded into [`VideoData`] stacks.
//! 3. Placed Smart Objects containing embedded animated PSDs or video files have their frame stacks
//!    resolved so they animate across the document timeline.
//! 4. The document [`Timeline`] is restored from [`PHOTOSHOP_TIMELINE_INFO`] (1075) or [`TIMELINE_INFO`] (4002),
//!    or synthesized from the maximum frame count across all video layers.

use std::path::{Path, PathBuf};

use photocraft_color::PixelFormat;
use photocraft_doc::{BlendMode, Document, Layer, LayerContent, Timeline, VideoData, VideoSource};
use photocraft_psd::descriptor::{Descriptor, Value, VersionedDescriptor};
use photocraft_psd::{ImageResource, PsdFile};
use photocraft_raster::Surface;

/// PSD image resource ID for timeline parameters (PhotoCraft extension).
pub const TIMELINE_INFO: u16 = 4002;

/// Photoshop native timeline parameters image resource ID (0x0433).
pub const PHOTOSHOP_TIMELINE_INFO: u16 = 1075;

/// Photoshop native onion skin parameters image resource ID (0x0436).
pub const PHOTOSHOP_ONION_SKIN: u16 = 1078;

/// Parses a frame index from a layer name like `Frame 001`, `frame_042`, `Frame-5`, `frame1`.
pub fn parse_frame_number(name: &str) -> Option<u32> {
    let s = name.trim();
    let lower = s.to_ascii_lowercase();
    let rest = lower.strip_prefix("frame")?;
    let trimmed = rest.trim_start_matches([' ', '_', '-']);
    let digits = if let Some(dot) = trimmed.rfind('.') {
        trimmed.get(..dot)?
    } else {
        trimmed
    };
    let digits = digits.trim();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok()
}

/// Checks if a group's children represent a video frame sequence.
pub fn is_frame_sequence(children: &[Layer]) -> bool {
    if children.len() < 2 {
        return false;
    }
    let total = children.len();
    let frame_count = children
        .iter()
        .filter(|c| matches!(c.content, LayerContent::Raster(_)) && parse_frame_number(&c.name).is_some())
        .count();
    // Either all children are frames, or at least 90% are frames with at least 2 frames.
    frame_count >= 2 && (frame_count == total || frame_count.saturating_mul(10) >= total.saturating_mul(9))
}

/// Extracts frame surfaces from children, sorted by frame number.
fn extract_frames(children: &mut Vec<Layer>, fmt: PixelFormat) -> Option<Vec<Surface>> {
    if !is_frame_sequence(children) {
        return None;
    }
    let mut with_idx: Vec<(u32, Surface)> = Vec::with_capacity(children.len());
    for c in children.drain(..) {
        if let Some(num) = parse_frame_number(&c.name)
            && let LayerContent::Raster(s) = c.content
        {
            with_idx.push((num, s));
        }
    }
    if with_idx.len() < 2 {
        return None;
    }
    with_idx.sort_by_key(|(n, _)| *n);

    let min_num = with_idx.first().map(|p| p.0).unwrap_or(0);
    let max_num = with_idx.last().map(|p| p.0).unwrap_or(0);
    let span = (max_num.saturating_sub(min_num).saturating_add(1)) as usize;

    if span == with_idx.len() {
        // Contiguous frame sequence (standard PhotoCraft export or Photoshop export).
        Some(with_idx.into_iter().map(|(_, s)| s).collect())
    } else if span > with_idx.len() && span <= with_idx.len().saturating_add(100) {
        // Small gaps in frame sequence (fill gap slots with blank surfaces).
        let mut frames: Vec<Option<Surface>> = (0..span).map(|_| None).collect();
        for (num, surf) in with_idx {
            let idx = num.saturating_sub(min_num) as usize;
            if let Some(slot) = frames.get_mut(idx) {
                *slot = Some(surf);
            }
        }
        let result = frames
            .into_iter()
            .map(|opt| opt.unwrap_or_else(|| Surface::new(fmt)))
            .collect();
        Some(result)
    } else {
        // Non-contiguous sequence with large gaps: preserve sorted frames sequentially.
        Some(with_idx.into_iter().map(|(_, s)| s).collect())
    }
}

/// Recursively reassembles frame-stack groups into video layers.
pub fn reassemble_video_layers(layers: &mut [Layer], fmt: PixelFormat, fps: f32) -> bool {
    let mut any_converted = false;
    for l in layers.iter_mut() {
        if let LayerContent::Group(g) = &mut l.content {
            if reassemble_video_layers(&mut g.children, fmt, fps) {
                any_converted = true;
            }
            if let Some(frames) = extract_frames(&mut g.children, fmt) {
                let first_frame = frames.first().cloned().unwrap_or_else(|| Surface::new(fmt));
                l.content = LayerContent::Raster(first_frame);
                l.video = Some(VideoData::new(frames, fps));
                if l.blend == BlendMode::PassThrough {
                    l.blend = BlendMode::Normal;
                }
                any_converted = true;
            }
        }
    }
    any_converted
}

/// Locates FFmpeg on the system, checking common install paths, packages, and portable locations.
pub fn find_ffmpeg() -> Option<PathBuf> {
    for env_var in ["PHOTOCRAFT_FFMPEG", "FFMPEG_PATH", "FFMPEG"] {
        if let Ok(val) = std::env::var(env_var) {
            let p = PathBuf::from(val);
            if p.is_file() {
                return Some(p);
            }
            let candidate = p.join("ffmpeg.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
            let candidate_bin = p.join("bin").join("ffmpeg.exe");
            if candidate_bin.is_file() {
                return Some(candidate_bin);
            }
        }
    }

    if let Ok(status) = std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        && status.success() {
            return Some("ffmpeg".into());
        }

    #[cfg(windows)]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let p = PathBuf::from(local_app_data);
            let winget_dir = p.join("Microsoft").join("WinGet").join("Packages");
            if winget_dir.exists()
                && let Ok(entries) = std::fs::read_dir(&winget_dir)
            {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.to_string_lossy().to_ascii_lowercase().contains("ffmpeg") {
                        if let Ok(sub_entries) = std::fs::read_dir(&path) {
                            for sub in sub_entries.flatten() {
                                let sub_path = sub.path();
                                let candidate = sub_path.join("bin").join("ffmpeg.exe");
                                if candidate.exists() {
                                    return Some(candidate);
                                }
                            }
                        }
                        let bin2 = path.join("bin").join("ffmpeg.exe");
                        if bin2.exists() {
                            return Some(bin2);
                        }
                    }
                }
            }
            let winapps = p.join("Microsoft").join("WindowsApps").join("ffmpeg.exe");
            if winapps.exists() {
                return Some(winapps);
            }
            let local_prog = p.join("Programs").join("ffmpeg").join("bin").join("ffmpeg.exe");
            if local_prog.exists() {
                return Some(local_prog);
            }
        }
        if let Ok(prog_files) = std::env::var("ProgramFiles") {
            let pf = PathBuf::from(prog_files).join("ffmpeg").join("bin").join("ffmpeg.exe");
            if pf.exists() {
                return Some(pf);
            }
        }
        if let Ok(user_profile) = std::env::var("USERPROFILE") {
            let dl = PathBuf::from(user_profile).join("Downloads");
            if dl.exists()
                && let Some(found) = search_ffmpeg_in_dir(&dl, 5) {
                    return Some(found);
                }
        }
    }

    if let Ok(exe_path) = std::env::current_exe()
        && let Some(parent) = exe_path.parent()
    {
        let candidate = parent.join("ffmpeg.exe");
        if candidate.exists() {
            return Some(candidate);
        }
        let candidate_unix = parent.join("ffmpeg");
        if candidate_unix.exists() {
            return Some(candidate_unix);
        }
    }

    None
}

#[cfg(windows)]
fn search_ffmpeg_in_dir(dir: &Path, max_depth: usize) -> Option<PathBuf> {
    if max_depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("ffmpeg.exe")) {
            return Some(p);
        }
        if p.is_dir() {
            let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if name.starts_with('.') || name == "node_modules" || name == "target" || name == "windows" {
                continue;
            }
            if let Some(found) = search_ffmpeg_in_dir(&p, max_depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

/// Decodes frames from a video file using FFmpeg into surfaces in `fmt`.
pub fn extract_video_frames(video_path: &Path, fmt: PixelFormat) -> Result<Vec<Surface>, String> {
    let ffmpeg_bin = find_ffmpeg().ok_or_else(|| {
        "Video decoding requires FFmpeg. FFmpeg executable was not found.".to_string()
    })?;

    let unique_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let temp_dir = std::env::temp_dir().join(format!("photocraft_vid_{unique_id}"));
    std::fs::create_dir_all(&temp_dir).map_err(|e| format!("create temp dir: {e}"))?;

    let out_pattern = temp_dir.join("frame_%06d.png");
    let output = std::process::Command::new(ffmpeg_bin)
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg(&out_pattern)
        .output();

    let Ok(output) = output else {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err("failed to launch FFmpeg process".to_string());
    };

    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_dir_all(&temp_dir);
        let detail = err_msg.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
        return Err(format!("FFmpeg failed to extract frames from `{}`: {detail}", video_path.display()));
    }

    let dir_entries = match std::fs::read_dir(&temp_dir) {
        Ok(e) => e,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&temp_dir);
            return Err(format!("read temp dir: {e}"));
        }
    };

    let mut frame_files: Vec<PathBuf> = dir_entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "png"))
        .collect();
    frame_files.sort();

    if frame_files.is_empty() {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(format!("no frames extracted by FFmpeg for `{}`", video_path.display()));
    }

    let mut frames = Vec::with_capacity(frame_files.len());
    for f in &frame_files {
        let bytes = match std::fs::read(f) {
            Ok(b) => b,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&temp_dir);
                return Err(format!("read frame: {e}"));
            }
        };
        let name = f.to_string_lossy();
        let imp = match crate::flat::import_flat(&name, &bytes) {
            Ok(i) => i,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&temp_dir);
                return Err(format!("decode frame: {e}"));
            }
        };
        let surf = photocraft_compose::flatten_to_surface(&imp.document, fmt, None);
        frames.push(surf);
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(frames)
}

fn as_desc(v: Option<&Value>) -> Option<&Descriptor> {
    match v {
        Some(Value::Descriptor(d)) | Some(Value::GlobalObject(d)) => Some(d),
        _ => None,
    }
}

fn as_num(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Integer(i)) => Some(*i as f64),
        Some(Value::LargeInteger(i)) => Some(*i as f64),
        Some(Value::Double(d)) => Some(*d),
        _ => None,
    }
}

/// Parses Photoshop's native Timeline Information image resource (ID 1075).
pub fn parse_photoshop_timeline_resource(data: &[u8]) -> Option<Timeline> {
    let (vd, _) = VersionedDescriptor::parse_prefix(data).ok()?;
    let desc = &vd.descriptor;

    let step_desc = as_desc(desc.get("frameStep"));
    let step_num = step_desc.and_then(|s| as_num(s.get("numerator"))).unwrap_or(100.0);
    let step_den = step_desc.and_then(|s| as_num(s.get("denominator"))).unwrap_or(2997.0);

    let fps = match desc.get("frameRate") {
        Some(Value::Double(d)) if *d > 0.0 => *d as f32,
        _ => {
            if step_num > 0.0 {
                (step_den / step_num) as f32
            } else {
                30.0
            }
        }
    };

    let time_to_frames = |obj: Option<&Descriptor>| -> Option<usize> {
        let d = obj?;
        let num = as_num(d.get("numerator"))?;
        let den = as_num(d.get("denominator"))?;
        if den <= 0.0 || step_num <= 0.0 {
            return None;
        }
        let secs = num / den;
        let step_secs = step_num / step_den;
        let frames = (secs / step_secs).round() as isize;
        Some(frames.max(0) as usize)
    };

    let duration_frames = time_to_frames(as_desc(desc.get("duration"))).unwrap_or(0);
    let current_frame = time_to_frames(as_desc(desc.get("time"))).unwrap_or(0);
    let work_start = time_to_frames(as_desc(desc.get("workInTime"))).unwrap_or(0);
    let work_end = time_to_frames(as_desc(desc.get("workOutTime"))).unwrap_or(duration_frames);

    let d = duration_frames.max(1);
    let mut tl = Timeline::new(d, fps);
    tl.current = current_frame.min(d.saturating_sub(1));
    tl.work_start = work_start.min(d.saturating_sub(1));
    tl.work_end = if work_end > work_start { work_end.min(d) } else { d };
    Some(tl)
}

fn resolve_path_candidates(
    orig: Option<&str>,
    full: Option<&str>,
    rel: Option<&str>,
    nm: Option<&str>,
    doc_name: &str,
) -> Option<PathBuf> {
    // 1. Direct check of originalPath
    if let Some(p) = orig.filter(|s| !s.trim().is_empty()) {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }

    // 2. Direct check of fullPath URI
    if let Some(p) = full.filter(|s| !s.trim().is_empty()) {
        let clean = p
            .strip_prefix("file:///")
            .or_else(|| p.strip_prefix("file://"))
            .unwrap_or(p);
        let unescaped = clean.replace("%20", " ");
        let pb = PathBuf::from(&unescaped);
        if pb.is_file() {
            return Some(pb);
        }
    }

    // 3. Relative to document file location
    let doc_path = Path::new(doc_name);
    let doc_dir = if doc_path.is_file() {
        doc_path.parent()
    } else if doc_path.is_dir() {
        Some(doc_path)
    } else {
        doc_path.parent().filter(|p| !p.as_os_str().is_empty())
    };

    if let Some(dir) = doc_dir {
        for candidate in [rel, nm].into_iter().flatten() {
            let pb = dir.join(candidate);
            if pb.is_file() {
                return Some(pb);
            }
        }
    }

    // 4. Current working directory
    for candidate in [rel, nm].into_iter().flatten() {
        let pb = PathBuf::from(candidate);
        if pb.is_file() {
            return Some(pb);
        }
    }

    // 5. User Downloads folder
    #[cfg(windows)]
    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        let dl = PathBuf::from(user_profile).join("Downloads");
        for candidate in [rel, nm].into_iter().flatten() {
            let pb = dl.join(candidate);
            if pb.is_file() {
                return Some(pb);
            }
        }
    }

    None
}

fn is_video_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.rsplit('.').next(),
        Some("mp4" | "mov" | "avi" | "mkv" | "webm" | "wmv" | "m4v" | "mpg" | "mpeg" | "flv")
    )
}

fn load_pxsc_video_layer(
    data: &[u8],
    fmt: PixelFormat,
    default_fps: f32,
    doc_name: &str,
) -> Option<VideoData> {
    let (vd, _) = VersionedDescriptor::parse_prefix(data).ok()?;
    let desc = &vd.descriptor;
    let fr = as_desc(desc.get("frameReader"))?;
    let lnk = as_desc(fr.get("Lnk ")).or_else(|| as_desc(fr.get("Lnk")))?;

    let text_val = |key: &str| -> Option<String> {
        match lnk.get(key) {
            Some(Value::Text(t)) => Some(t.to_string_lossy()),
            _ => None,
        }
    };

    let original_path = text_val("originalPath");
    let full_path = text_val("fullPath");
    let rel_path = text_val("relPath");
    let nm = text_val("Nm  ").or_else(|| text_val("Nm"));

    let resolved = resolve_path_candidates(
        original_path.as_deref(),
        full_path.as_deref(),
        rel_path.as_deref(),
        nm.as_deref(),
        doc_name,
    )?;

    let frames = extract_video_frames(&resolved, fmt).ok()?;
    if frames.is_empty() {
        return None;
    }

    let mut v = VideoData::new(frames, default_fps);
    v.source = VideoSource::File {
        path: resolved.to_string_lossy().into_owned(),
    };
    Some(v)
}

/// Recursively resolves Photoshop `PxSc` video layers and animated Smart Objects.
fn resolve_video_layer(
    l: &mut Layer,
    metadata: &photocraft_doc::Metadata,
    fmt: PixelFormat,
    default_fps: f32,
    doc_name: &str,
    warnings: &mut Vec<String>,
) {
    if let Some(ch) = l.children_mut() {
        for c in ch {
            resolve_video_layer(c, metadata, fmt, default_fps, doc_name, warnings);
        }
    }

    if l.video.is_some() {
        return;
    }

    // 1. Check for PxSc tagged block (Photoshop video layer)
    if let Some((_, block_data)) = l.psd_blocks.iter().find(|(k, _)| k == b"PxSc")
        && let Some(v) = load_pxsc_video_layer(block_data, fmt, default_fps, doc_name) {
            let first_frame = v.frames.first().cloned().unwrap_or_else(|| Surface::new(fmt));
            l.content = LayerContent::Raster(first_frame);
            l.video = Some(v);
            return;
        }

    // 2. Check for animated Smart Object (referencing an embedded animated PSD or video)
    if let LayerContent::Smart(sm) = &mut l.content
        && let photocraft_doc::SmartSource::Linked { path: uuid } = &sm.source
            && let Some(linked) = crate::linked::find_linked_file(metadata, uuid) {
                if linked.file_name.to_ascii_lowercase().ends_with(".psd")
                    || linked.bytes.starts_with(b"8BPS")
                {
                    if let Ok(child_file) = photocraft_psd::PsdFile::from_bytes(&linked.bytes) {
                        let (mut child_doc, mut child_warnings) = crate::psd_import::psd_to_document(&child_file);
                        warnings.append(&mut child_warnings);
                        let duration = child_doc
                            .timeline
                            .as_ref()
                            .map_or(0, |t| t.duration)
                            .max(max_video_frames(&child_doc.layers));
                        if duration > 1 {
                            let fps = child_doc.timeline.as_ref().map_or(default_fps, |t| t.fps);
                            let mut frames = Vec::with_capacity(duration);
                            for f in 0..duration {
                                sync_video_layers(&mut child_doc.layers, f);
                                let rendered = photocraft_compose::flatten_to_surface(&child_doc, fmt, None);
                                frames.push(rendered);
                            }
                            if let Some(first) = frames.first().cloned() {
                                sm.cache = Some(first);
                            }
                            let mut v = VideoData::new(frames, fps);
                            v.source = VideoSource::File {
                                path: linked.file_name.clone(),
                            };
                            l.video = Some(v);
                        }
                    }
                } else if is_video_file_name(&linked.file_name)
                    && let Some(path) = resolve_path_candidates(None, None, Some(&linked.file_name), None, doc_name)
                        && let Ok(frames) = extract_video_frames(&path, fmt)
                            && !frames.is_empty() {
                                if let Some(first) = frames.first().cloned() {
                                    sm.cache = Some(first);
                                }
                                let mut v = VideoData::new(frames, default_fps);
                                v.source = VideoSource::File {
                                    path: path.to_string_lossy().into_owned(),
                                };
                                l.video = Some(v);
                            }
            }
}

/// Computes the maximum frame count across all video layers in the hierarchy.
pub fn max_video_frames(layers: &[Layer]) -> usize {
    let mut max_d = 0;
    for l in layers {
        if let Some(v) = &l.video {
            max_d = max_d.max(v.frames.len());
        }
        if let Some(ch) = l.children() {
            max_d = max_d.max(max_video_frames(ch));
        }
    }
    max_d
}

/// Sets every video layer's displayed content (raster surface or smart object cache) to its frame at `cur`.
pub fn sync_video_layers(layers: &mut [Layer], cur: usize) {
    for l in layers.iter_mut() {
        if let Some(v) = &l.video
            && !v.frames.is_empty()
        {
            let idx = cur.min(v.frames.len().saturating_sub(1));
            if let Some(f) = v.frames.get(idx) {
                match &mut l.content {
                    LayerContent::Raster(s) => *s = f.clone(),
                    LayerContent::Smart(sm) => sm.cache = Some(f.clone()),
                    _ => {}
                }
            }
        }
        if let Some(ch) = l.children_mut() {
            sync_video_layers(ch, cur);
        }
    }
}

/// Reconstructs video layers and restores or creates the document timeline.
pub fn import(doc: &mut Document, file: &PsdFile, warnings: &mut Vec<String>) {
    // 1. Restore timeline from Photoshop Resource 1075 (0x0433)
    if doc.timeline.is_none()
        && let Some(r) = file.resources.iter().find(|r| r.id == PHOTOSHOP_TIMELINE_INFO)
            && let Some(tl) = parse_photoshop_timeline_resource(&r.data) {
                doc.timeline = Some(tl);
            }

    // 2. Fall back to PhotoCraft TIMELINE_INFO (4002)
    if doc.timeline.is_none()
        && let Some(r) = file.resources.iter().find(|r| r.id == TIMELINE_INFO)
    {
        match serde_json::from_slice::<Timeline>(&r.data) {
            Ok(tl) => doc.timeline = Some(tl),
            Err(e) => warnings.push(format!("failed to parse timeline resource: {e}")),
        }
    }

    let default_fps = doc.timeline.as_ref().map_or(30.0, |t| t.fps);
    let fmt = doc.pixel_format();

    // 3. Frame-sequence groups (PhotoCraft export convention)
    reassemble_video_layers(&mut doc.layers, fmt, default_fps);

    // 4. Photoshop PxSc video layers & animated Smart Objects
    let doc_name = doc.name.clone();
    for l in doc.layers.iter_mut() {
        resolve_video_layer(l, &doc.metadata, fmt, default_fps, &doc_name, warnings);
    }

    // 5. Ensure timeline exists or is updated with the maximum frame count
    let max_frames = max_video_frames(&doc.layers);
    if max_frames > 0 {
        if doc.timeline.is_none() {
            doc.timeline = Some(Timeline::new(max_frames, default_fps));
        } else if let Some(t) = &mut doc.timeline {
            t.duration = t.duration.max(max_frames);
            t.work_end = t.work_end.max(t.duration);
            t.clamp();
        }
        let cur = doc.timeline.as_ref().map_or(0, |t| t.current);
        sync_video_layers(&mut doc.layers, cur);
    }
}

/// Builds a Photoshop native timeline resource (1075) for two-way Photoshop interoperability.
pub fn export_photoshop_timeline_resource(tl: &Timeline) -> Option<ImageResource> {
    let fps = tl.fps.max(1.0) as f64;
    let step_num = 100i32;
    let step_den = (100.0 * fps).round() as i32;
    let frame_step_desc = Descriptor::new("null")
        .with("numerator", Value::Integer(step_num))
        .with("denominator", Value::Integer(step_den));

    let to_time_desc = |frames: usize| -> Descriptor {
        let num = (frames as i32).saturating_mul(step_num);
        Descriptor::new("null")
            .with("numerator", Value::Integer(num))
            .with("denominator", Value::Integer(step_den))
    };

    let dur_desc = to_time_desc(tl.duration);
    let cur_desc = to_time_desc(tl.current);
    let ws_desc = to_time_desc(tl.work_start);
    let we_desc = to_time_desc(tl.work_end);

    let main_desc = Descriptor::new("null")
        .with("Vrsn", Value::Integer(1))
        .with("enab", Value::Boolean(true))
        .with("frameStep", Value::Descriptor(frame_step_desc))
        .with("frameRate", Value::Double(fps))
        .with("time", Value::Descriptor(cur_desc))
        .with("duration", Value::Descriptor(dur_desc))
        .with("workInTime", Value::Descriptor(ws_desc))
        .with("workOutTime", Value::Descriptor(we_desc))
        .with("hasMotion", Value::Boolean(true));

    let bytes = VersionedDescriptor::new(main_desc).to_bytes();
    Some(ImageResource::new(PHOTOSHOP_TIMELINE_INFO, bytes))
}

/// Serializes the document timeline to image resources for lossless round-trip.
pub fn export_resources(doc: &Document) -> Vec<ImageResource> {
    let Some(tl) = doc.timeline.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(bytes) = serde_json::to_vec(tl) {
        out.push(ImageResource::new(TIMELINE_INFO, bytes));
    }
    if let Some(ps_res) = export_photoshop_timeline_resource(tl) {
        out.push(ps_res);
    }
    out
}

/// Serializes the document timeline to an image resource for lossless round-trip.
pub fn export_resource(doc: &Document) -> Option<ImageResource> {
    let tl = doc.timeline.as_ref()?;
    let bytes = serde_json::to_vec(tl).ok()?;
    Some(ImageResource::new(TIMELINE_INFO, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::Group;
    use photocraft_geom::Size;

    #[test]
    fn parse_frame_numbers() {
        assert_eq!(parse_frame_number("Frame 001"), Some(1));
        assert_eq!(parse_frame_number("Frame 091"), Some(91));
        assert_eq!(parse_frame_number("frame_42"), Some(42));
        assert_eq!(parse_frame_number("frame-5"), Some(5));
        assert_eq!(parse_frame_number("frame12.png"), Some(12));
        assert_eq!(parse_frame_number("Frame 0"), Some(0));

        // Non-frame layer names must not match
        assert_eq!(parse_frame_number("Layer 1"), None);
        assert_eq!(parse_frame_number("Group 1"), None);
        assert_eq!(parse_frame_number("Snow 1"), None);
        assert_eq!(parse_frame_number("Background"), None);
        assert_eq!(parse_frame_number("Framework"), None);
        assert_eq!(parse_frame_number("frame"), None);
    }

    #[test]
    fn normal_group_not_converted() {
        let fmt = PixelFormat { mode: ColorMode::Rgb, sample: SampleType::U8, alpha: true };
        let c1 = Layer::new("Layer 1", LayerContent::Raster(Surface::new(fmt)));
        let c2 = Layer::new("Layer 2", LayerContent::Raster(Surface::new(fmt)));
        let group_layer = Layer::new("My Group", LayerContent::Group(Group { children: vec![c1, c2], expanded: true, artboard: None }));
        let mut layers = vec![group_layer];

        let converted = reassemble_video_layers(&mut layers, fmt, 30.0);
        assert!(!converted);
        assert!(matches!(layers[0].content, LayerContent::Group(_)));
        assert!(layers[0].video.is_none());
    }

    #[test]
    fn frame_sequence_group_converted() {
        let fmt = PixelFormat { mode: ColorMode::Rgb, sample: SampleType::U8, alpha: true };
        let c1 = Layer::new("Frame 003", LayerContent::Raster(Surface::new(fmt)));
        let c2 = Layer::new("Frame 002", LayerContent::Raster(Surface::new(fmt)));
        let c3 = Layer::new("Frame 001", LayerContent::Raster(Surface::new(fmt)));
        let group_layer = Layer::new("Video Layer", LayerContent::Group(Group { children: vec![c1, c2, c3], expanded: true, artboard: None }));
        let mut doc = Document::new("test", Size::new(100, 100), ColorMode::Rgb, SampleType::U8);
        doc.layers = vec![group_layer];

        let dummy_file = crate::document_to_psd(&Document::new("dummy", Size::new(10, 10), ColorMode::Rgb, SampleType::U8));
        let mut warnings = Vec::new();
        import(&mut doc, &dummy_file, &mut warnings);

        assert_eq!(doc.layers.len(), 1);
        assert!(!matches!(doc.layers[0].content, LayerContent::Group(_)));
        assert!(doc.layers[0].video.is_some());
        let v = doc.layers[0].video.as_ref().unwrap();
        assert_eq!(v.frames.len(), 3);
        assert!(doc.timeline.is_some());
        assert_eq!(doc.timeline.as_ref().unwrap().duration, 3);
    }
}
