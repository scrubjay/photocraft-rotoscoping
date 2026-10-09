//! File › Export › EffectCraft Composition: exports the document to PSD and generates an
//! EffectCraft project (.ecproj) with all multi-frame video and rotoscoped layers automatically
//! sequenced into animated precompositions.

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::process::Command;

#[cfg(any(test, not(target_arch = "wasm32")))]
use serde_json::json;
use serde_json::Value;

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn can_export(s: &Session) -> std::result::Result<(), String> {
    crate::file_cmds::native_doc(s)
}

/// Locate `effectcraft-cli` binary across system PATH, user directories, and standard install paths.
#[cfg(not(target_arch = "wasm32"))]
pub fn find_effectcraft_cli() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("EFFECTCRAFT_BIN") {
        let p = PathBuf::from(env_path);
        if p.exists() {
            return Some(p);
        }
    }

    if let Ok(status) = Command::new("effectcraft-cli")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        && status.success()
    {
        return Some(PathBuf::from("effectcraft-cli"));
    }

    #[cfg(windows)]
    {
        // Check Downloads directory for portable or extracted releases
        if let Ok(user_profile) = std::env::var("USERPROFILE") {
            let downloads = PathBuf::from(&user_profile).join("Downloads");
            if downloads.exists()
                && let Ok(entries) = std::fs::read_dir(&downloads) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() && path.to_string_lossy().to_ascii_lowercase().contains("effectcraft") {
                            let direct = path.join("effectcraft-cli.exe");
                            if direct.exists() {
                                return Some(direct);
                            }
                            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                                for sub in sub_entries.flatten() {
                                    let sub_direct = sub.path().join("effectcraft-cli.exe");
                                    if sub_direct.exists() {
                                        return Some(sub_direct);
                                    }
                                }
                            }
                        }
                    }
            }
        }

        // Check LocalAppData and ProgramFiles
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let p = PathBuf::from(local_app_data).join("Programs").join("EffectCraft").join("effectcraft-cli.exe");
            if p.exists() {
                return Some(p);
            }
        }
        if let Ok(prog_files) = std::env::var("ProgramFiles") {
            let p = PathBuf::from(prog_files).join("EffectCraft").join("effectcraft-cli.exe");
            if p.exists() {
                return Some(p);
            }
        }
    }

    // Check sibling of photocraft binary
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(parent) = exe_path.parent() {
            let candidate_exe = parent.join("effectcraft-cli.exe");
            if candidate_exe.exists() {
                return Some(candidate_exe);
            }
            let candidate = parent.join("effectcraft-cli");
            if candidate.exists() {
                return Some(candidate);
            }
    }

    None
}

#[cfg(target_arch = "wasm32")]
pub fn find_effectcraft_cli() -> Option<PathBuf> {
    None
}

/// File › Export › EffectCraft Composition…: generate .ecproj project with sequenced layers.
#[cfg(not(target_arch = "wasm32"))]
fn export_effectcraft(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = (*st.doc).clone();
    let name = doc.name.clone();
    let stem = name.rsplit_once('.').map_or(name.as_str(), |(a, _)| a).to_string();

    let out_path = if let Some(path_str) = p.get("path").and_then(Value::as_str) {
        if path_str.trim().is_empty() {
            return Err(EngineError::BadParams {
                cmd: "file.export.effectcraft".into(),
                msg: "empty `path`".into(),
            });
        }
        PathBuf::from(path_str)
    } else if let Some(dir_str) = p.get("dir").and_then(Value::as_str) {
        if dir_str.trim().is_empty() {
            return Err(EngineError::BadParams {
                cmd: "file.export.effectcraft".into(),
                msg: "empty `dir`".into(),
            });
        }
        PathBuf::from(dir_str).join(format!("{stem}.ecproj"))
    } else {
        return Err(EngineError::BadParams {
            cmd: "file.export.effectcraft".into(),
            msg: "need `path` or `dir`".into(),
        });
    };

    let ecproj_path = if out_path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("ecproj")) {
        out_path.with_extension("ecproj")
    } else {
        out_path
    };

    let parent_dir = ecproj_path.parent().unwrap_or_else(|| Path::new("."));
    if !parent_dir.as_os_str().is_empty() {
        std::fs::create_dir_all(parent_dir).map_err(|e| EngineError::Other(format!("create output directory: {e}")))?;
    }

    let psd_path = ecproj_path.with_extension("psd");

    // 1. Export document to companion PSD
    let opts = photocraft_io::ExportOptions::default();
    let psd_bytes = photocraft_io::export(&doc, "psd", &opts)
        .map(|r| r.bytes)
        .map_err(|e| EngineError::Other(format!("export PSD for EffectCraft: {e}")))?;
    crate::file_cmds::write_file(&psd_path.to_string_lossy(), &psd_bytes)?;

    // 2. Locate effectcraft-cli
    let cli = find_effectcraft_cli().ok_or_else(|| {
        EngineError::Other(
            "EffectCraft CLI (`effectcraft-cli`) could not be found. Please ensure EffectCraft is installed or set the `EFFECTCRAFT_BIN` environment variable."
                .to_string(),
        )
    })?;

    // 3. Import PSD as composition into .ecproj
    let import_params = json!({
        "paths": [psd_path.to_string_lossy()],
        "importAs": "composition",
    });

    let import_output = Command::new(&cli)
        .arg("--empty")
        .arg("exec")
        .arg("file.import")
        .arg("--params")
        .arg(import_params.to_string())
        .arg("--save-as")
        .arg(&ecproj_path)
        .output()
        .map_err(|e| EngineError::Other(format!("run effectcraft-cli import: {e}")))?;

    if !import_output.status.success() {
        let err = String::from_utf8_lossy(&import_output.stderr);
        let detail = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("import failed");
        return Err(EngineError::Other(format!("EffectCraft PSD import failed: {detail}")));
    }

    // 4. Create sequence script and run on .ecproj to ripple frames
    let target_fps = doc.timeline.as_ref().map_or(29.97, |t| t.fps as f64);
    let sequence_js = format!(
        r#"
var targetFps = {target_fps};
function getFrameNum(name) {{
    var m = name.match(/(\d+)/);
    return m ? parseInt(m[1], 10) : -1;
}}
var maxDuration = 0;
for (var i = 1; i <= app.project.items.length; i++) {{
    var item = app.project.item(i);
    if (item.typeName == "Composition") {{
        var comp = item;
        if (targetFps > 0) {{
            comp.frameRate = targetFps;
        }}
        var isFrameSeq = false;
        if (comp.numLayers > 1) {{
            var fn = comp.layer(1).name.toLowerCase();
            if (fn.indexOf("frame") == 0 || fn.indexOf("layer") == 0 || /^(\d+|[a-z]+[_\s]?\d+)/i.test(fn)) {{
                isFrameSeq = true;
            }}
        }}
        if (isFrameSeq) {{
            var fps = comp.frameRate;
            var frameDur = 1.0 / fps;
            var firstNum = getFrameNum(comp.layer(1).name);
            var lastNum = getFrameNum(comp.layer(comp.numLayers).name);
            var reversed = (firstNum > lastNum && lastNum >= 0);
            for (var j = 1; j <= comp.numLayers; j++) {{
                var l = comp.layer(j);
                var frameIdx = reversed ? (comp.numLayers - j) : (j - 1);
                l.startTime = frameIdx * frameDur;
                l.inPoint = frameIdx * frameDur;
                l.outPoint = (frameIdx + 1) * frameDur;
            }}
            comp.duration = comp.numLayers * frameDur;
            if (comp.duration > maxDuration) {{
                maxDuration = comp.duration;
            }}
        }}
    }}
}}
if (maxDuration > 0) {{
    for (var i = 1; i <= app.project.items.length; i++) {{
        var item = app.project.item(i);
        if (item.typeName == "Composition" && item.duration < maxDuration) {{
            item.duration = maxDuration;
        }}
    }}
}}
"#
    );

    let temp_script_path = std::env::temp_dir().join(format!(
        "photocraft_seq_{}.js",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));

    std::fs::write(&temp_script_path, sequence_js)
        .map_err(|e| EngineError::Other(format!("write sequence script: {e}")))?;

    let script_output = Command::new(&cli)
        .arg("--project")
        .arg(&ecproj_path)
        .arg("script")
        .arg(&temp_script_path)
        .arg("--save")
        .output();

    let _ = std::fs::remove_file(&temp_script_path);

    let Ok(script_output) = script_output else {
        return Err(EngineError::Other("failed to execute effectcraft-cli sequence script".into()));
    };

    if !script_output.status.success() {
        let err = String::from_utf8_lossy(&script_output.stderr);
        let detail = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("script failed");
        return Err(EngineError::Other(format!("EffectCraft layer sequencing failed: {detail}")));
    }

    Ok(json!({
        "project": ecproj_path.to_string_lossy(),
        "psd": psd_path.to_string_lossy(),
    }))
}

#[cfg(target_arch = "wasm32")]
fn export_effectcraft(_s: &mut Session, _p: &Value) -> Result<Value> {
    Err(EngineError::Other("EffectCraft export is not available in the browser".into()))
}

macro_rules! spec {
    ($id:expr, $label:expr, $menu:expr, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: "{} → {}", enabled: $en, journal: false, run: $run }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "file.export.effectcraft",
            "EffectCraft Composition…",
            &["File", "Export"],
            can_export,
            |s, p| export_effectcraft(s, p)
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_params_fails_gracefully() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        assert!(s.execute("file.export.effectcraft", json!({})).is_err());
        assert!(s.execute("file.export.effectcraft", json!({"path": ""})).is_err());
        assert!(s.execute("file.export.effectcraft", json!({"dir": ""})).is_err());
        assert!(s.execute("file.export.effectcraft", json!({"path": 42})).is_err());
    }

    #[test]
    fn find_cli_does_not_panic() {
        let _ = find_effectcraft_cli();
    }
}
