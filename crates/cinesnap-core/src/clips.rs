//! Adding recordings to a project's timeline.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::project::{
    Clip, ClipBody, Cut, Project, RecordingClip, Transition, TransitionKind, Zoom,
};

/// A recording copied into a bundle, with the edits it carried, in the
/// clip's own time.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub clip: Clip,
    pub cuts: Vec<Cut>,
    pub zooms: Vec<Zoom>,
}

/// An id that no clip of `project` uses yet.
pub fn fresh_id(project: &Project) -> String {
    (1..)
        .map(|n| format!("clip{n}"))
        .find(|id| project.clips.iter().all(|c| &c.id != id))
        .unwrap()
}

/// Hard-link where possible (same disk: instant, no extra space), copy
/// otherwise.
fn place(from: &Path, to: &Path) -> Result<()> {
    if std::fs::hard_link(from, to).is_err() {
        std::fs::copy(from, to)
            .with_context(|| format!("copying {} into the project", from.display()))?;
    }
    Ok(())
}

/// Copy the recording in the bundle `from` into the bundle `dir` as a new
/// clip. The project file is not touched; the caller appends the clip.
pub fn import_recording(dir: &Path, project: &Project, from: &Path) -> Result<Imported> {
    let other = Project::load(from)?;
    let (index, rec) = other
        .clips
        .iter()
        .enumerate()
        .find_map(|(i, c)| c.recording().filter(|r| r.dir.is_empty()).map(|r| (i, r)))
        .context("that recording has no video of its own")?;
    let span = other.spans()[index];
    let id = fresh_id(project);
    let sub = format!("clips/{id}");
    let target: PathBuf = dir.join(&sub);
    std::fs::create_dir_all(&target)?;
    let src = &rec.source;
    let files = [Some(&src.video.file), Some(&src.events.file)]
        .into_iter()
        .chain([
            src.mic.as_ref().map(|m| &m.file),
            src.webcam.as_ref().map(|w| &w.file),
        ])
        .flatten();
    let result = files.into_iter().try_for_each(|file| {
        let source = from.join(file);
        // The event log is optional: a recording without one just has no
        // effects.
        if !source.is_file() && file == &src.events.file {
            return Ok(());
        }
        place(&source, &target.join(file))
    });
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&target);
        return Err(e);
    }
    // Cursor bitmaps, when the platform recorded them.
    let cursors = from.join("cursors");
    if let Ok(entries) = std::fs::read_dir(&cursors) {
        let _ = std::fs::create_dir_all(target.join("cursors"));
        for e in entries.flatten() {
            let _ = place(&e.path(), &target.join("cursors").join(e.file_name()));
        }
    }
    let local = other.local(span).context("that recording cannot be read")?;
    Ok(Imported {
        clip: Clip {
            id,
            transition: Transition {
                kind: TransitionKind::Crossfade,
                duration_ms: 500.0,
            },
            body: ClipBody::Recording(RecordingClip {
                dir: sub,
                source: rec.source.clone(),
                crop: rec.crop,
            }),
        },
        cuts: local.cuts,
        zooms: local.zooms,
    })
}

/// Append an imported clip to the end of the timeline, bringing its cuts and
/// zooms along.
pub fn append(project: &mut Project, imported: Imported) {
    let start = project.duration_ms();
    let id = imported.clip.id.clone();
    project.clips.push(imported.clip);
    project.cuts.extend(imported.cuts.into_iter().map(|c| Cut {
        start_ms: c.start_ms + start,
        end_ms: c.end_ms + start,
    }));
    project
        .zooms
        .extend(imported.zooms.into_iter().map(|z| Zoom {
            id: format!("{id}-{}", z.id),
            start_ms: z.start_ms + start,
            end_ms: z.end_ms + start,
            ..z
        }));
    project.sanitize();
}

/// Move everything on the timeline at or after `at_ms` by `delta_ms`.
pub fn shift_from(project: &mut Project, at_ms: f64, delta_ms: f64) {
    let moved = |t: f64| if t >= at_ms - 0.5 { t + delta_ms } else { t };
    for c in &mut project.cuts {
        (c.start_ms, c.end_ms) = (moved(c.start_ms), moved(c.end_ms));
    }
    for z in &mut project.zooms {
        (z.start_ms, z.end_ms) = (moved(z.start_ms), moved(z.end_ms));
    }
    for a in &mut project.annotations {
        (a.start_ms, a.end_ms) = (moved(a.start_ms), moved(a.end_ms));
    }
    for k in &mut project.webcam_keys {
        k.at_ms = moved(k.at_ms);
    }
    if let Some(t) = &mut project.transcript {
        for w in &mut t.words {
            (w.t0, w.t1) = (moved(w.t0), moved(w.t1));
        }
    }
}

/// Put a clip on the timeline before the clip at `index`, moving later
/// edits out of its way.
pub fn insert(project: &mut Project, index: usize, clip: Clip) {
    let index = index.min(project.clips.len());
    let at = project
        .spans()
        .get(index)
        .map_or(project.duration_ms(), |s| s.start_ms);
    shift_from(project, at, clip.duration_ms());
    project.clips.insert(index, clip);
}

/// Remove the folders of clips that are no longer on the timeline. Call
/// only when the removal can no longer be undone (when the project closes).
pub fn remove_orphans(dir: &Path, project: &Project) {
    let Ok(entries) = std::fs::read_dir(dir.join("clips")) else {
        return;
    };
    for entry in entries.flatten() {
        let name = format!("clips/{}", entry.file_name().to_string_lossy());
        let used = project
            .clips
            .iter()
            .filter_map(Clip::recording)
            .any(|r| r.dir == name);
        if !used {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::project;

    fn bundle(dir: &Path, duration_ms: f64) -> Project {
        std::fs::create_dir_all(dir).unwrap();
        let mut p = project(640, 360, duration_ms);
        p.cuts.push(Cut {
            start_ms: duration_ms - 500.0,
            end_ms: duration_ms,
        });
        std::fs::write(dir.join("video.mp4"), b"video").unwrap();
        std::fs::write(dir.join("events.jsonl"), b"{}\n").unwrap();
        p.save(dir).unwrap();
        p
    }

    #[test]
    fn imports_append_and_clean_up() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a.cinesnap"), tmp.path().join("b.cinesnap"));
        let mut p = bundle(&a, 4000.0);
        bundle(&b, 3000.0);

        let imported = import_recording(&a, &p, &b).unwrap();
        assert_eq!(imported.clip.id, "clip1");
        assert!(a.join("clips/clip1/video.mp4").is_file());
        assert!(a.join("clips/clip1/events.jsonl").is_file());
        append(&mut p, imported);
        assert_eq!(p.clips.len(), 2);
        assert_eq!(p.duration_ms(), 7000.0);
        // The second recording's trailing cut came along, shifted.
        assert_eq!(
            p.cuts[1],
            Cut {
                start_ms: 6500.0,
                end_ms: 7000.0
            }
        );
        p.validate().unwrap();
        p.save(&a).unwrap();
        assert_eq!(Project::load(&a).unwrap().clips.len(), 2);

        let again = import_recording(&a, &p, &b).unwrap();
        assert_eq!(again.clip.id, "clip2");
        // Never appended: an orphan, removed on clean-up; clip1 stays.
        remove_orphans(&a, &p);
        assert!(!a.join("clips/clip2").exists());
        assert!(a.join("clips/clip1/video.mp4").is_file());

        // A card in front moves every edit along.
        insert(
            &mut p,
            0,
            Clip {
                id: "title".into(),
                transition: Transition::default(),
                body: ClipBody::Card(crate::project::Card {
                    duration_ms: 2000.0,
                    ..Default::default()
                }),
            },
        );
        assert_eq!(
            p.cuts[0],
            Cut {
                start_ms: 5500.0,
                end_ms: 6000.0
            }
        );
        assert_eq!(
            p.cuts[1],
            Cut {
                start_ms: 8500.0,
                end_ms: 9000.0
            }
        );
        assert_eq!(p.spans()[1].start_ms, 2000.0);

        // Not a recording, or one with its video missing.
        assert!(import_recording(&a, &p, tmp.path()).is_err());
        std::fs::remove_file(b.join("video.mp4")).unwrap();
        assert!(import_recording(&a, &p, &b).is_err());
        assert!(!a.join("clips/clip2").exists(), "nothing left behind");
    }
}
