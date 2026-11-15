//! Upgrading manifests written by older versions.
//!
//! A manifest is upgraded as JSON, one version at a time, before it is read
//! into [`Project`](crate::project::Project). Steps only add or rewrite the
//! fields they are about, so anything else (including fields from a newer
//! version) survives.

use serde_json::{json, Map, Value};

use crate::project::PROJECT_VERSION;

type Step = fn(&mut Map<String, Value>);

/// `STEPS[i]` takes version `i + 1` to `i + 2`.
const STEPS: [Step; 2] = [v1_to_v2, v2_to_v3];

const _: () = assert!(STEPS.len() as u32 + 1 == PROJECT_VERSION);

/// Upgrade `manifest` in place to the current version. Returns the version
/// it was written with. Manifests from a newer version are left as they are.
pub fn migrate(manifest: &mut Value) -> u32 {
    let Some(obj) = manifest.as_object_mut() else {
        return 0;
    };
    let from = obj
        .get("version")
        .and_then(Value::as_u64)
        .map_or(1, |v| v.max(1) as u32);
    if from >= PROJECT_VERSION {
        return from;
    }
    for step in &STEPS[from as usize - 1..] {
        step(obj);
    }
    obj.insert("version".into(), json!(PROJECT_VERSION));
    from
}

/// Version 2 put everything on the timeline into `clips`. A version 1 file
/// is a single recording: the bundle's own.
fn v1_to_v2(p: &mut Map<String, Value>) {
    let has_recording = p
        .get("clips")
        .and_then(Value::as_array)
        .is_some_and(|c| c.iter().any(|c| c["kind"] == "recording"));
    if has_recording {
        return;
    }
    let (Some(source), Some(crop)) = (p.get("source").cloned(), p.get("crop").cloned()) else {
        return;
    };
    let main =
        json!({ "id": "main", "kind": "recording", "dir": "", "source": source, "crop": crop });
    match p.get_mut("clips").and_then(Value::as_array_mut) {
        Some(clips) => clips.insert(0, main),
        None => {
            p.insert("clips".into(), json!([main]));
        }
    }
}

/// Version 3 added markers, system audio and mute switches, all off by
/// default. Existing values are left as they are so that an older build
/// opening the file still hears what this one does (a microphone silenced
/// with a volume of zero stays at zero rather than becoming a mute).
fn v2_to_v3(p: &mut Map<String, Value>) {
    p.entry("markers").or_insert_with(|| json!([]));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Value {
        json!({
            "video": { "file": "video.mp4", "width": 1920, "height": 1080, "fps": 60, "durationMs": 5000 },
            "events": { "file": "events.jsonl" },
            "platform": "linux-x11", "target": "screen"
        })
    }

    fn v1() -> Value {
        json!({
            "format": "cinesnap-project", "version": 1, "createdAt": "2026-11-01T00:00:00Z", "app": "0.1.0",
            "source": source(),
            "crop": { "x": 10, "y": 0, "width": 1280, "height": 720 },
            "cuts": [{ "startMs": 0, "endMs": 400 }],
            "audio": { "micVolume": 0 }
        })
    }

    #[test]
    fn version_1_gets_its_recording_as_the_only_clip() {
        let mut m = v1();
        assert_eq!(migrate(&mut m), 1);
        assert_eq!(m["version"], PROJECT_VERSION);
        let clips = m["clips"].as_array().unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0]["id"], "main");
        assert_eq!(clips[0]["dir"], "");
        assert_eq!(clips[0]["crop"]["x"], 10);
        assert_eq!(clips[0]["source"], source());
        assert_eq!(m["cuts"][0]["endMs"], 400);
    }

    #[test]
    fn version_1_cards_stay_behind_the_recording() {
        let mut m = v1();
        m["clips"] = json!([{ "id": "c1", "kind": "card", "durationMs": 2000 }]);
        migrate(&mut m);
        let ids: Vec<&str> = m["clips"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["main", "c1"]);
    }

    #[test]
    fn audio_levels_are_left_for_older_builds() {
        let mut m = v1();
        migrate(&mut m);
        assert_eq!(m["audio"], json!({ "micVolume": 0 }));
        assert_eq!(m["markers"], json!([]));
    }

    #[test]
    fn version_2_keeps_its_clips() {
        let mut m = v1();
        m["version"] = json!(2);
        m["clips"] = json!([{ "id": "x", "kind": "recording", "dir": "", "source": source(), "crop": m["crop"] }]);
        let before = m["clips"].clone();
        assert_eq!(migrate(&mut m), 2);
        assert_eq!(m["clips"], before);
        assert_eq!(m["version"], PROJECT_VERSION);
    }

    #[test]
    fn missing_version_is_treated_as_1() {
        let mut m = v1();
        m.as_object_mut().unwrap().remove("version");
        assert_eq!(migrate(&mut m), 1);
        assert_eq!(m["clips"][0]["id"], "main");
    }

    #[test]
    fn current_and_newer_manifests_are_untouched() {
        for v in [PROJECT_VERSION, PROJECT_VERSION + 1] {
            let mut m = v1();
            m["version"] = json!(v);
            m["somethingNew"] = json!({ "a": 1 });
            let before = m.clone();
            assert_eq!(migrate(&mut m), v);
            assert_eq!(m, before);
        }
    }

    #[test]
    fn migrating_twice_changes_nothing() {
        let mut m = v1();
        migrate(&mut m);
        let once = m.clone();
        migrate(&mut m);
        assert_eq!(m, once);
    }
}
