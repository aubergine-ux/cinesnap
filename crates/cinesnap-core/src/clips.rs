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
/// Something that merely ends at `at_ms` stays where it is; a cut running
/// across `at_ms` is split so it does not swallow whatever is being
/// inserted there.
pub fn shift_from(project: &mut Project, at_ms: f64, delta_ms: f64) {
    const EPS: f64 = 0.5;
    let moved = |t: f64| if t >= at_ms - EPS { t + delta_ms } else { t };
    let mut cuts = Vec::with_capacity(project.cuts.len());
    for c in &project.cuts {
        if c.end_ms <= at_ms + EPS {
            cuts.push(*c);
        } else if c.start_ms >= at_ms - EPS {
            cuts.push(Cut {
                start_ms: c.start_ms + delta_ms,
                end_ms: c.end_ms + delta_ms,
            });
        } else {
            cuts.push(Cut {
                start_ms: c.start_ms,
                end_ms: at_ms,
            });
            cuts.push(Cut {
                start_ms: at_ms + delta_ms,
                end_ms: c.end_ms + delta_ms,
            });
        }
    }
    cuts.retain(|c| c.end_ms - c.start_ms >= 1.0);
    project.cuts = cuts;
    // Zooms and annotations belong to the clip they start in.
    let range = |start: &mut f64, end: &mut f64| {
        if *start >= at_ms - EPS {
            *start += delta_ms;
            *end += delta_ms;
        } else {
            *end = end.min(at_ms);
        }
    };
    for z in &mut project.zooms {
        range(&mut z.start_ms, &mut z.end_ms);
    }
    for a in &mut project.annotations {
        range(&mut a.start_ms, &mut a.end_ms);
    }
    for k in &mut project.webcam_keys {
        k.at_ms = moved(k.at_ms);
    }
    if let Some(t) = &mut project.transcript {
        for w in &mut t.words {
            if w.t0 >= at_ms - EPS {
                w.t0 += delta_ms;
                w.t1 += delta_ms;
            }
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
        // A card at the very end does not stretch the cut that ends there,
        // and one between the recordings splits a cut that spans the join.
        let outro = |id: &str| Clip {
            id: id.into(),
            transition: Transition::default(),
            body: ClipBody::Card(crate::project::Card::default()),
        };
        insert(&mut p, 99, outro("outro"));
        assert_eq!(
            p.cuts[1],
            Cut {
                start_ms: 8500.0,
                end_ms: 9000.0
            }
        );
        assert_eq!(p.duration_ms(), 12_000.0);
        p.cuts[0].end_ms = 6500.0;
        insert(&mut p, 2, outro("middle"));
        assert_eq!(
            p.cuts,
            vec![
                Cut {
                    start_ms: 5500.0,
                    end_ms: 6000.0
                },
                Cut {
                    start_ms: 9000.0,
                    end_ms: 9500.0
                },
                Cut {
                    start_ms: 11_500.0,
                    end_ms: 12_000.0
                },
            ]
        );

        // Not a recording, or one with its video missing.
        assert!(import_recording(&a, &p, tmp.path()).is_err());
        std::fs::remove_file(b.join("video.mp4")).unwrap();
        assert!(import_recording(&a, &p, &b).is_err());
        assert!(!a.join("clips/clip2").exists(), "nothing left behind");
    }
}
