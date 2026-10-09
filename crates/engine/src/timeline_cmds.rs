//! Window › Timeline: create and drive the document timeline (playhead over N frames at a frame
//! rate). Navigation doesn't add history steps. Video layers (Layer › Video Layers, added later)
//! read `current` to show their frame. Headless and scriptable so agents can render frame ranges.

use std::sync::Arc;

use photocraft_doc::Timeline;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document".into())
}

fn has_timeline(s: &Session) -> std::result::Result<(), String> {
    match s.active() {
        Some(d) if d.doc.timeline.is_some() => Ok(()),
        Some(_) => Err("no timeline (create one first)".into()),
        None => Err("no document".into()),
    }
}

/// Mutate the document timeline without a history step (navigation / metadata).
fn with_timeline(s: &mut Session, f: impl FnOnce(&mut Option<Timeline>)) -> Result<Value> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let mut doc = (*st.doc).clone();
    let old_cur = doc.timeline.as_ref().map_or(0, |t| t.current);
    crate::video_cmds::commit_frame(&mut doc, old_cur);
    f(&mut doc.timeline);
    if let Some(t) = &mut doc.timeline {
        t.clamp();
    }
    crate::video_cmds::sync(&mut doc);
    st.doc = Arc::new(doc);
    st.revision += 1;
    st.last_damage = None;
    info(s)
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let duration = p.get("duration").and_then(Value::as_u64).unwrap_or(30) as usize;
    let fps = p.get("fps").and_then(Value::as_f64).unwrap_or(30.0) as f32;
    with_timeline(s, |t| *t = Some(Timeline::new(duration, fps)))
}

fn set_frame_rate(s: &mut Session, p: &Value) -> Result<Value> {
    let fps = p.get("fps").and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams {
        cmd: "timeline.setFrameRate".into(),
        msg: "need `fps` (e.g. 24.0)".into(),
    })? as f32;
    if fps <= 0.0 || fps.is_nan() || fps > 240.0 {
        return Err(EngineError::BadParams {
            cmd: "timeline.setFrameRate".into(),
            msg: "frame rate must be between 1 and 240 fps".into(),
        });
    }
    with_timeline(s, |t| {
        if let Some(t) = t {
            t.fps = fps;
        }
    })
}

fn delete(s: &mut Session) -> Result<Value> {
    with_timeline(s, |t| *t = None)
}

fn set_frame(s: &mut Session, p: &Value) -> Result<Value> {
    let f =
        p.get("frame").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: "timeline.setFrame".into(), msg: "need `frame`".into() })? as usize;
    with_timeline(s, |t| {
        if let Some(t) = t {
            t.current = f;
        }
    })
}

fn step(s: &mut Session, delta: i64) -> Result<Value> {
    with_timeline(s, |t| {
        if let Some(t) = t {
            t.current = (t.current as i64 + delta).clamp(0, t.duration as i64 - 1) as usize;
        }
    })
}

fn first_frame(s: &mut Session) -> Result<Value> {
    with_timeline(s, |t| {
        if let Some(t) = t {
            t.current = 0;
        }
    })
}

fn last_frame(s: &mut Session) -> Result<Value> {
    with_timeline(s, |t| {
        if let Some(t) = t {
            t.current = t.duration.saturating_sub(1);
        }
    })
}

fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    with_timeline(s, |t| {
        if let Some(t) = t {
            if let Some(fps) = p.get("fps").and_then(Value::as_f64) {
                t.fps = fps as f32;
            }
            if let Some(d) = p.get("duration").and_then(Value::as_u64) {
                t.duration = d as usize;
            }
            if let Some(ws) = p.get("workStart").and_then(Value::as_u64) {
                t.work_start = ws as usize;
            }
            if let Some(we) = p.get("workEnd").and_then(Value::as_u64) {
                t.work_end = we as usize;
            }
        }
    })
}

fn info(s: &mut Session) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    Ok(match &st.doc.timeline {
        Some(t) => {
            json!({"timeline": {"fps": t.fps, "duration": t.duration, "current": t.current, "workStart": t.work_start, "workEnd": t.work_end, "time": t.time()}})
        }
        None => json!({"timeline": null}),
    })
}

macro_rules! spec {
    ($id:expr, $label:expr, $params:expr, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: $en, journal: false, run: $run }
    };
    ($id:expr, $label:expr, $sc:expr, $params:expr, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: $sc, params: $params, enabled: $en, journal: false, run: $run }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("timeline.create", "Create Video Timeline", r#"{duration?:30, fps?:30} → {timeline}"#, has_doc, |s, p| create(s, p)),
        spec!("timeline.delete", "Delete Timeline", "{} → {timeline:null}", has_timeline, |s, _| delete(s)),
        spec!("timeline.setFrame", "Go to Frame", r#"{frame} → {timeline}"#, has_timeline, |s, p| set_frame(s, p)),
        spec!("timeline.nextFrame", "Next Frame", Some("Right"), r#"{count?:1} → {timeline}"#, has_timeline, |s, p| {
            let count = p.get("count").and_then(|v| v.as_i64()).unwrap_or(1);
            step(s, count)
        }),
        spec!("timeline.previousFrame", "Previous Frame", Some("Left"), r#"{count?:1} → {timeline}"#, has_timeline, |s, p| {
            let count = p.get("count").and_then(|v| v.as_i64()).unwrap_or(1);
            step(s, -count)
        }),
        spec!("timeline.firstFrame", "First Frame", "{} → {timeline}", has_timeline, |s, _| first_frame(s)),
        spec!("timeline.lastFrame", "Last Frame", "{} → {timeline}", has_timeline, |s, _| last_frame(s)),
        spec!("timeline.splitAtPlayhead", "Split at Playhead", "{} → {timeline}", has_timeline, |s, p| {
            crate::video_cmds::split_at_playhead(s, p)?;
            info(s)
        }),
        spec!("timeline.trimFront", "Trim Clip Start", r#"{frames?: 1, layer?: id} → {timeline}"#, has_timeline, |s, p| {
            crate::video_cmds::trim_front(s, p)?;
            info(s)
        }),
        spec!("timeline.setProps", "Timeline Settings", r#"{fps?, duration?, workStart?, workEnd?} → {timeline}"#, has_timeline, |s, p| set_props(s, p)),
        spec!("timeline.setFrameRate", "Set Timeline Frame Rate", r#"{fps: 24.0} → {timeline}"#, has_timeline, |s, p| set_frame_rate(s, p)),
        spec!("timeline.info", "Timeline Info", "{} → {timeline}", has_doc, |s, _| info(s)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_navigate_delete() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"], Value::Null);
        let r = s.execute("timeline.create", json!({"duration": 24, "fps": 24.0})).unwrap();
        assert_eq!(r["timeline"]["duration"], 24);
        assert_eq!(r["timeline"]["current"], 0);
        // Navigate.
        s.execute("timeline.setFrame", json!({"frame": 10})).unwrap();
        s.execute("timeline.nextFrame", json!({})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"]["current"], 11);
        // Clamp beyond the end.
        let r = s.execute("timeline.setFrame", json!({"frame": 999})).unwrap();
        assert_eq!(r["timeline"]["current"], 23);
        // Props.
        s.execute("timeline.setProps", json!({"fps": 30.0})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"]["fps"], 30.0);
        // Delete.
        s.execute("timeline.delete", json!({})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"], Value::Null);
    }

    #[test]
    fn navigation_without_timeline_errors() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert!(s.execute("timeline.setFrame", json!({"frame": 1})).is_err());
    }

    #[test]
    fn first_last_frame_and_split() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("timeline.create", json!({"duration": 10, "fps": 24.0})).unwrap();
        s.execute("timeline.setFrame", json!({"frame": 5})).unwrap();
        s.execute("timeline.firstFrame", json!({})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"]["current"], 0);
        s.execute("timeline.lastFrame", json!({})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"]["current"], 9);

        // Split on blank video layer
        s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
        s.execute("timeline.setFrame", json!({"frame": 2})).unwrap();
        s.execute("timeline.splitAtPlayhead", json!({})).unwrap();
        let dur = s.execute("timeline.info", json!({})).unwrap()["timeline"]["duration"].as_u64().unwrap();
        assert!(dur >= 10);
    }

    #[test]
    fn rotoscope_frame_persistence() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("timeline.create", json!({"duration": 3, "fps": 30.0})).unwrap();
        s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();

        // Paint/fill on frame 0
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();

        // Move to frame 1 and fill with green
        s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();

        // Move to frame 2 and fill with blue
        s.execute("timeline.setFrame", json!({"frame": 2})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();

        // Navigate back to frame 0 and verify frame 0 retained red!
        s.execute("timeline.setFrame", json!({"frame": 0})).unwrap();
        let active_id = s.active().unwrap().active_layer.unwrap();
        let doc = &s.active().unwrap().doc;
        let l = doc.layer(active_id).unwrap();
        let v = l.video.as_ref().unwrap();
        assert_eq!(v.frames.len(), 3);

        let mut px = vec![[0u8; 4]; 1];
        v.frames[0].read_rgba8_into(photocraft_geom::Rect::from_xywh(0, 0, 1, 1), &mut px);
        assert_eq!(px[0][0], 255, "frame 0 red preserved");
        assert_eq!(px[0][1], 0, "frame 0 no green");

        v.frames[1].read_rgba8_into(photocraft_geom::Rect::from_xywh(0, 0, 1, 1), &mut px);
        assert_eq!(px[0][1], 255, "frame 1 green preserved");

        v.frames[2].read_rgba8_into(photocraft_geom::Rect::from_xywh(0, 0, 1, 1), &mut px);
        assert_eq!(px[0][2], 255, "frame 2 blue preserved");
    }

    #[test]
    fn step_count_navigation() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("timeline.create", json!({"duration": 30, "fps": 30.0})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"]["current"].as_u64().unwrap(), 0);

        // Step fwd 10 frames
        s.execute("timeline.nextFrame", json!({"count": 10})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"]["current"].as_u64().unwrap(), 10);

        // Step back 1 frame
        s.execute("timeline.previousFrame", json!({})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"]["current"].as_u64().unwrap(), 9);

        // Step back 10 frames (should clamp to 0)
        s.execute("timeline.previousFrame", json!({"count": 10})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"]["current"].as_u64().unwrap(), 0);
    }

    #[test]
    fn set_frame_rate_test() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("timeline.create", json!({"duration": 30, "fps": 30.0})).unwrap();

        // Change fps to 24
        s.execute("timeline.setFrameRate", json!({"fps": 24.0})).unwrap();
        let fps = s.execute("timeline.info", json!({})).unwrap()["timeline"]["fps"].as_f64().unwrap();
        assert_eq!(fps, 24.0);

        // Invalid fps returns Err, not panic
        assert!(s.execute("timeline.setFrameRate", json!({"fps": -5.0})).is_err());
        assert!(s.execute("timeline.setFrameRate", json!({"fps": 500.0})).is_err());
    }
}

