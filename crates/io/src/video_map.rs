//! Video layers and timeline <-> PSD image resources and frame-stack groups.
//!
//! When saving video layers to PSD, each video layer is exported as an open layer group
//! containing individual raster frames named `Frame {:03}` (reversed for bottom-to-top
//! PSD record order), and the document timeline is saved in resource [`TIMELINE_INFO`].
//!
//! On import, PSD groups whose child layers form a sequence of raster frames (`Frame 001`,
//! `Frame 002`, ...) are reassembled into native video layers with a [`VideoData`] frame
//! stack, and the document [`Timeline`] is restored or synthesized from the maximum frame count.

use photocraft_color::PixelFormat;
use photocraft_doc::{BlendMode, Document, Layer, LayerContent, Timeline, VideoData};
use photocraft_psd::{ImageResource, PsdFile};
use photocraft_raster::Surface;

/// PSD image resource ID for timeline parameters (PhotoCraft extension).
pub const TIMELINE_INFO: u16 = 4002;

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

/// Sets every video layer's displayed content to its frame at `cur`.
pub fn sync_video_layers(layers: &mut [Layer], cur: usize) {
    for l in layers.iter_mut() {
        if let Some(v) = &l.video
            && !v.frames.is_empty()
            && let LayerContent::Raster(s) = &mut l.content
        {
            let idx = cur.min(v.frames.len().saturating_sub(1));
            if let Some(f) = v.frames.get(idx) {
                *s = f.clone();
            }
        }
        if let Some(ch) = l.children_mut() {
            sync_video_layers(ch, cur);
        }
    }
}

/// Reconstructs video layers and restores or creates the document timeline.
pub fn import(doc: &mut Document, file: &PsdFile, warnings: &mut Vec<String>) {
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
    reassemble_video_layers(&mut doc.layers, fmt, default_fps);

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

