//! Window › Timeline: a Photoshop-style video timeline panel supporting rotoscoping,
//! onion skinning, frame-by-frame animation, playhead scrubbing, and track layout.

use egui::{Color32, CornerRadius, Rect, RichText, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

fn default_true() -> bool {
    true
}

fn default_dock_height() -> f32 {
    180.0
}

fn default_zoom() -> f32 {
    16.0
}

/// Rotoscoping Onion Skinning settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OnionSkinSettings {
    pub enabled: bool,
    pub frames_before: usize,
    pub frames_after: usize,
    pub frame_spacing: usize,
    pub max_opacity: f32,
    pub min_opacity: f32,
    pub tint_colors: bool,
}

impl Default for OnionSkinSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            frames_before: 1,
            frames_after: 1,
            frame_spacing: 1,
            max_opacity: 0.5,
            min_opacity: 0.2,
            tint_colors: true,
        }
    }
}

/// Interactive stretching state when dragging the trim handle at the end of a clip sequence.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StretchDrag {
    pub layer_id: Option<u64>,
    pub start_frames: usize,
    pub target_frames: usize,
}

/// Timeline panel view state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimelineUi {
    pub open: bool,
    /// Docked at bottom of window (Photoshop Motion layout) vs floating.
    #[serde(default = "default_true")]
    pub docked: bool,
    /// Dock height in points.
    #[serde(default = "default_dock_height")]
    pub dock_height: f32,
    /// Playing back (auto-advancing the playhead at the frame rate).
    #[serde(default)]
    pub playing: bool,
    /// Loop playback.
    #[serde(default = "default_true")]
    pub looping: bool,
    /// Mute audio.
    #[serde(default)]
    pub mute_audio: bool,
    /// egui time (seconds) of the last auto-advance.
    #[serde(default, skip)]
    pub last_time: f64,
    /// Onion skin settings for rotoscoping.
    #[serde(default)]
    pub onion_skin: OnionSkinSettings,
    /// Onion skin settings dialog open.
    #[serde(default)]
    pub onion_skin_dialog: bool,
    /// Frame rate modal open.
    #[serde(default)]
    pub frame_rate_modal: bool,
    /// Custom FPS string input.
    #[serde(default)]
    pub custom_fps_input: String,
    /// Zoom level: points per frame.
    #[serde(default = "default_zoom")]
    pub zoom: f32,
    /// Horizontal scroll offset.
    #[serde(default)]
    pub scroll_x: f32,
    /// Active interactive stretching of a clip sequence on the timeline.
    #[serde(default, skip)]
    pub stretching: Option<StretchDrag>,
}

impl Default for TimelineUi {
    fn default() -> Self {
        Self {
            open: false,
            docked: true,
            dock_height: 180.0,
            playing: false,
            looping: true,
            mute_audio: false,
            last_time: 0.0,
            onion_skin: OnionSkinSettings::default(),
            onion_skin_dialog: false,
            frame_rate_modal: false,
            custom_fps_input: "24.0".to_string(),
            zoom: 16.0,
            scroll_x: 0.0,
            stretching: None,
        }
    }
}

/// Menu checkmark for Window › Timeline.
pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    (id == "window.panel.timeline").then_some(app.ui.timeline.open)
}

/// Ids this module owns (for parity / enablement).
pub fn handles(id: &str) -> bool {
    id == "window.panel.timeline"
}

/// Toggle the Timeline panel.
pub fn menu(app: &mut PhotocraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    if id == "window.panel.timeline" {
        app.ui.timeline.open = !app.ui.timeline.open;
        return Some(Ok(json!({ "open": app.ui.timeline.open })));
    }
    None
}

/// Render the bottom docked timeline panel (when open and docked).
pub fn dock_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if !app.ui.timeline.open || !app.ui.timeline.docked {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let mut h = app.ui.timeline.dock_height;
    egui::Panel::bottom("timeline_dock")
        .resizable(true)
        .default_size(h)
        .size_range(130.0..=600.0)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::ZERO))
        .show(ui, |ui| {
            h = ui.max_rect().height();
            timeline_content(app, ui, true);
        });
    app.ui.timeline.dock_height = h;
}

/// Render floating windows for the timeline (when undocked) and onion skin dialog.
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // 1. Floating Timeline Window (when open and undocked)
    if app.ui.timeline.open && !app.ui.timeline.docked {
        let t = Tokens::get(ctx);
        egui::Window::new(tl!("Timeline"))
            .id(egui::Id::new("timeline_float_window"))
            .default_size(vec2(760.0, 220.0))
            .min_size(vec2(400.0, 150.0))
            .resizable(true)
            .frame(egui::Frame::window(&ctx.global_style()).fill(t.chrome))
            .show(ctx, |ui| {
                timeline_content(app, ui, false);
            });
    }

    // 2. Onion Skin Settings Dialog Modal
    if app.ui.timeline.onion_skin_dialog {
        let mut close_dialog = false;
        let t = Tokens::get(ctx);
        egui::Window::new("Onion Skin Settings")
            .id(egui::Id::new("onion_skin_settings_modal"))
            .collapsible(false)
            .resizable(false)
            .default_size(vec2(320.0, 260.0))
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .frame(egui::Frame::window(&ctx.global_style()).fill(t.card))
            .show(ctx, |ui| {
                let s = &mut app.ui.timeline.onion_skin;
                ui.add_space(4.0);
                ui.checkbox(&mut s.enabled, RichText::new("Enable Onion Skins").strong());
                ui.separator();
                ui.add_space(4.0);

                egui::Grid::new("onion_skin_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Frames Before:");
                    ui.add(egui::Slider::new(&mut s.frames_before, 1..=10));
                    ui.end_row();

                    ui.label("Frames After:");
                    ui.add(egui::Slider::new(&mut s.frames_after, 1..=10));
                    ui.end_row();

                    ui.label("Frame Spacing:");
                    ui.add(egui::Slider::new(&mut s.frame_spacing, 1..=5));
                    ui.end_row();

                    ui.label("Max Opacity:");
                    let mut max_pct = (s.max_opacity * 100.0).round();
                    if ui.add(egui::Slider::new(&mut max_pct, 10.0..=100.0).suffix("%")).changed() {
                        s.max_opacity = (max_pct / 100.0).clamp(0.1, 1.0);
                    }
                    ui.end_row();

                    ui.label("Min Opacity:");
                    let mut min_pct = (s.min_opacity * 100.0).round();
                    if ui.add(egui::Slider::new(&mut min_pct, 5.0..=50.0).suffix("%")).changed() {
                        s.min_opacity = (min_pct / 100.0).clamp(0.05, 0.5);
                    }
                    ui.end_row();
                });

                ui.add_space(6.0);
                ui.checkbox(&mut s.tint_colors, "Tint Frames (Red for Past, Green for Future)");
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::widgets::primary_button(ui, tl!("OK"), 80.0).clicked() {
                        close_dialog = true;
                    }
                });
            });
        if close_dialog {
            app.ui.timeline.onion_skin_dialog = false;
        }
    }

    // 3. Playback Auto-Advance
    if app.ui.timeline.playing {
        if let Some(t) = app.session.active().and_then(|d| d.doc.timeline.clone()) {
            let now = ctx.input(|i| i.time);
            let period = 1.0 / f64::from(t.fps.max(1.0));
            if now - app.ui.timeline.last_time >= period {
                app.ui.timeline.last_time = now;
                let (ws, we) = (t.work_start, t.work_end.max(t.work_start + 1));
                let next = if t.current + 1 < we {
                    t.current + 1
                } else if app.ui.timeline.looping {
                    ws
                } else {
                    app.ui.timeline.playing = false;
                    t.current
                };
                if app.ui.timeline.playing {
                    let _ = app.run("timeline.setFrame", json!({ "frame": next as u64 }));
                }
            }
            ctx.request_repaint();
        } else {
            app.ui.timeline.playing = false;
        }
    }
}

/// Shared rendering for dock panel or floating window.
fn timeline_content(app: &mut PhotocraftApp, ui: &mut egui::Ui, is_docked: bool) {
    let t = Tokens::get(ui.ctx());
    let have_doc = app.session.active().is_some();
    let tl = app.session.active().and_then(|d| d.doc.timeline.clone());

    let mut act: Option<(&str, Value)> = None;
    let mut close = false;
    let mut toggle_play = false;
    let playing = app.ui.timeline.playing;

    // Top separator / border line
    let r = ui.max_rect();
    ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, t.separator));

    // 1. Header Bar: "Timeline" Tab + Controls/Menu
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;

        // Tab item look
        let tab_resp = ui.scope(|ui| {
            ui.visuals_mut().widgets.inactive.bg_fill = Color32::TRANSPARENT;
            ui.add(
                egui::Button::new(
                    RichText::new(tl!("Timeline"))
                        .size(13.0)
                        .strong()
                        .color(t.text),
                )
                .frame(false),
            )
        });
        // Active indicator line under tab
        let tab_r = tab_resp.response.rect;
        ui.painter().line_segment(
            [pos2(tab_r.left(), tab_r.bottom() + 1.0), pos2(tab_r.right(), tab_r.bottom() + 1.0)],
            Stroke::new(2.0, t.accent),
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Close button
            if crate::icons::button(ui, "x", 18.0, false, tl!("Close")).clicked() {
                close = true;
            }

            // Dock / undock toggle
            let dock_tip = if is_docked { "Undock to Floating Window" } else { "Dock to Bottom" };
            let dock_icon = if is_docked { "app-window" } else { "panel-right" };
            if crate::icons::button(ui, dock_icon, 18.0, false, dock_tip).clicked() {
                app.ui.timeline.docked = !app.ui.timeline.docked;
            }

            // Hamburger menu for timeline actions
            ui.menu_button("☰", |ui| {
                ui.set_min_width(200.0);
                if ui.checkbox(&mut app.ui.timeline.onion_skin.enabled, "Enable Onion Skins").clicked() {
                    ui.close();
                }
                if ui.button("Onion Skin Settings…").clicked() {
                    app.ui.timeline.onion_skin_dialog = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("Set Timeline Frame Rate…").clicked() {
                    app.ui.timeline.frame_rate_modal = true;
                    ui.close();
                }
                ui.menu_button("Timeline Frame Rate Presets", |ui| {
                    for fps in [12.0, 23.976, 24.0, 25.0, 29.97, 30.0, 60.0] {
                        if ui.button(format!("{fps} fps")).clicked() {
                            act = Some(("timeline.setFrameRate", json!({ "fps": fps })));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                if ui.button(tl!("New Blank Video Layer")).clicked() {
                    act = Some(("layer.videoLayers.newBlankVideoLayer", json!({})));
                    ui.close();
                }
                if ui.button(tl!("Insert Blank Frame")).clicked() {
                    act = Some(("layer.videoLayers.insertBlankFrame", json!({})));
                    ui.close();
                }
                if ui.button(tl!("Duplicate Frame")).clicked() {
                    act = Some(("layer.videoLayers.duplicateFrame", json!({})));
                    ui.close();
                }
                if ui.button(tl!("Delete Frame")).clicked() {
                    act = Some(("layer.videoLayers.deleteFrame", json!({})));
                    ui.close();
                }
                if ui.button("Split at Playhead").clicked() {
                    act = Some(("timeline.splitAtPlayhead", json!({})));
                    ui.close();
                }
                ui.separator();
                if tl.is_some() {
                    if ui.button(tl!("Delete Timeline")).clicked() {
                        act = Some(("timeline.delete", json!({})));
                        ui.close();
                    }
                } else if ui.button(tl!("Create Video Timeline")).clicked() {
                    act = Some(("timeline.create", json!({ "duration": 30, "fps": 30.0 })));
                    ui.close();
                }
            });
        });
    });

    ui.separator();

    if !have_doc {
        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            ui.weak(tl!("Open a document to use the timeline."));
        });
        if close {
            app.ui.timeline.open = false;
            app.ui.timeline.playing = false;
        }
        return;
    }

    let Some(tl) = tl else {
        // Photoshop empty timeline state: big center "Create Video Timeline" button
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            if crate::widgets::primary_button(ui, tl!("Create Video Timeline"), 220.0).clicked() {
                act = Some(("timeline.create", json!({ "duration": 30, "fps": 30.0 })));
            }
            ui.add_space(6.0);
            ui.weak("30 frames @ 30.00 fps");
        });
        if close {
            app.ui.timeline.open = false;
            app.ui.timeline.playing = false;
        }
        if let Some((cmd, p)) = act {
            let _ = app.run(cmd, p);
        }
        return;
    };

    // Keyboard navigation within the timeline panel
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)) {
        act = Some(("timeline.nextFrame", json!({})));
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)) {
        act = Some(("timeline.previousFrame", json!({})));
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowRight)) {
        act = Some(("timeline.nextFrame", json!({"count": 10})));
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowLeft)) {
        act = Some(("timeline.previousFrame", json!({"count": 10})));
    }

    // 2. Playback & Tools Toolbar Row
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;

        // Playback buttons
        if crate::icons::button(ui, "chevrons-left", 20.0, false, "First Frame").clicked() {
            act = Some(("timeline.firstFrame", json!({})));
        }
        let prev_tip = format!("{} (←)", "Previous Frame");
        if crate::icons::button(ui, "chevron-left", 20.0, false, &prev_tip).clicked() {
            act = Some(("timeline.previousFrame", json!({})));
        }
        let play_icon = if playing { "pause" } else { "play" };
        if crate::icons::button(ui, play_icon, 22.0, playing, if playing { tl!("Pause") } else { tl!("Play") }).clicked() {
            toggle_play = true;
        }
        let next_tip = format!("{} (→)", "Next Frame");
        if crate::icons::button(ui, "chevron-right", 20.0, false, &next_tip).clicked() {
            act = Some(("timeline.nextFrame", json!({})));
        }
        if crate::icons::button(ui, "chevrons-right", 20.0, false, "Last Frame").clicked() {
            act = Some(("timeline.lastFrame", json!({})));
        }

        // Loop playback button
        if crate::icons::button(ui, "rotate-cw", 20.0, app.ui.timeline.looping, "Loop Playback").clicked() {
            app.ui.timeline.looping = !app.ui.timeline.looping;
        }

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        // Timecode & Frame info
        let tc = format_timecode(tl.current, tl.fps);
        ui.label(RichText::new(tc).monospace().strong().size(12.0).color(t.text));
        if ui
            .button(RichText::new(format!("({:.2} fps ▾)", tl.fps)).monospace().size(11.0).color(t.accent_text))
            .on_hover_text("Change Timeline Frame Rate…")
            .clicked()
        {
            app.ui.timeline.custom_fps_input = format!("{:.2}", tl.fps);
            app.ui.timeline.frame_rate_modal = true;
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // Rotoscoping / Animation tools
        let onion_on = app.ui.timeline.onion_skin.enabled;
        let onion_btn = egui::Button::new(
            RichText::new(if onion_on { "🧅 Onion Skins: ON" } else { "🧅 Onion Skins" })
                .size(11.5)
                .color(if onion_on { t.accent_text } else { t.text }),
        )
        .fill(if onion_on { t.accent } else { Color32::TRANSPARENT });
        if ui.add(onion_btn).on_hover_text("Toggle Onion Skinning for rotoscoping").clicked() {
            app.ui.timeline.onion_skin.enabled = !app.ui.timeline.onion_skin.enabled;
        }

        if crate::icons::button(ui, "settings", 20.0, app.ui.timeline.onion_skin_dialog, "Onion Skin Settings…").clicked() {
            app.ui.timeline.onion_skin_dialog = !app.ui.timeline.onion_skin_dialog;
        }

        // Scissor tool (Split at Playhead)
        if crate::icons::button(ui, "scissors", 20.0, false, "Split at Playhead (cut frame)").clicked() {
            act = Some(("timeline.splitAtPlayhead", json!({})));
        }

        // Quick Insert Blank Frame
        if ui.button(RichText::new("+ Blank").size(11.0)).on_hover_text("Insert Blank Video Frame").clicked() {
            act = Some(("layer.videoLayers.insertBlankFrame", json!({})));
        }

        // Right side: zoom slider
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Slider::new(&mut app.ui.timeline.zoom, 6.0..=50.0).show_value(false).text("Zoom"));
            ui.label(RichText::new(format!("{}f", tl.duration)).color(t.text_dim).monospace().size(11.0));
        });
    });

    ui.separator();

    // 3. Tracks & Ruler View
    let available_w = ui.available_width();
    let header_w = 170.0;
    let track_content_w = (available_w - header_w).max(100.0);

    let zoom = app.ui.timeline.zoom.clamp(6.0, 50.0);
    let stretch_frames = app.ui.timeline.stretching.as_ref().map_or(0, |s| s.target_frames);
    let max_duration = tl.duration.max(stretch_frames);
    let total_timeline_w = (max_duration as f32 * zoom).max(track_content_w);

    let doc_ref = app.session.active().map(|d| &d.doc);
    let active_id = app.session.active().and_then(|d| d.active_layer);
    let display_rows = doc_ref.map_or_else(Vec::new, |d| crate::layer_tree_ui::display_rows(d, true));

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.horizontal(|ui| {
            // Left Column: Track Headers
            ui.allocate_ui_with_layout(vec2(header_w, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(header_w);
                let hr = ui.max_rect();
                ui.painter().rect_filled(hr, 0.0, t.chrome);

                // Header spacer matching ruler height
                ui.allocate_exact_size(vec2(header_w, 24.0), Sense::hover());
                ui.separator();

                if display_rows.is_empty() {
                    let (v_row, _) = ui.allocate_exact_size(vec2(header_w, 32.0), Sense::click());
                    ui.painter().rect_filled(v_row, 0.0, Color32::from_black_alpha(20));
                    let mut icon_ui = ui.new_child(egui::UiBuilder::new().max_rect(v_row));
                    icon_ui.horizontal_centered(|ui| {
                        ui.add_space(4.0);
                        ui.label(RichText::new("▤").size(12.0));
                        ui.label(RichText::new("Layer 1").strong().size(11.0).color(t.text));
                    });
                    ui.separator();
                } else {
                    for &(depth, l) in &display_rows {
                        let is_active = active_id == Some(l.id);
                        let is_video = l.video.is_some();
                        let (v_row, row_resp) = ui.allocate_exact_size(vec2(header_w, 32.0), Sense::click());
                        let bg_color = if is_active {
                            Color32::from_rgb(45, 65, 95)
                        } else {
                            Color32::from_black_alpha(20)
                        };
                        ui.painter().rect_filled(v_row, 0.0, bg_color);
                        let mut icon_ui = ui.new_child(egui::UiBuilder::new().max_rect(v_row));
                        icon_ui.horizontal_centered(|ui| {
                            ui.add_space(4.0 + (depth as f32 * 8.0).min(40.0));
                            let eye = if l.visible { "👁" } else { " " };
                            if ui.button(RichText::new(eye).size(10.0)).on_hover_text("Toggle layer visibility").clicked() {
                                act = Some(("layer.setVisibility", json!({ "layer": l.id.0, "visible": !l.visible })));
                            }
                            let icon_text = if is_video { "🎞" } else if l.is_group() { "📁" } else { "▤" };
                            ui.label(RichText::new(icon_text).size(12.0));
                            let name_color = if is_active { t.accent_text } else { t.text };
                            ui.label(RichText::new(&l.name).strong().size(11.0).color(name_color));
                        });
                        if row_resp.clicked() {
                            act = Some(("layer.select", json!({ "layer": l.id.0 })));
                        }
                        ui.separator();
                    }
                }

                // Audio Track Header Row
                let (a_row, _) = ui.allocate_exact_size(vec2(header_w, 28.0), Sense::hover());
                ui.painter().rect_filled(a_row, 0.0, Color32::from_black_alpha(10));
                let mut audio_ui = ui.new_child(egui::UiBuilder::new().max_rect(a_row));
                audio_ui.horizontal_centered(|ui| {
                    ui.add_space(14.0);
                    ui.label(RichText::new("🎵").size(11.0));
                    ui.label(RichText::new("Audio Track").size(11.0).color(t.text_dim));
                });
            });

            // Vertical divider line between header and tracks
            let sep_r = Rect::from_min_size(ui.cursor().min, vec2(1.0, ui.available_height()));
            ui.painter().line_segment([sep_r.left_top(), sep_r.left_bottom()], Stroke::new(1.0, t.separator));

            // Right Column: Scrollable Ruler and Track Lanes
            egui::ScrollArea::horizontal()
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    ui.set_min_width(total_timeline_w);
                    let origin = ui.cursor().min;

                    // --- Ruler Row (height 24.0) ---
                    let ruler_rect = Rect::from_min_size(origin, vec2(total_timeline_w, 24.0));
                    let ruler_resp = ui.interact(ruler_rect, egui::Id::new("timeline_ruler_scrub"), Sense::click_and_drag());
                    ui.painter().rect_filled(ruler_rect, 0.0, Color32::from_black_alpha(35));

                    // Ruler Ticks and Labels
                    let step_frames = if zoom < 10.0 { 30 } else if zoom < 20.0 { 10 } else { 5 };
                    for f in 0..=max_duration {
                        let x = origin.x + f as f32 * zoom;
                        let is_sec = tl.fps > 0.0 && f % (tl.fps.round() as usize) == 0;
                        let is_major = f % step_frames == 0;

                        let tick_len = if is_sec { 12.0 } else if is_major { 8.0 } else { 4.0 };
                        let tick_color = if is_sec { t.text } else if is_major { t.text_dim } else { Color32::from_white_alpha(30) };
                        ui.painter().line_segment(
                            [pos2(x, ruler_rect.bottom() - tick_len), pos2(x, ruler_rect.bottom())],
                            Stroke::new(1.0, tick_color),
                        );

                        if is_major || is_sec {
                            let label = if is_sec && f > 0 {
                                format!("{:02}:00f", (f as f32 / tl.fps).floor() as usize)
                            } else {
                                format!("{f}f")
                            };
                            ui.painter().text(
                                pos2(x + 2.0, ruler_rect.top() + 3.0),
                                egui::Align2::LEFT_TOP,
                                label,
                                egui::FontId::monospace(9.0),
                                t.text_dim,
                            );
                        }
                    }

                    // Work area bracket display
                    let ws_x = origin.x + tl.work_start as f32 * zoom;
                    let we_x = origin.x + tl.work_end as f32 * zoom;
                    ui.painter().line_segment([pos2(ws_x, ruler_rect.top() + 1.0), pos2(we_x, ruler_rect.top() + 1.0)], Stroke::new(2.0, Color32::from_rgb(180, 180, 200)));
                    ui.painter().rect_filled(Rect::from_min_size(pos2(ws_x, ruler_rect.top()), vec2(3.0, 7.0)), 0.0, Color32::WHITE);
                    ui.painter().rect_filled(Rect::from_min_size(pos2(we_x - 3.0, ruler_rect.top()), vec2(3.0, 7.0)), 0.0, Color32::WHITE);

                    ui.advance_cursor_after_rect(ruler_rect);

                    let mut cur_lane_y = origin.y + 24.0;
                    let mut any_track_scrub = false;

                    // --- Layer Track Lanes ---
                    if display_rows.is_empty() {
                        let layer_frames = tl.duration;
                        let effective_frames = if let Some(s) = &app.ui.timeline.stretching && s.layer_id.is_none() {
                            s.target_frames
                        } else {
                            layer_frames
                        };
                        let clip_w = (effective_frames as f32 * zoom).max(10.0);
                        let track_rect = Rect::from_min_size(pos2(origin.x, cur_lane_y), vec2(total_timeline_w, 32.0));
                        let track_resp = ui.interact(track_rect, egui::Id::new("timeline_track_scrub_default"), Sense::click_and_drag());
                        ui.painter().rect_filled(track_rect, 0.0, Color32::from_black_alpha(15));

                        let clip_rect = Rect::from_min_size(pos2(origin.x, cur_lane_y + 3.0), vec2(clip_w, 26.0));
                        ui.painter().rect_filled(clip_rect, CornerRadius::same(3), Color32::from_rgb(125, 80, 130));
                        ui.painter().rect_stroke(clip_rect, CornerRadius::same(3), Stroke::new(1.0, Color32::from_rgb(155, 110, 160)), egui::StrokeKind::Inside);
                        ui.painter().text(pos2(clip_rect.left() + 8.0, clip_rect.center().y), egui::Align2::LEFT_CENTER, "Layer 1", egui::FontId::proportional(11.0), Color32::WHITE);

                        // Left Trim Handle
                        ui.painter().rect_filled(Rect::from_min_size(clip_rect.left_top(), vec2(3.0, clip_rect.height())), CornerRadius::same(2), Color32::from_white_alpha(150));

                        // Right Trim Handle (Interactive Stretching)
                        let handle_w = 12.0;
                        let handle_rect = Rect::from_min_size(pos2(clip_rect.right() - handle_w * 0.5, clip_rect.top()), vec2(handle_w, clip_rect.height()));
                        let handle_resp = ui.interact(handle_rect, egui::Id::new("timeline_trim_handle_right_default"), Sense::click_and_drag());
                        let is_handle_hovered = handle_resp.hovered() || handle_resp.dragged();
                        if is_handle_hovered {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast);
                        }
                        if handle_resp.drag_started() {
                            app.ui.timeline.stretching = Some(StretchDrag {
                                layer_id: None,
                                start_frames: layer_frames,
                                target_frames: layer_frames,
                            });
                        }
                        if let Some(stretch) = &mut app.ui.timeline.stretching {
                            if stretch.layer_id.is_none() {
                                if handle_resp.dragged() {
                                    if let Some(ptr) = ui.input(|i| i.pointer.interact_pos()) {
                                        let rel_x = ptr.x - origin.x;
                                        let frames = (rel_x / zoom).round().max(1.0) as usize;
                                        stretch.target_frames = frames.min(100_000);
                                        ui.ctx().request_repaint();
                                    }
                                }
                                if handle_resp.drag_stopped() {
                                    let target = stretch.target_frames;
                                    act = Some(("timeline.setProps", json!({ "duration": target, "workEnd": target })));
                                    app.ui.timeline.stretching = None;
                                    ui.ctx().request_repaint();
                                }
                            }
                        }

                        let is_actively_stretching = app.ui.timeline.stretching.as_ref().is_some_and(|s| s.layer_id.is_none());
                        let handle_bar_rect = Rect::from_min_size(pos2(clip_rect.right() - 4.0, clip_rect.top()), vec2(4.0, clip_rect.height()));
                        let handle_color = if is_actively_stretching {
                            Color32::from_rgb(255, 200, 60)
                        } else if is_handle_hovered {
                            Color32::WHITE
                        } else {
                            Color32::from_white_alpha(180)
                        };
                        ui.painter().rect_filled(handle_bar_rect, CornerRadius::same(2), handle_color);
                        if is_actively_stretching || is_handle_hovered {
                            let mid_y = clip_rect.center().y;
                            ui.painter().line_segment([pos2(clip_rect.right() - 2.0, mid_y - 4.0), pos2(clip_rect.right() - 2.0, mid_y + 4.0)], Stroke::new(1.0, Color32::from_black_alpha(120)));
                        }
                        if let Some(stretch) = &app.ui.timeline.stretching {
                            if stretch.layer_id.is_none() {
                                let diff = stretch.target_frames as i64 - stretch.start_frames as i64;
                                let badge_text = if diff >= 0 {
                                    format!("{}f (+{}f)", stretch.target_frames, diff)
                                } else {
                                    format!("{}f ({}f)", stretch.target_frames, diff)
                                };
                                let badge_pos = pos2(clip_rect.right(), clip_rect.top() - 14.0);
                                let galley = ui.painter().layout_no_wrap(badge_text, egui::FontId::monospace(10.0), Color32::WHITE);
                                let badge_rect = Rect::from_center_size(badge_pos, galley.size() + vec2(8.0, 4.0));
                                ui.painter().rect_filled(badge_rect, CornerRadius::same(3), Color32::from_black_alpha(220));
                                ui.painter().rect_stroke(badge_rect, CornerRadius::same(3), Stroke::new(1.0, Color32::from_rgb(255, 200, 60)), egui::StrokeKind::Inside);
                                ui.painter().galley(badge_rect.min + vec2(4.0, 2.0), galley, Color32::WHITE);
                            }
                        }

                        if (track_resp.clicked() || track_resp.dragged()) && !is_handle_hovered && !handle_resp.dragged() && app.ui.timeline.stretching.is_none() {
                            any_track_scrub = true;
                        }
                        cur_lane_y += 32.0;
                    } else {
                        for &(depth, l) in &display_rows {
                            let _ = depth;
                            let is_video = l.video.is_some();
                            let is_active = active_id == Some(l.id);
                            let layer_frames = l.video.as_ref().map_or(tl.duration, |v| v.frames.len());
                            let effective_frames = if let Some(s) = &app.ui.timeline.stretching && s.layer_id == Some(l.id.0) {
                                s.target_frames
                            } else {
                                layer_frames
                            };
                            let clip_w = (effective_frames as f32 * zoom).max(10.0);

                            let track_rect = Rect::from_min_size(pos2(origin.x, cur_lane_y), vec2(total_timeline_w, 32.0));
                            let track_resp = ui.interact(track_rect, egui::Id::new(("timeline_lane_scrub", l.id.0)), Sense::click_and_drag());

                            let lane_bg = if is_active { Color32::from_white_alpha(8) } else { Color32::from_black_alpha(15) };
                            ui.painter().rect_filled(track_rect, 0.0, lane_bg);

                            let clip_rect = Rect::from_min_size(pos2(origin.x, cur_lane_y + 3.0), vec2(clip_w, 26.0));

                            // Distinct color coding: Slate Blue/Indigo for Video layers, Mauve/Plum for non-video layers
                            let (clip_color, border_color) = if is_video {
                                (Color32::from_rgb(60, 105, 165), Color32::from_rgb(85, 135, 205))
                            } else {
                                (Color32::from_rgb(125, 80, 130), Color32::from_rgb(155, 110, 160))
                            };

                            ui.painter().rect_filled(clip_rect, CornerRadius::same(3), clip_color);
                            ui.painter().rect_stroke(clip_rect, CornerRadius::same(3), Stroke::new(1.0, border_color), egui::StrokeKind::Inside);

                            // Left Trim Handle
                            ui.painter().rect_filled(Rect::from_min_size(clip_rect.left_top(), vec2(3.0, clip_rect.height())), CornerRadius::same(2), Color32::from_white_alpha(150));

                            // Right Trim Handle (Interactive Stretching)
                            let handle_w = 12.0;
                            let handle_rect = Rect::from_min_size(pos2(clip_rect.right() - handle_w * 0.5, clip_rect.top()), vec2(handle_w, clip_rect.height()));
                            let handle_resp = ui.interact(handle_rect, egui::Id::new(("timeline_trim_handle_right", l.id.0)), Sense::click_and_drag());
                            let is_handle_hovered = handle_resp.hovered() || handle_resp.dragged();
                            if is_handle_hovered {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast);
                            }
                            if handle_resp.drag_started() {
                                app.ui.timeline.stretching = Some(StretchDrag {
                                    layer_id: Some(l.id.0),
                                    start_frames: layer_frames,
                                    target_frames: layer_frames,
                                });
                            }
                            if let Some(stretch) = &mut app.ui.timeline.stretching {
                                if stretch.layer_id == Some(l.id.0) {
                                    if handle_resp.dragged() {
                                        if let Some(ptr) = ui.input(|i| i.pointer.interact_pos()) {
                                            let rel_x = ptr.x - origin.x;
                                            let frames = (rel_x / zoom).round().max(1.0) as usize;
                                            stretch.target_frames = frames.min(100_000);
                                            ui.ctx().request_repaint();
                                        }
                                    }
                                    if handle_resp.drag_stopped() {
                                        let target = stretch.target_frames;
                                        if is_video {
                                            act = Some(("layer.videoLayers.setFrameCount", json!({ "layer": l.id.0, "count": target })));
                                        } else {
                                            act = Some(("timeline.setProps", json!({ "duration": target, "workEnd": target })));
                                        }
                                        app.ui.timeline.stretching = None;
                                        ui.ctx().request_repaint();
                                    }
                                }
                            }

                            let is_actively_stretching = app.ui.timeline.stretching.as_ref().is_some_and(|s| s.layer_id == Some(l.id.0));
                            let handle_bar_rect = Rect::from_min_size(pos2(clip_rect.right() - 4.0, clip_rect.top()), vec2(4.0, clip_rect.height()));
                            let handle_color = if is_actively_stretching {
                                Color32::from_rgb(255, 200, 60)
                            } else if is_handle_hovered {
                                Color32::WHITE
                            } else {
                                Color32::from_white_alpha(180)
                            };
                            ui.painter().rect_filled(handle_bar_rect, CornerRadius::same(2), handle_color);
                            if is_actively_stretching || is_handle_hovered {
                                let mid_y = clip_rect.center().y;
                                ui.painter().line_segment([pos2(clip_rect.right() - 2.0, mid_y - 4.0), pos2(clip_rect.right() - 2.0, mid_y + 4.0)], Stroke::new(1.0, Color32::from_black_alpha(120)));
                            }
                            if let Some(stretch) = &app.ui.timeline.stretching {
                                if stretch.layer_id == Some(l.id.0) {
                                    let diff = stretch.target_frames as i64 - stretch.start_frames as i64;
                                    let badge_text = if diff >= 0 {
                                        format!("{}f (+{}f)", stretch.target_frames, diff)
                                    } else {
                                        format!("{}f ({}f)", stretch.target_frames, diff)
                                    };
                                    let badge_pos = pos2(clip_rect.right(), clip_rect.top() - 14.0);
                                    let galley = ui.painter().layout_no_wrap(badge_text, egui::FontId::monospace(10.0), Color32::WHITE);
                                    let badge_rect = Rect::from_center_size(badge_pos, galley.size() + vec2(8.0, 4.0));
                                    ui.painter().rect_filled(badge_rect, CornerRadius::same(3), Color32::from_black_alpha(220));
                                    ui.painter().rect_stroke(badge_rect, CornerRadius::same(3), Stroke::new(1.0, Color32::from_rgb(255, 200, 60)), egui::StrokeKind::Inside);
                                    ui.painter().galley(badge_rect.min + vec2(4.0, 2.0), galley, Color32::WHITE);
                                }
                            }

                            // Frame dividers (video layers with zoom >= 14.0)
                            if is_video && zoom >= 14.0 {
                                for f in 1..effective_frames {
                                    let fx = origin.x + f as f32 * zoom;
                                    ui.painter().line_segment([pos2(fx, clip_rect.top() + 3.0), pos2(fx, clip_rect.bottom() - 3.0)], Stroke::new(1.0, Color32::from_black_alpha(40)));
                                }
                            }

                            // Thumbnail & Label on clip
                            let label = if is_video {
                                format!("{} ({} frames)", l.name, effective_frames)
                            } else {
                                l.name.clone()
                            };
                            ui.painter().text(
                                pos2(clip_rect.left() + 8.0, clip_rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                label,
                                egui::FontId::proportional(11.0),
                                Color32::WHITE,
                            );

                            if (track_resp.clicked() || track_resp.dragged()) && !is_handle_hovered && !handle_resp.dragged() && app.ui.timeline.stretching.is_none() {
                                any_track_scrub = true;
                            }
                            cur_lane_y += 32.0;
                        }
                    }

                    // --- Audio Track Lane Row (height 28.0) ---
                    let audio_rect = Rect::from_min_size(pos2(origin.x, cur_lane_y), vec2(total_timeline_w, 28.0));
                    ui.painter().rect_filled(audio_rect, 0.0, Color32::from_black_alpha(25));
                    let audio_clip_w = (tl.duration as f32 * zoom).max(10.0);
                    let audio_clip = Rect::from_min_size(pos2(origin.x, cur_lane_y + 3.0), vec2(audio_clip_w, 22.0));
                    ui.painter().rect_filled(audio_clip, CornerRadius::same(2), Color32::from_rgb(45, 75, 75));
                    ui.painter().text(
                        pos2(audio_clip.left() + 8.0, audio_clip.center().y),
                        egui::Align2::LEFT_CENTER,
                        "Audio",
                        egui::FontId::proportional(10.0),
                        Color32::from_white_alpha(180),
                    );

                    let bottom_y = cur_lane_y + 28.0;

                    // --- Playhead Indicator & Needle across all tracks ---
                    let cur_x = origin.x + tl.current as f32 * zoom;

                    // Vertical needle line across all lanes
                    ui.painter().line_segment(
                        [pos2(cur_x, ruler_rect.top() + 12.0), pos2(cur_x, bottom_y)],
                        Stroke::new(1.5, Color32::from_rgb(230, 45, 45)),
                    );

                    // Blue playhead triangle / head at ruler
                    let head_rect = Rect::from_center_size(pos2(cur_x, ruler_rect.top() + 6.0), vec2(10.0, 12.0));
                    let head_color = Color32::from_rgb(50, 130, 240);
                    ui.painter().rect_filled(head_rect, CornerRadius::same(2), head_color);
                    ui.painter().rect_stroke(head_rect, CornerRadius::same(2), Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);

                    // Scrubbing logic on click / drag
                    if (ruler_resp.clicked() || ruler_resp.dragged() || any_track_scrub) && app.ui.timeline.stretching.is_none() {
                        let ptr = ui.input(|i| i.pointer.interact_pos());
                        if let Some(pos) = ptr {
                            let rel_x = pos.x - origin.x;
                            let frame = (rel_x / zoom).round().clamp(0.0, (tl.duration.saturating_sub(1)) as f32) as u64;
                            act = Some(("timeline.setFrame", json!({ "frame": frame })));
                            ui.ctx().request_repaint();
                        }
                    }
                });
        });
    });

    // 4. Frame Rate Settings Modal
    if app.ui.timeline.frame_rate_modal {
        let mut modal_open = true;
        egui::Window::new("Timeline Frame Rate")
            .open(&mut modal_open)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.set_min_width(280.0);
                ui.label(RichText::new("Standard Frame Rate Presets:").strong());
                ui.add_space(4.0);
                for fps in [12.0, 23.976, 24.0, 25.0, 29.97, 30.0, 60.0] {
                    let desc = match fps as u32 {
                        12 => "12 fps (Stop Motion / 2s)",
                        23 => "23.976 fps (NTSC Film)",
                        24 => "24 fps (Cinema Film)",
                        25 => "25 fps (PAL / Europe)",
                        29 => "29.97 fps (NTSC Video)",
                        30 => "30 fps (Standard Web/Video)",
                        60 => "60 fps (High Frame Rate)",
                        _ => "",
                    };
                    let label = if desc.is_empty() { format!("{fps} fps") } else { desc.to_string() };
                    if ui.button(label).clicked() {
                        act = Some(("timeline.setFrameRate", json!({ "fps": fps })));
                        app.ui.timeline.frame_rate_modal = false;
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Custom FPS:");
                    ui.add(egui::TextEdit::singleline(&mut app.ui.timeline.custom_fps_input).desired_width(70.0));
                    if ui.button(tl!("Apply")).clicked() {
                        if let Ok(fps) = app.ui.timeline.custom_fps_input.trim().parse::<f64>() {
                            if fps >= 1.0 && fps <= 240.0 {
                                act = Some(("timeline.setFrameRate", json!({ "fps": fps })));
                                app.ui.timeline.frame_rate_modal = false;
                            }
                        }
                    }
                });
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tl!("Close")).clicked() {
                        app.ui.timeline.frame_rate_modal = false;
                    }
                });
            });
        if !modal_open {
            app.ui.timeline.frame_rate_modal = false;
        }
    }

    if close {
        app.ui.timeline.open = false;
        app.ui.timeline.playing = false;
    }
    if toggle_play {
        app.ui.timeline.playing = !app.ui.timeline.playing;
        app.ui.timeline.last_time = ui.ctx().input(|i| i.time);
    }
    if let Some((cmd, p)) = act {
        let _ = app.run(cmd, p);
    }
}

fn format_timecode(frame: usize, fps: f32) -> String {
    let fps = if fps > 0.0 { fps } else { 30.0 };
    let total_sec = frame as f32 / fps;
    let hours = (total_sec / 3600.0) as usize;
    let mins = ((total_sec % 3600.0) / 60.0) as usize;
    let secs = (total_sec % 60.0) as usize;
    let f = (frame as f32 % fps).round() as usize;
    format!("{hours:02};{mins:02};{secs:02};{f:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_timecode() {
        assert_eq!(format_timecode(0, 30.0), "00;00;00;00");
        assert_eq!(format_timecode(15, 30.0), "00;00;00;15");
        assert_eq!(format_timecode(30, 30.0), "00;00;01;00");
        assert_eq!(format_timecode(3600, 30.0), "00;02;00;00");
    }

    #[test]
    fn test_onion_skin_defaults() {
        let s = OnionSkinSettings::default();
        assert!(!s.enabled);
        assert_eq!(s.frames_before, 1);
        assert_eq!(s.frames_after, 1);
        assert_eq!(s.frame_spacing, 1);
        assert_eq!(s.max_opacity, 0.5);
        assert_eq!(s.min_opacity, 0.2);
        assert!(s.tint_colors);
    }

    #[test]
    fn test_timeline_ui_defaults() {
        let t = TimelineUi::default();
        assert!(!t.open);
        assert!(t.docked);
        assert_eq!(t.dock_height, 180.0);
        assert_eq!(t.zoom, 16.0);
        assert!(!t.playing);
    }
}

