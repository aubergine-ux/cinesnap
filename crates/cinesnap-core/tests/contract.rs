//! The editor writes `project.json`; this crate reads it. The fixture is a
//! project that uses every editor feature, written by the frontend's own
//! code (`src/lib/contract.test.ts` keeps it in step).

use cinesnap_core::events::EventLog;
use cinesnap_core::motion::compute_motion;
use cinesnap_core::project::{ClipBody, Project, Shape, TransitionKind};

const FIXTURE: &str = include_str!("fixtures/editor-project.json");

fn load() -> Project {
    let mut p = Project::parse(FIXTURE).expect("the editor's project file parses");
    p.validate().expect("and is valid");
    p.sanitize();
    p
}

#[test]
fn reads_everything_the_editor_writes() {
    let p = load();
    let kinds: Vec<&str> = p
        .clips
        .iter()
        .map(|c| match c.body {
            ClipBody::Card(_) => "card",
            ClipBody::Recording(_) => "recording",
        })
        .collect();
    assert_eq!(kinds, ["card", "recording", "recording", "card"]);
    assert_eq!(p.clips[2].transition.kind, TransitionKind::Zoom);
    assert_eq!(p.clips[2].recording().unwrap().dir, "clips/clip1");
    let outro = p.clips[3].card().unwrap();
    assert_eq!(
        (outro.align.as_str(), outro.font.as_deref()),
        ("left", Some("PT Serif"))
    );
    assert!(outro.background.is_some());

    assert_eq!(p.annotations.len(), 6);
    let names = ["text", "arrow", "box", "highlight", "spotlight", "blur"];
    for (a, name) in p.annotations.iter().zip(names) {
        let got = match a.shape {
            Shape::Text { .. } => "text",
            Shape::Arrow { .. } => "arrow",
            Shape::Box { .. } => "box",
            Shape::Highlight { .. } => "highlight",
            Shape::Spotlight { .. } => "spotlight",
            Shape::Blur { .. } => "blur",
        };
        assert_eq!(got, name);
    }
    assert_eq!(p.webcam.shape, "circle", "the Pop style was applied last");
    assert_eq!((p.webcam.x, p.webcam.y), (Some(0.12), Some(0.2)));
    assert_eq!(p.webcam_keys.len(), 2);
    assert!(!p.webcam_keys[1].show);
    assert_eq!(p.transcript.as_ref().unwrap().words.len(), 3);
    assert_eq!(p.theme.font, "Poppins");
    assert!(p.captions.uppercase && p.captions.background.is_none());
    let audio = &p.audio;
    assert!(audio.normalize && audio.eq.enabled);
    assert_eq!(audio.music.as_ref().unwrap().source, "builtin:open-sky");
    assert!(!audio.mic_mute && audio.system_mute);
    assert_eq!(audio.system_volume, 0.7);
    assert_eq!(p.export.preset, "shorts");

    let main = p.clips[1].recording().unwrap();
    let system = main.source.system.as_ref().unwrap();
    assert_eq!(
        (
            system.file.as_str(),
            system.offset_ms,
            system.app.as_deref()
        ),
        ("system.wav", -40.0, Some("Firefox"))
    );
    assert!(main.source.video.vfr);
    assert_eq!(main.source.video.hdr.as_deref(), Some("tonemapped"));
    let d = &p.frame.device;
    assert_eq!(
        (d.kind.as_str(), d.title.as_str(), d.dark),
        ("browser", "cinesnap.dev/docs", false)
    );
    assert!(p.auto_zoom.follow_text);
    assert!(p.reframe.enabled);
    let r = &p.reframe.segments[0];
    assert_eq!(
        (r.start_ms, r.end_ms, r.x, r.y),
        (5000.0, 8000.0, 640.0, 360.0)
    );
    assert!(p.motion_blur.enabled && p.motion_blur.strength == 0.75);
    let c = &p.cursor;
    assert_eq!(
        (c.style.as_str(), c.image.as_deref()),
        ("image", Some("cursors/pointer.png"))
    );
    assert_eq!((c.hotspot_x, c.hotspot_y), (0.2, 0.1));
    assert_eq!(
        (
            c.highlight.as_str(),
            c.highlight_color.as_str(),
            c.highlight_size
        ),
        ("ring", "#00ff88", 40.0)
    );
    let markers: Vec<(f64, &str)> = p
        .markers
        .iter()
        .map(|m| (m.at_ms, m.label.as_str()))
        .collect();
    assert_eq!(markers, [(4200.0, "Install"), (11000.0, "")]);
}

/// Everything in `theirs` is in `ours`, with the same value.
fn contains(ours: &serde_json::Value, theirs: &serde_json::Value, path: &str) {
    use serde_json::Value;
    match (ours, theirs) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                let mine = a.get(k).unwrap_or_else(|| panic!("{path}.{k} was dropped"));
                contains(mine, v, &format!("{path}.{k}"));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}");
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                contains(x, y, &format!("{path}[{i}]"));
            }
        }
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            assert!((a - b).abs() < 1e-9, "{path}: {a} vs {b}")
        }
        (a, b) => assert_eq!(a, b, "{path}"),
    }
}

#[test]
fn nothing_is_lost_on_the_way_back() {
    // What the core writes must contain everything the editor wrote, so a
    // save from the app never drops an edit.
    let theirs: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    let ours = serde_json::to_value(load()).unwrap();
    contains(&ours, &theirs, "project");
}

#[test]
fn motion_covers_the_whole_timeline() {
    let p = load();
    let logs: Vec<EventLog> = p
        .clips
        .iter()
        .map(|_| EventLog::empty(1920, 1080))
        .collect();
    let m = compute_motion(&p, &logs, &[]);
    // 3 s card + 24 s + 8 s (the last second cut) + 3 s card, at 30 fps.
    assert_eq!(m.fps, 30);
    assert_eq!(m.frames, 37 * 30);
    assert_eq!(m.clip[0], 0);
    assert_eq!(*m.clip.last().unwrap(), 3);
    assert_eq!(m.clip_starts, vec![0.0, 3000.0, 27_000.0, 35_000.0]);
    // A plain cut into the first recording, a zoom into the second, a
    // crossfade into the outro.
    let kinds: Vec<_> = m.transitions.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, [TransitionKind::Zoom, TransitionKind::Crossfade]);
    assert_eq!(m.transitions[0].frame, 27 * 30);
    assert_eq!(m.transitions[0].half, 9);
    // The webcam is hidden on cards, visible in a recording, and gone after
    // the keyframe that hides it (12 s into the first recording).
    assert_eq!(m.webcam_at(10).3, 0.0);
    assert_eq!(m.webcam_at(5 * 30).3, 1.0);
    assert_eq!(m.webcam_at(20 * 30).3, 0.0);
    // Captions come out in output time, one line here.
    assert_eq!(m.captions.len(), 1);
    assert_eq!(m.captions[0].words.len(), 3);
    assert_eq!(m.captions[0].start_ms, 3500.0);
}
