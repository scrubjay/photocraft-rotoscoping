//! Layer › Video Layers (and Rasterize › Video). A video layer holds a stack of frames; the
//! layer's displayed raster content is the frame at the timeline playhead, kept in sync by [`sync`]
//! after any edit or navigation. Headless and scriptable. Frame persistence in `.pcraft` is a
//! follow-up; a saved document keeps the current frame as the layer's content.

use photocraft_doc::{Document, Layer, LayerContent, LayerId, Timeline, VideoData};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Set every video layer's displayed content to its frame at the playhead.
pub fn sync(doc: &mut Document) {
    let cur = doc.timeline.as_ref().map_or(0, |t| t.current);
    sync_layers(&mut doc.layers, cur);
}

fn sync_layers(layers: &mut [Layer], cur: usize) {
    for l in layers.iter_mut() {
        let frame = l.video.as_ref().filter(|v| !v.frames.is_empty()).map(|v| v.frames[cur.min(v.frames.len() - 1)].clone());
        if let Some(f) = frame
            && let LayerContent::Raster(s) = &mut l.content
        {
            *s = f;
        }
        if let Some(ch) = l.children_mut() {
            sync_layers(ch, cur);
        }
    }
}

/// Commit every video layer's active raster content back to its frame stack at `frame_idx`.
pub fn commit_frame(doc: &mut Document, frame_idx: usize) {
    commit_layers(&mut doc.layers, frame_idx);
}

fn commit_layers(layers: &mut [Layer], frame_idx: usize) {
    for l in layers.iter_mut() {
        if let Some(v) = &mut l.video
            && let LayerContent::Raster(s) = &l.content
        {
            if frame_idx < v.frames.len() {
                v.frames[frame_idx] = s.clone();
            } else if frame_idx < 100_000 {
                while v.frames.len() <= frame_idx {
                    v.frames.push(s.clone());
                }
            }
        }
        if let Some(ch) = l.children_mut() {
            commit_layers(ch, frame_idx);
        }
    }
}

/// The active layer, if it's a video layer.
fn active_video(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document")?;
    match st.active_layer.and_then(|id| st.doc.layer(id)) {
        Some(l) if l.video.is_some() => Ok(()),
        _ => Err("select a video layer".into()),
    }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document".into())
}

/// Edit the active video layer's [`VideoData`] at the playhead, then re-sync the displayed frame.
fn edit_video(s: &mut Session, label: &str, f: impl FnOnce(&mut VideoData, usize) -> Result<()>) -> Result<Value> {
    let count = s.edit(label, |doc, active| {
        let cur = doc.timeline.as_ref().map_or(0, |t| t.current);
        commit_frame(doc, cur);
        let id = active.ok_or(EngineError::NoDocument)?;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let v = l.video.as_mut().ok_or_else(|| EngineError::BadParams { cmd: label.into(), msg: "the active layer is not a video layer".into() })?;
        f(v, cur)?;
        let n = v.frames.len();
        if let Some(t) = &mut doc.timeline {
            t.duration = t.duration.max(n.max(1));
            t.work_end = t.work_end.max(t.duration);
            t.clamp();
        }
        sync(doc);
        Ok(n)
    })?;
    Ok(json!({"frames": count}))
}

fn blank_of(fmt: photocraft_color::PixelFormat) -> Surface {
    Surface::new(fmt)
}

fn new_blank(s: &mut Session, _p: &Value) -> Result<Value> {
    let id = s.edit("New Blank Video Layer", |doc, active| {
        let fmt = doc.pixel_format();
        if doc.timeline.is_none() {
            doc.timeline = Some(Timeline::new(1, 30.0));
        }
        let fps = doc.timeline.as_ref().map_or(30.0, |t| t.fps);
        let n = doc.timeline.as_ref().map_or(1, |t| t.duration).max(1);
        let frames: Vec<Surface> = (0..n).map(|_| Surface::new(fmt)).collect();
        let name = doc.next_layer_name("Video Layer");
        let mut l = Layer::raster(name, fmt);
        l.video = Some(VideoData::new(frames, fps));
        let id = doc.insert_above(*active, l);
        *active = Some(id);
        sync(doc);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0}))
}

fn insert_blank(s: &mut Session, _p: &Value) -> Result<Value> {
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    edit_video(s, "Insert Blank Frame", move |v, cur| {
        let at = (cur + 1).min(v.frames.len());
        v.frames.insert(at, blank_of(fmt));
        Ok(())
    })
}

fn duplicate_frame(s: &mut Session, _p: &Value) -> Result<Value> {
    edit_video(s, "Duplicate Frame", |v, cur| {
        if let Some(f) = v.frames.get(cur).cloned() {
            v.frames.insert(cur + 1, f);
        }
        Ok(())
    })
}

fn delete_frame(s: &mut Session, _p: &Value) -> Result<Value> {
    edit_video(s, "Delete Frame", |v, cur| {
        if v.frames.len() > 1 && cur < v.frames.len() {
            v.frames.remove(cur);
        }
        Ok(())
    })
}

fn restore_frame(s: &mut Session, _p: &Value) -> Result<Value> {
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    edit_video(s, "Restore Frame", move |v, cur| {
        if let Some(f) = v.frames.get_mut(cur) {
            *f = blank_of(fmt);
        }
        Ok(())
    })
}

fn restore_all(s: &mut Session, _p: &Value) -> Result<Value> {
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    edit_video(s, "Restore All Frames", move |v, _| {
        for f in v.frames.iter_mut() {
            *f = blank_of(fmt);
        }
        Ok(())
    })
}

fn show_altered(s: &mut Session, _p: &Value) -> Result<Value> {
    edit_video(s, "Show Altered Video", |v, _| {
        v.show_altered = !v.show_altered;
        Ok(())
    })
}

/// Rasterize: drop the frame stack, keeping the current frame as a normal pixel layer.
fn rasterize(s: &mut Session, _p: &Value) -> Result<Value> {
    s.edit("Rasterize Video", |doc, active| {
        let id = active.ok_or(EngineError::NoDocument)?;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        // The content already shows the current frame (kept in sync); just drop the stack.
        l.video = None;
        Ok(())
    })?;
    Ok(json!({"rasterized": true}))
}

// ------------------------------------------------------------------ file-based footage

fn find_ffmpeg() -> Option<std::path::PathBuf> {
    if let Ok(status) = std::process::Command::new("ffmpeg").arg("-version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status() {
        if status.success() {
            return Some("ffmpeg".into());
        }
    }
    #[cfg(windows)]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let p = std::path::PathBuf::from(local_app_data);
            let winget_dir = p.join("Microsoft").join("WinGet").join("Packages");
            if winget_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&winget_dir) {
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
            }
            let winapps = p.join("Microsoft").join("WindowsApps").join("ffmpeg.exe");
            if winapps.exists() {
                return Some(winapps);
            }
        }
        if let Ok(prog_files) = std::env::var("ProgramFiles") {
            let pf = std::path::PathBuf::from(prog_files).join("ffmpeg").join("bin").join("ffmpeg.exe");
            if pf.exists() {
                return Some(pf);
            }
        }
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let candidate = parent.join("ffmpeg.exe");
            if candidate.exists() {
                return Some(candidate);
            }
            let candidate_unix = parent.join("ffmpeg");
            if candidate_unix.exists() {
                return Some(candidate_unix);
            }
        }
    }
    None
}

fn is_video_file(p: &std::path::Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("mp4" | "mov" | "avi" | "mkv" | "webm" | "wmv" | "m4v" | "mpg" | "mpeg" | "flv")
    )
}

fn extract_video_frames_with_ffmpeg(path: &str, fmt: photocraft_color::PixelFormat) -> Result<Vec<Surface>> {
    let ffmpeg_bin = find_ffmpeg().ok_or_else(|| {
        EngineError::Other("Video decoding requires FFmpeg. FFmpeg was not found. Please install FFmpeg (e.g. winget install Gyan.FFmpeg.Essentials) or load an animated GIF or image sequence.".to_string())
    })?;

    let unique_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let temp_dir = std::env::temp_dir().join(format!("photocraft_video_{unique_id}"));
    std::fs::create_dir_all(&temp_dir).map_err(|e| EngineError::Other(format!("create temp dir: {e}")))?;

    let out_pattern = temp_dir.join("frame_%06d.png");
    let output = std::process::Command::new(ffmpeg_bin)
        .arg("-y")
        .arg("-i")
        .arg(path)
        .arg(&out_pattern)
        .output();

    let Ok(output) = output else {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(EngineError::Other("failed to launch FFmpeg process".to_string()));
    };

    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_dir_all(&temp_dir);
        let detail = err_msg.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
        return Err(EngineError::Other(format!("FFmpeg failed to extract frames from `{path}`: {detail}")));
    }

    let Ok(dir_entries) = std::fs::read_dir(&temp_dir) else {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(EngineError::Other("failed to read temp dir".to_string()));
    };

    let mut frame_files: Vec<std::path::PathBuf> = dir_entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "png"))
        .collect();
    frame_files.sort();

    if frame_files.is_empty() {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(EngineError::Other(format!("no frames extracted by FFmpeg for `{path}`")));
    }

    let mut frames = Vec::with_capacity(frame_files.len());
    for f in &frame_files {
        let bytes = std::fs::read(f).map_err(|e| {
            let _ = std::fs::remove_dir_all(&temp_dir);
            EngineError::Other(format!("read frame: {e}"))
        })?;
        let name = f.to_string_lossy();
        let doc = photocraft_io::import(&name, &bytes).map_err(|e| {
            let _ = std::fs::remove_dir_all(&temp_dir);
            EngineError::Other(format!("decode frame: {e}"))
        })?.document;
        frames.push(crate::file_cmds::flattened(&doc, fmt));
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(frames)
}

/// Load frames from `path`: a video file (via FFmpeg), an animated GIF, a single image (1 frame),
/// or a directory of images (sorted = an image sequence). Each becomes a full raster frame in the document's format.
fn load_frames(path: &str, fmt: photocraft_color::PixelFormat) -> Result<Vec<Surface>> {
    let p = std::path::Path::new(path);

    // 1. Video files (MP4, MOV, WebM, etc.) via FFmpeg
    if !p.is_dir() && is_video_file(p) {
        return extract_video_frames_with_ffmpeg(path, fmt);
    }

    // 2. Animated GIF files
    if !p.is_dir() && p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() == Some("gif") {
        if let Ok(bytes) = std::fs::read(p) {
            if let Ok(images) = photocraft_codecs::decode_gif_frames(&bytes) {
                if !images.is_empty() {
                    let mut frames = Vec::with_capacity(images.len());
                    for (i, img) in images.iter().enumerate() {
                        if let Ok(r) = photocraft_io::image_to_document(&format!("Frame {}", i + 1), img) {
                            frames.push(crate::file_cmds::flattened(&r.document, fmt));
                        }
                    }
                    if !frames.is_empty() {
                        return Ok(frames);
                    }
                }
            }
        }
    }

    // 3. Image sequences (directory) or single images
    let is_img = |f: &std::path::Path| {
        matches!(
            f.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
            Some("png" | "jpg" | "jpeg" | "tif" | "tiff" | "webp" | "bmp" | "gif" | "tga" | "exr" | "pcraft" | "psd")
        )
    };
    let files: Vec<std::path::PathBuf> = if p.is_dir() {
        let mut v: Vec<_> = std::fs::read_dir(p)
            .map_err(|e| EngineError::Other(format!("read dir `{path}`: {e}")))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|f| is_img(f))
            .collect();
        v.sort();
        v
    } else {
        vec![p.to_path_buf()]
    };
    if files.is_empty() {
        return Err(EngineError::Other(format!("no images in `{path}`")));
    }
    let mut frames = Vec::with_capacity(files.len());
    for f in &files {
        let bytes = std::fs::read(f).map_err(|e| EngineError::Other(format!("read `{}`: {e}", f.display())))?;
        let name = f.to_string_lossy();
        let doc = photocraft_io::import(&name, &bytes).map_err(|e| EngineError::Other(format!("`{}`: {e}", f.display())))?.document;
        frames.push(crate::file_cmds::flattened(&doc, fmt));
    }
    Ok(frames)
}

fn new_from_file(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::BadParams { cmd: "layer.videoLayers.newVideoLayerFromFile".into(), msg: "need `path`".into() })?
        .to_string();
    let fps = p.get("fps").and_then(Value::as_f64).unwrap_or(30.0) as f32;
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let frames = load_frames(&path, fmt)?;
    let n = frames.len();
    let id = s.edit("New Video Layer from File", |doc, active| {
        if doc.timeline.is_none() {
            doc.timeline = Some(Timeline::new(n, fps));
        } else if let Some(t) = &mut doc.timeline {
            t.duration = t.duration.max(n);
            t.clamp();
        }
        let name = doc.next_layer_name(std::path::Path::new(&path).file_stem().and_then(|s| s.to_str()).unwrap_or("Video"));
        let mut l = Layer::raster(name, fmt);
        let mut v = VideoData::new(frames, fps);
        v.source = photocraft_doc::VideoSource::File { path: path.clone() };
        l.video = Some(v);
        let id = doc.insert_above(*active, l);
        *active = Some(id);
        sync(doc);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "frames": n}))
}

fn replace_footage(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::BadParams { cmd: "layer.videoLayers.replaceFootage".into(), msg: "need `path`".into() })?
        .to_string();
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let frames = load_frames(&path, fmt)?;
    edit_video(s, "Replace Footage", move |v, _| {
        v.frames = frames;
        v.source = photocraft_doc::VideoSource::File { path: path.clone() };
        Ok(())
    })
}

fn interpret_footage(s: &mut Session, p: &Value) -> Result<Value> {
    let fps = p.get("fps").and_then(Value::as_f64);
    edit_video(s, "Interpret Footage", move |v, _| {
        if let Some(f) = fps {
            v.fps = f as f32;
        }
        Ok(())
    })
}

fn reload_frame(s: &mut Session, _p: &Value) -> Result<Value> {
    // Reload the whole stack from its file source (what a frame reload needs for a sequence).
    let (src, fmt) = {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let id = st.active_layer.ok_or(EngineError::NoDocument)?;
        let v = st
            .doc
            .layer(id)
            .and_then(|l| l.video.as_ref())
            .ok_or_else(|| EngineError::BadParams { cmd: "layer.videoLayers.reloadFrame".into(), msg: "not a video layer".into() })?;
        (v.source.clone(), st.doc.pixel_format())
    };
    let photocraft_doc::VideoSource::File { path } = src else {
        return Err(EngineError::BadParams { cmd: "layer.videoLayers.reloadFrame".into(), msg: "this layer has no file source to reload".into() });
    };
    let frames = load_frames(&path, fmt)?;
    edit_video(s, "Reload Frame", move |v, _| {
        v.frames = frames;
        Ok(())
    })
}

/// Import › Video Frames to Layers: load frames from a file/sequence and add each as its own layer.
fn frames_to_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::BadParams { cmd: "file.import.videoFramesToLayers".into(), msg: "need `path`".into() })?
        .to_string();
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let frames = load_frames(&path, fmt)?;
    let n = frames.len();
    s.edit("Video Frames to Layers", |doc, active| {
        let mut above = *active;
        for (i, f) in frames.into_iter().enumerate() {
            let mut l = Layer::raster(format!("Frame {}", i + 1), fmt);
            if let LayerContent::Raster(surf) = &mut l.content {
                *surf = f;
            }
            above = Some(doc.insert_above(above, l));
        }
        *active = above;
        Ok(())
    })?;
    Ok(json!({"layers": n}))
}

/// Export › Render Video: composite the whole document at each timeline frame and write an image
/// sequence (`{dir}/{stem}_NNNN.{format}`). Clean-room; no proprietary codec.
fn render_video(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = p.get("dir").and_then(Value::as_str).ok_or_else(|| EngineError::BadParams { cmd: "file.export.renderVideo".into(), msg: "need `dir`".into() })?;
    let format = p.get("format").and_then(Value::as_str).unwrap_or("png");
    std::fs::create_dir_all(dir).map_err(|e| EngineError::Other(format!("mkdir `{dir}`: {e}")))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let base = (*st.doc).clone();
    let frames = base.timeline.as_ref().map_or(1, |t| t.duration).max(1);
    let fps = base.timeline.as_ref().map_or(30.0, |t| t.fps).max(1.0);
    let stem = base.name.rsplit_once('.').map_or(base.name.as_str(), |(a, _)| a).to_string();
    let fmt = base.pixel_format();
    let bounds = base.bounds();
    let (w, h) = (bounds.width() as usize, bounds.height() as usize);

    // Animated GIF: one file, every frame quantised to its own palette.
    if format.eq_ignore_ascii_case("gif") {
        use photocraft_algo::quantize::{self, Dither, Forced, PaletteKind};
        let delay = (100.0 / fps).round().clamp(1.0, 65535.0) as u16;
        let mut gframes = Vec::with_capacity(frames);
        for f in 0..frames {
            let mut doc = base.clone();
            if let Some(t) = &mut doc.timeline {
                t.current = f;
            }
            sync(&mut doc);
            let surf = crate::file_cmds::flattened(&doc, fmt);
            let mut px = vec![[0.0f32; 4]; w * h];
            surf.read_rgba_into(bounds, &mut px);
            let pal = quantize::build_palette(&px, PaletteKind::Adaptive, 256, Forced::None).map_err(EngineError::Other)?;
            let idx = quantize::quantize(&mut px, w, &pal, Dither::None, 1.0, None);
            gframes.push(photocraft_codecs::web::GifFrame { indices: idx, palette: pal, transparent: None, delay_cs: delay });
        }
        let gif = photocraft_codecs::web::encode_gif_animated(w as u32, h as u32, &gframes, true).map_err(|e| EngineError::Other(e.to_string()))?;
        let path = format!("{dir}/{stem}.gif");
        crate::file_cmds::write_file(&path, &gif)?;
        return Ok(json!({"frames": frames, "file": path}));
    }

    // MP4 / MOV Video export via FFmpeg
    if format.eq_ignore_ascii_case("mp4") || format.eq_ignore_ascii_case("mov") {
        let ffmpeg_bin = find_ffmpeg().ok_or_else(|| {
            EngineError::Other("Rendering video files requires FFmpeg. Please install FFmpeg (e.g. winget install Gyan.FFmpeg.Essentials) or export as an image sequence / animated GIF.".to_string())
        })?;
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let temp_dir = std::env::temp_dir().join(format!("photocraft_render_{unique_id}"));
        std::fs::create_dir_all(&temp_dir).map_err(|e| EngineError::Other(format!("create temp dir: {e}")))?;

        let opts = photocraft_io::ExportOptions::default();
        for f in 0..frames {
            let mut doc = base.clone();
            if let Some(t) = &mut doc.timeline {
                t.current = f;
            }
            sync(&mut doc);
            let frame_path = temp_dir.join(format!("frame_{f:06}.png"));
            let bytes = photocraft_io::export(&doc, "png", &opts)
                .map(|r| r.bytes)
                .map_err(|e| {
                    let _ = std::fs::remove_dir_all(&temp_dir);
                    EngineError::Other(format!("render frame {f}: {e}"))
                })?;
            if let Err(e) = std::fs::write(&frame_path, &bytes) {
                let _ = std::fs::remove_dir_all(&temp_dir);
                return Err(EngineError::Other(format!("write frame {f}: {e}")));
            }
        }

        let out_path = format!("{dir}/{stem}.{format}");
        let input_pattern = temp_dir.join("frame_%06d.png");
        let fps_str = format!("{fps:.3}");
        let output = std::process::Command::new(ffmpeg_bin)
            .arg("-y")
            .arg("-framerate")
            .arg(&fps_str)
            .arg("-i")
            .arg(&input_pattern)
            .arg("-c:v")
            .arg("libx264")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg(&out_path)
            .output();

        let _ = std::fs::remove_dir_all(&temp_dir);

        let Ok(output) = output else {
            return Err(EngineError::Other("failed to execute FFmpeg for video rendering".to_string()));
        };
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            let detail = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
            return Err(EngineError::Other(format!("FFmpeg video encoding failed: {detail}")));
        }
        return Ok(json!({"frames": frames, "file": out_path}));
    }

    // Image sequence.
    let opts = photocraft_io::ExportOptions::default();
    let mut files = Vec::new();
    for f in 0..frames {
        let mut doc = base.clone();
        if let Some(t) = &mut doc.timeline {
            t.current = f;
        }
        sync(&mut doc);
        let path = format!("{dir}/{stem}_{f:04}.{format}");
        let bytes = photocraft_io::export(&doc, format, &opts).map(|r| r.bytes).map_err(|e| EngineError::Other(format!("render frame {f}: {e}")))?;
        crate::file_cmds::write_file(&path, &bytes)?;
        files.push(path);
    }
    Ok(json!({"frames": files.len(), "dir": dir}))
}

pub fn split_at_playhead(s: &mut Session, _p: &Value) -> Result<Value> {
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let new_layer_id = s.edit("Split at Playhead", |doc, active| {
        let cur = doc.timeline.as_ref().map_or(0, |t| t.current);
        commit_frame(doc, cur);
        let id = active.ok_or(EngineError::NoDocument)?;
        let orig = doc.layer(id).ok_or(EngineError::NoLayer(id))?.clone();

        let new_id = if let Some(v) = &orig.video {
            let total_frames = v.frames.len();
            let (lower_frames, upper_frames) = if cur < total_frames {
                let split_pt = (cur + 1).min(total_frames);
                let lower = v.frames[..split_pt].to_vec();
                let mut upper = Vec::with_capacity(total_frames.max(split_pt + 1));
                for _ in 0..split_pt {
                    upper.push(blank_of(fmt));
                }
                if split_pt < total_frames {
                    upper.extend_from_slice(&v.frames[split_pt..]);
                } else {
                    upper.push(blank_of(fmt));
                }
                (lower, upper)
            } else {
                let lower = v.frames.clone();
                let mut upper = Vec::with_capacity(cur + 1);
                for _ in 0..cur {
                    upper.push(blank_of(fmt));
                }
                upper.push(blank_of(fmt));
                (lower, upper)
            };

            if let Some(l) = doc.layer_mut(id) {
                if let Some(v_mut) = &mut l.video {
                    v_mut.frames = lower_frames;
                }
            }

            let mut upper_layer = orig.duplicate();
            upper_layer.name = doc.next_layer_name(&format!("{} (Split)", orig.name));
            if let Some(v_upper) = &mut upper_layer.video {
                v_upper.frames = upper_frames;
            }
            doc.insert_above(Some(id), upper_layer)
        } else {
            let mut split_layer = orig.duplicate();
            split_layer.name = doc.next_layer_name(&format!("{} (Split)", orig.name));
            doc.insert_above(Some(id), split_layer)
        };

        if let Some(t) = &mut doc.timeline {
            let max_frames = doc.layers.iter().map(|l| l.video.as_ref().map_or(0, |v| v.frames.len())).max().unwrap_or(0);
            t.duration = t.duration.max(max_frames.max(1));
            t.work_end = t.work_end.max(t.duration);
            t.clamp();
        }

        *active = Some(new_id);
        sync(doc);
        Ok(new_id)
    })?;

    Ok(json!({"layer": new_layer_id.0}))
}

pub fn set_frame_count(s: &mut Session, p: &Value) -> Result<Value> {
    let count = p
        .get("count")
        .or_else(|| p.get("frames"))
        .or_else(|| p.get("duration"))
        .and_then(Value::as_u64)
        .ok_or_else(|| EngineError::BadParams {
            cmd: "layer.videoLayers.setFrameCount".into(),
            msg: "need `count` (frame count >= 1)".into(),
        })? as usize;
    let count = count.max(1).min(100_000);
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let target_layer = p.get("layer").and_then(Value::as_u64).map(LayerId);
    let r = s.edit("Change Video Frame Count", |doc, active| {
        let cur = doc.timeline.as_ref().map_or(0, |t| t.current);
        commit_frame(doc, cur);
        let id = target_layer.or(*active).ok_or(EngineError::NoDocument)?;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if let Some(v) = &mut l.video {
            let cur_len = v.frames.len();
            if count > cur_len {
                while v.frames.len() < count {
                    v.frames.push(blank_of(fmt));
                }
            } else if count < cur_len {
                v.frames.truncate(count);
            }
        }
        if let Some(t) = &mut doc.timeline {
            t.duration = t.duration.max(count);
            t.work_end = t.work_end.max(t.duration);
            t.clamp();
        }
        sync(doc);
        Ok(count)
    })?;
    Ok(json!({"frames": r}))
}

macro_rules! spec {
    ($id:expr, $label:expr, $menu:expr, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: "{} → {}", enabled: $en, journal: true, run: $run }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    let vl = &["Layer", "Video Layers"] as &[&str];
    vec![
        spec!("layer.videoLayers.newBlankVideoLayer", "New Blank Video Layer", vl, has_doc, |s, p| new_blank(s, p)),
        spec!("layer.videoLayers.insertBlankFrame", "Insert Blank Frame", vl, active_video, |s, p| insert_blank(s, p)),
        spec!("layer.videoLayers.duplicateFrame", "Duplicate Frame", vl, active_video, |s, p| duplicate_frame(s, p)),
        spec!("layer.videoLayers.deleteFrame", "Delete Frame", vl, active_video, |s, p| delete_frame(s, p)),
        spec!("layer.videoLayers.restoreFrame", "Restore Frame", vl, active_video, |s, p| restore_frame(s, p)),
        spec!("layer.videoLayers.restoreAllFrames", "Restore All Frames", vl, active_video, |s, p| restore_all(s, p)),
        spec!("layer.videoLayers.showAlteredVideo", "Show Altered Video", vl, active_video, |s, p| show_altered(s, p)),
        spec!("layer.videoLayers.rasterize", "Rasterize", vl, active_video, |s, p| rasterize(s, p)),
        spec!("layer.rasterize.video", "Video", &["Layer", "Rasterize"], active_video, |s, p| rasterize(s, p)),
        spec!("layer.videoLayers.splitAtPlayhead", "Split at Playhead", &[], has_doc, |s, p| split_at_playhead(s, p)),
        spec!("layer.videoLayers.setFrameCount", "Set Video Frame Count", &[], has_doc, |s, p| set_frame_count(s, p)),
        spec!("layer.videoLayers.newVideoLayerFromFile", "New Video Layer from File…", vl, has_doc, |s, p| new_from_file(s, p)),
        spec!("layer.videoLayers.replaceFootage", "Replace Footage…", vl, active_video, |s, p| replace_footage(s, p)),
        spec!("layer.videoLayers.interpretFootage", "Interpret Footage…", vl, active_video, |s, p| interpret_footage(s, p)),
        spec!("layer.videoLayers.reloadFrame", "Reload Frame", vl, active_video, |s, p| reload_frame(s, p)),
        spec!("file.import.videoFramesToLayers", "Video Frames to Layers…", &["File", "Import"], has_doc, |s, p| frames_to_layers(s, p)),
        spec!("file.export.renderVideo", "Render Video…", &["File", "Export"], has_doc, |s, p| render_video(s, p)),
    ]
}

#[cfg(test)]
mod tests;
