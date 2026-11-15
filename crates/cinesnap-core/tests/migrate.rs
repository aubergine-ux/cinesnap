//! Projects from earlier releases keep opening: manifests written by
//! v0.1.0 (format 1) and v0.2.0 (format 2, with edits), upgraded on load.

use cinesnap_core::project::{ClipBody, Project, PROJECT_VERSION};

const V1: &str = include_str!("fixtures/v1-project.json");
const V2: &str = include_str!("fixtures/v2-project.json");

fn open(text: &str) -> Project {
    let mut p = Project::parse(text).expect("parses");
    p.validate().expect("is valid");
    p.sanitize();
    p
}

#[test]
fn a_v1_project_is_one_recording_clip() {
    let raw: serde_json::Value = serde_json::from_str(V1).unwrap();
    assert_eq!(raw["version"], 1);
    assert!(raw.get("clips").is_none());

    let p = open(V1);
    assert_eq!(p.version, PROJECT_VERSION);
    assert_eq!(p.clips.len(), 1);
    let ClipBody::Recording(main) = &p.clips[0].body else {
        panic!("the clip is the recording");
    };
    assert_eq!(main.dir, "");
    assert_eq!(main.crop, p.crop);
    assert_eq!(
        main.source.video.duration_ms,
        raw["source"]["video"]["durationMs"].as_f64().unwrap()
    );
    assert_eq!(p.zooms.len(), raw["zooms"].as_array().unwrap().len());
    assert!(p.markers.is_empty());
}

#[test]
fn a_v2_project_keeps_its_edits() {
    let p = open(V2);
    assert_eq!(p.version, PROJECT_VERSION);
    let cuts: Vec<(f64, f64)> = p.cuts.iter().map(|c| (c.start_ms, c.end_ms)).collect();
    assert_eq!(cuts, [(500.0, 900.0)]);
    assert_eq!(p.annotations.len(), 4);
    assert_eq!(
        p.audio.music.as_ref().unwrap().source,
        "builtin:slow-coffee"
    );
    // Silenced by volume before mute switches existed, and still silent.
    assert_eq!(p.audio.mic_volume, 0.0);
    assert!(!p.audio.mic_mute && !p.audio.system_mute);
    assert_eq!(p.audio.system_volume, 1.0);
}

#[test]
fn saving_an_upgraded_project_and_opening_it_again_changes_nothing() {
    for text in [V1, V2] {
        let dir = tempfile::tempdir().unwrap();
        let first = open(text);
        first.save(dir.path()).unwrap();
        let again = Project::load(dir.path()).unwrap();
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&again).unwrap()
        );
    }
}
