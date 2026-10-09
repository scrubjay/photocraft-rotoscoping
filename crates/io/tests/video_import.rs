use std::path::Path;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, Timeline, VideoData};
use photocraft_geom::Size;
use photocraft_io::{document_to_psd, psd_to_document};
use photocraft_psd::PsdFile;
use photocraft_raster::Surface;

#[test]
fn test_roto_import_order() {
    let p = Path::new(r"C:\Users\fschmidt\Downloads\RotoSnowtest.psd");
    if !p.exists() {
        return;
    }
    let Ok(bytes) = std::fs::read(p) else { return };
    let Ok(file) = PsdFile::from_bytes(&bytes) else { return };
    let (doc, _warnings) = psd_to_document(&file);
    assert_eq!(doc.layers.len(), 2, "Expected 2 reassembled video layers");
    assert!(doc.timeline.is_some(), "Expected document timeline to be active");
    let tl = doc.timeline.as_ref().unwrap();
    assert_eq!(tl.duration, 91, "Expected 91 frames duration on timeline");

    for l in &doc.layers {
        assert!(!matches!(l.content, LayerContent::Group(_)), "Layer should not be a raw group");
        assert!(l.video.is_some(), "Layer should have video data");
        let v = l.video.as_ref().unwrap();
        assert_eq!(v.frames.len(), 91, "Expected 91 video frames");
    }

    // Round-trip export to PSD and re-import
    let exported = document_to_psd(&doc);
    let (back_doc, _) = psd_to_document(&exported);
    assert_eq!(back_doc.layers.len(), 2, "Expected 2 video layers after round-trip");
    assert!(back_doc.timeline.is_some(), "Expected timeline to survive round-trip");
    let back_tl = back_doc.timeline.as_ref().unwrap();
    assert_eq!(back_tl.duration, 91, "Expected timeline duration 91 after round-trip");
    for (i, l) in back_doc.layers.iter().enumerate() {
        assert!(l.video.is_some(), "Layer {i} should have video data after round-trip");
        let v = l.video.as_ref().unwrap();
        assert_eq!(v.frames.len(), 91, "Layer {i} should have 91 frames after round-trip");
    }
}

#[test]
fn test_synthetic_video_psd_roundtrip() {
    let mut doc = Document::new("Anim", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
    let fmt = PixelFormat {
        mode: ColorMode::Rgb,
        sample: SampleType::U8,
        alpha: true,
    };
    let frames = vec![Surface::new(fmt), Surface::new(fmt), Surface::new(fmt), Surface::new(fmt)];
    let mut layer = Layer::new("Clip 1", LayerContent::Raster(frames[0].clone()));
    layer.video = Some(VideoData::new(frames, 24.0));
    doc.layers = vec![layer];
    doc.timeline = Some(Timeline::new(4, 24.0));

    let exported = document_to_psd(&doc);
    let (imported, warnings) = psd_to_document(&exported);
    assert!(warnings.is_empty(), "Warnings: {warnings:?}");
    assert_eq!(imported.layers.len(), 1);
    assert!(imported.timeline.is_some());
    let tl = imported.timeline.as_ref().unwrap();
    assert_eq!(tl.duration, 4);
    assert_eq!(tl.fps, 24.0);

    let l = &imported.layers[0];
    assert_eq!(l.name, "Clip 1");
    assert!(l.video.is_some());
    assert_eq!(l.video.as_ref().unwrap().frames.len(), 4);
}
