//! Rotoscoping Onion Skinning: overlay ghosted preceding and succeeding frames
//! on the active document canvas so artists can rotoscope / animate frame-by-frame.

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use photocraft_doc::{Document, Layer};
use photocraft_geom::Rect;

use crate::PhotocraftApp;
use crate::timeline_ui::OnionSkinSettings;

pub struct OnionSkinCache {
    pub revision: u64,
    pub current_frame: usize,
    pub settings: OnionSkinSettings,
    pub tex: TextureHandle,
}

/// Ensure the onion skin overlay texture for document `idx` is up-to-date and return its texture id.
pub fn ensure(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize) -> Option<egui::TextureId> {
    if !app.ui.timeline.onion_skin.enabled {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    let tl = st.doc.timeline.as_ref()?;
    let cur = tl.current;
    let settings = app.ui.timeline.onion_skin.clone();

    let doc_id = st.doc.id.0;
    let revision = st.revision;

    // Check if cached texture is still valid
    if let Some(cache) = app.onion_skins.get(&doc_id)
        && cache.revision == revision && cache.current_frame == cur && cache.settings == settings
    {
        return Some(cache.tex.id());
    }

    // Find active video layer or topmost video layer
    let v_layer = find_video_layer(&st.doc, st.active_layer);
    let v = v_layer.and_then(|l| l.video.as_ref())?;
    if v.frames.is_empty() {
        return None;
    }

    let bounds = st.doc.bounds();
    let (w, h) = (bounds.width() as usize, bounds.height() as usize);
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return None;
    }

    // Allocate overlay pixel buffer
    let mut overlay = vec![Color32::TRANSPARENT; w * h];
    let max_a = settings.max_opacity.clamp(0.05, 1.0);
    let min_a = settings.min_opacity.clamp(0.01, max_a);
    let spacing = settings.frame_spacing.max(1);

    // 1. Render past frames (preceding)
    let n_before = settings.frames_before.min(10);
    for step in 1..=n_before {
        if let Some(frame_idx) = cur.checked_sub(step * spacing)
            && frame_idx < v.frames.len()
        {
            let factor = if n_before > 1 {
                (step - 1) as f32 / (n_before - 1) as f32
            } else {
                0.0
            };
            let alpha = max_a - factor * (max_a - min_a);
            blend_frame(&v.frames[frame_idx], bounds, &mut overlay, alpha, true, settings.tint_colors);
        }
    }

    // 2. Render future frames (succeeding)
    let n_after = settings.frames_after.min(10);
    for step in 1..=n_after {
        let frame_idx = cur + step * spacing;
        if frame_idx < v.frames.len() {
            let factor = if n_after > 1 {
                (step - 1) as f32 / (n_after - 1) as f32
            } else {
                0.0
            };
            let alpha = max_a - factor * (max_a - min_a);
            blend_frame(&v.frames[frame_idx], bounds, &mut overlay, alpha, false, settings.tint_colors);
        }
    }

    let img = ColorImage::new([w, h], overlay);
    let tex = ctx.load_texture(format!("pc-onion-skin-{}", doc_id), img, TextureOptions::LINEAR);
    let tex_id = tex.id();
    app.onion_skins.insert(doc_id, OnionSkinCache {
        revision,
        current_frame: cur,
        settings,
        tex,
    });

    Some(tex_id)
}

fn find_video_layer(doc: &Document, active: Option<photocraft_doc::LayerId>) -> Option<&Layer> {
    if let Some(id) = active
        && let Some(l) = doc.layer(id)
        && l.video.is_some()
    {
        return Some(l);
    }
    // Fall back to first layer with video data
    fn find_in_layers(layers: &[Layer]) -> Option<&Layer> {
        for l in layers {
            if l.video.is_some() {
                return Some(l);
            }
            if let Some(ch) = l.children()
                && let Some(found) = find_in_layers(ch)
            {
                return Some(found);
            }
        }
        None
    }
    find_in_layers(&doc.layers)
}

fn blend_frame(
    surf: &photocraft_raster::Surface,
    bounds: Rect,
    dst: &mut [Color32],
    opacity: f32,
    is_past: bool,
    tint: bool,
) {
    let mut px = vec![[0u8; 4]; dst.len()];
    surf.read_rgba8_into(bounds, &mut px);
    for (d, s) in dst.iter_mut().zip(px.iter()) {
        let src_a = (s[3] as f32 / 255.0) * opacity;
        if src_a <= 0.005 {
            continue;
        }
        let (sr, sg, sb) = if tint {
            if is_past {
                // Warm tint (reddish/orange) for past frames
                (
                    (s[0] as f32 * 1.2).min(255.0),
                    s[1] as f32 * 0.5,
                    s[2] as f32 * 0.5,
                )
            } else {
                // Cool tint (greenish/cyan) for future frames
                (
                    s[0] as f32 * 0.4,
                    (s[1] as f32 * 1.2).min(255.0),
                    (s[2] as f32 * 0.8).min(255.0),
                )
            }
        } else {
            (s[0] as f32, s[1] as f32, s[2] as f32)
        };

        // Standard alpha compositing over dst
        let da = d.a() as f32 / 255.0;
        let out_a = src_a + da * (1.0 - src_a);
        if out_a > 0.0 {
            let out_r = (sr * src_a + (d.r() as f32) * da * (1.0 - src_a)) / out_a;
            let out_g = (sg * src_a + (d.g() as f32) * da * (1.0 - src_a)) / out_a;
            let out_b = (sb * src_a + (d.b() as f32) * da * (1.0 - src_a)) / out_a;
            *d = Color32::from_rgba_premultiplied(
                (out_r * out_a).round() as u8,
                (out_g * out_a).round() as u8,
                (out_b * out_a).round() as u8,
                (out_a * 255.0).round() as u8,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;

    #[test]
    fn test_blend_frame_warm_tint() {
        let r = Rect::new(0, 0, 1, 1);
        let src = vec![200u8, 200, 200, 255];
        let surf = Surface::from_interleaved(PixelFormat::RGBA8, r, &src);
        let mut dst = vec![Color32::TRANSPARENT];
        blend_frame(&surf, r, &mut dst, 0.5, true, true);
        let pixel = dst[0];
        // Past frame with tint should boost red
        assert!(pixel.r() > 0);
        assert!(pixel.a() > 0);
    }

    #[test]
    fn test_blend_frame_cool_tint() {
        let r = Rect::new(0, 0, 1, 1);
        let src = vec![200u8, 200, 200, 255];
        let surf = Surface::from_interleaved(PixelFormat::RGBA8, r, &src);
        let mut dst = vec![Color32::TRANSPARENT];
        blend_frame(&surf, r, &mut dst, 0.5, false, true);
        let pixel = dst[0];
        // Future frame with tint should boost green
        assert!(pixel.g() > 0);
        assert!(pixel.a() > 0);
    }

    #[test]
    fn test_blend_frame_zero_alpha() {
        let r = Rect::new(0, 0, 1, 1);
        let src = vec![255u8, 255, 255, 0];
        let surf = Surface::from_interleaved(PixelFormat::RGBA8, r, &src);
        let mut dst = vec![Color32::TRANSPARENT];
        blend_frame(&surf, r, &mut dst, 0.5, true, true);
        assert_eq!(dst[0], Color32::TRANSPARENT);
    }
}


