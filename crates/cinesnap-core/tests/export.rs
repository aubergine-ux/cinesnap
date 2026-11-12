//! End-to-end: synthesize a recording, export it, inspect the result.
//! Skipped (with a note) when ffmpeg is not installed.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use cinesnap_core::export::{export, Cancelled, ExportOptions};
use cinesnap_core::ffmpeg::{self, Encoder, FrameReader};
use cinesnap_core::project::{Background, Cut, Project};
use cinesnap_core::synth::{generate, SynthOptions};

fn ffmpeg() -> Option<PathBuf> {
    let f = ffmpeg::locate(None);
    if f.is_none() {
        eprintln!("ffmpeg not found, skipping");
    }
    f
}

fn demo(ffmpeg: &Path, dir: &Path) -> Project {
    generate(
        dir,
        ffmpeg,
        &SynthOptions {
            width: 640,
            height: 360,
            fps: 30,
            baked_cursor: false,
        },
    )
    .unwrap()
}

fn run(dir: &Path, project: &Project, ffmpeg: &Path, out: &Path) -> anyhow::Result<()> {
    let opts = ExportOptions {
        ffmpeg,
        out,
        encoder: Some(Encoder::X264),
    };
    export(dir, project, &opts, &mut |_| {}, &AtomicBool::new(false))
}

fn frame(ffmpeg: &Path, file: &Path, w: usize, h: usize, index: u64) -> Vec<u8> {
    let mut r = FrameReader::open(ffmpeg, file, "null", w, h, 0.0).unwrap();
    r.frame_at(index).unwrap().to_vec()
}

fn px(buf: &[u8], w: usize, x: usize, y: usize) -> [i32; 3] {
    let o = (y * w + x) * 4;
    [buf[o] as i32, buf[o + 1] as i32, buf[o + 2] as i32]
}

fn close(a: [i32; 3], b: [i32; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 24)
}

#[test]
fn mp4_export_has_expected_size_length_and_content() {
    let Some(ff) = ffmpeg() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo.cinesnap");
    let mut p = demo(&ff, &dir);
    assert_eq!(
        p.zooms.len(),
        1,
        "all activity is close enough to merge: {:?}",
        p.zooms
    );
    p.export.preset = "720p".into();
    p.export.fps = 30;
    p.frame.background = Background::Solid {
        color: "#ff0000".into(),
    };
    p.frame.shadow = 0.0;
    p.cuts = vec![
        Cut {
            start_ms: 0.0,
            end_ms: 500.0,
        },
        Cut {
            start_ms: 11_000.0,
            end_ms: 12_500.0,
        },
    ];

    let out = tmp.path().join("out.mp4");
    run(&dir, &p, &ff, &out).unwrap();
    let info = ffmpeg::probe(&ff, &out).unwrap();
    assert_eq!((info.width, info.height), (1280, 720));
    assert!(
        (info.duration_ms - 10_500.0).abs() < 100.0,
        "{}",
        info.duration_ms
    );
    assert!(!tmp.path().join("out.part.mp4").exists());

    // First frame: not zoomed. Background in the padding, sidebar colour at
    // the left of the frame, page colour in the middle.
    let f0 = frame(&ff, &out, 1280, 720, 0);
    assert!(
        close(px(&f0, 1280, 8, 8), [255, 0, 0]),
        "{:?}",
        px(&f0, 1280, 8, 8)
    );
    assert!(
        close(px(&f0, 1280, 93, 586), [0x15, 0x18, 0x23]),
        "{:?}",
        px(&f0, 1280, 93, 586)
    );
    // While zoomed on the sidebar click (source 1.6 s → output 1.1 s) the
    // sidebar fills far more of the frame: the same spot that was page
    // background is now sidebar.
    let before = px(&f0, 1280, 276, 172);
    let zoomed = px(&frame(&ff, &out, 1280, 720, 45), 1280, 276, 172);
    assert!(close(before, [0x0f, 0x11, 0x17]), "{before:?}");
    assert!(
        !close(zoomed, before),
        "zoom changed what is under that pixel: {zoomed:?}"
    );
}

#[test]
fn gif_export_and_vertical_aspect() {
    let Some(ff) = ffmpeg() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo.cinesnap");
    let mut p = demo(&ff, &dir);
    p.export.format = "gif".into();
    p.export.preset = "small".into();
    p.cuts = vec![Cut {
        start_ms: 3000.0,
        end_ms: 12_500.0,
    }];
    let out = tmp.path().join("out.gif");
    run(&dir, &p, &ff, &out).unwrap();
    let info = ffmpeg::probe(&ff, &out).unwrap();
    assert_eq!((info.width, info.height), (480, 270));
    assert_eq!(&std::fs::read(&out).unwrap()[..6], b"GIF89a");

    p.export.format = "mp4".into();
    p.export.preset = "720p".into();
    p.frame.aspect = "9:16".into();
    let out = tmp.path().join("tall.mp4");
    run(&dir, &p, &ff, &out).unwrap();
    let info = ffmpeg::probe(&ff, &out).unwrap();
    assert_eq!((info.width, info.height), (720, 1280));
}

#[test]
fn cancel_and_empty_exports_fail_cleanly() {
    let Some(ff) = ffmpeg() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo.cinesnap");
    let mut p = demo(&ff, &dir);
    p.export.preset = "720p".into();
    let out = tmp.path().join("out.mp4");
    let opts = ExportOptions {
        ffmpeg: &ff,
        out: &out,
        encoder: Some(Encoder::X264),
    };

    let cancel = AtomicBool::new(false);
    let err = export(
        &dir,
        &p,
        &opts,
        &mut |pr| {
            if pr.frame == 5 {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        },
        &cancel,
    )
    .unwrap_err();
    assert!(err.is::<Cancelled>(), "{err:#}");
    assert!(!out.exists() && !tmp.path().join("out.part.mp4").exists());

    p.cuts = vec![Cut {
        start_ms: 0.0,
        end_ms: 99_999.0,
    }];
    let err = run(&dir, &p, &ff, &out).unwrap_err();
    assert!(err.to_string().contains("nothing to export"), "{err:#}");
}

#[test]
fn broken_recordings_fail_without_leftovers() {
    let Some(ff) = ffmpeg() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo.cinesnap");
    let mut p = demo(&ff, &dir);
    p.export.preset = "720p".into();
    let out = tmp.path().join("out.mp4");
    let leftovers = || out.exists() || tmp.path().join("out.part.mp4").exists();

    // A missing or unreadable event log only means no effects.
    std::fs::write(p.events_path(&dir), "garbage{{{\n").unwrap();
    run(&dir, &p, &ff, &out).unwrap();
    std::fs::remove_file(&out).unwrap();

    let missing = tmp.path().join("nowhere").join("out.mp4");
    let err = run(&dir, &p, &ff, &missing).unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err:#}");

    std::fs::write(p.video_path(&dir), "not a video").unwrap();
    let err = run(&dir, &p, &ff, &out).unwrap_err();
    assert!(err.to_string().contains("could not decode"), "{err:#}");
    assert!(!leftovers());

    p.source.video.width = 0;
    assert!(run(&dir, &p, &ff, &out).is_err());
    assert!(Project::load(&dir).is_ok(), "the saved manifest is intact");
    assert!(!leftovers());
}

fn has_audio(ffmpeg: &Path, file: &Path) -> bool {
    let out = ffmpeg::command(ffmpeg)
        .arg("-i")
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stderr).contains("Audio: aac")
}

/// A "voice": bursts of tone with silence in between.
fn fake_mic(ffmpeg: &Path, dir: &Path) {
    let mut c = ffmpeg::command(ffmpeg);
    c.args(["-v", "error", "-y", "-f", "lavfi", "-i"]);
    c.arg("aevalsrc=0.4*sin(2*PI*220*t)*between(mod(t\\,4)\\,1\\,3):d=13:s=48000");
    c.arg(dir.join("mic.wav"));
    assert!(c.status().unwrap().success());
}

#[test]
fn clips_cards_transitions_annotations_and_sound() {
    use cinesnap_core::audio::{build_mix, Wav};
    use cinesnap_core::clips;
    use cinesnap_core::project::{
        Annotation, AudioTrack, Card, Clip, ClipBody, Music, Shape, Transition, TransitionKind,
    };

    let Some(ff) = ffmpeg() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("a.cinesnap");
    let other = tmp.path().join("b.cinesnap");
    let mut p = demo(&ff, &dir);
    demo(&ff, &other);
    let length = p.duration_ms();
    fake_mic(&ff, &dir);
    p.main_mut().source.mic = Some(AudioTrack {
        file: "mic.wav".into(),
        offset_ms: 0.0,
    });

    // Keep four seconds of each recording, joined by a crossfade, behind a
    // two second title card.
    p.zooms.clear();
    p.cuts = vec![Cut {
        start_ms: 4000.0,
        end_ms: length,
    }];
    let mut imported = clips::import_recording(&dir, &p, &other).unwrap();
    imported.zooms.clear();
    imported.cuts = vec![Cut {
        start_ms: 4000.0,
        end_ms: length,
    }];
    imported.clip.transition = Transition {
        kind: TransitionKind::Crossfade,
        duration_ms: 1000.0,
    };
    clips::append(&mut p, imported);
    clips::insert(
        &mut p,
        0,
        Clip {
            id: "title".into(),
            transition: Transition::default(),
            body: ClipBody::Card(Card {
                duration_ms: 2000.0,
                title: "Cinesnap".into(),
                background: Some(Background::Solid {
                    color: "#0000ff".into(),
                }),
                ..Card::default()
            }),
        },
    );
    // Hide the top-left corner of the second recording.
    let second = p.spans()[2].start_ms;
    p.annotations.push(Annotation {
        id: "blur".into(),
        start_ms: second,
        end_ms: second + 4000.0,
        shape: Shape::Spotlight {
            x: 320.0,
            y: 0.0,
            width: 320.0,
            height: 360.0,
            dim: 1.0,
            ellipse: false,
        },
    });
    p.audio.music = Some(Music {
        source: "builtin:open-sky".into(),
        ..Music::default()
    });
    p.frame.background = Background::Solid {
        color: "#ff0000".into(),
    };
    p.frame.shadow = 0.0;
    p.export.preset = "720p".into();
    p.export.fps = 30;
    p.sanitize();
    p.validate().unwrap();

    let out = tmp.path().join("out.mp4");
    run(&dir, &p, &ff, &out).unwrap();
    let info = ffmpeg::probe(&ff, &out).unwrap();
    assert!(
        (info.duration_ms - 10_000.0).abs() < 150.0,
        "{}",
        info.duration_ms
    );
    assert!(has_audio(&ff, &out));

    // The card, then the first recording.
    let card = frame(&ff, &out, 1280, 720, 30);
    assert!(
        close(px(&card, 1280, 8, 8), [0, 0, 255]),
        "{:?}",
        px(&card, 1280, 8, 8)
    );
    let first = frame(&ff, &out, 1280, 720, 100);
    assert!(close(px(&first, 1280, 8, 8), [255, 0, 0]));
    let lit = px(&first, 1280, 276, 172);
    // In the second recording the left half is blacked out by the spotlight
    // and the right half is not.
    let late = frame(&ff, &out, 1280, 720, 280);
    assert!(
        close(px(&late, 1280, 276, 172), [0, 0, 0]),
        "{:?}",
        px(&late, 1280, 276, 172)
    );
    assert!(!close(px(&late, 1280, 1000, 300), [0, 0, 0]));
    // Part way through the crossfade that pixel is part way there.
    let mid = px(&frame(&ff, &out, 1280, 720, 184), 1280, 276, 172);
    assert!(
        mid[2] > 1 && mid[0] < lit[0] && mid[2] < lit[2],
        "{mid:?} between {lit:?} and black"
    );

    // The soundtrack: music alone under the card, voice bursts from the
    // first recording, the music lowered while they play.
    let mix = tmp.path().join("mix.wav");
    assert!(build_mix(&dir, &p, &ff, &mix, &mut |_| {}, &AtomicBool::new(false)).unwrap());
    let mut wav = Wav::open(&mix).unwrap();
    assert_eq!(wav.channels, 2);
    assert!((wav.frames as i64 - 480_000).abs() < 2000, "{}", wav.frames);
    let mut level = |from_s: f64| {
        let mut buf = vec![0i16; 48_000];
        wav.read((from_s * 48_000.0) as u64, &mut buf).unwrap();
        (buf.iter()
            .map(|&s| (s as f64 / 32768.0).powi(2))
            .sum::<f64>()
            / buf.len() as f64)
            .sqrt()
    };
    let (music_only, voice, second_clip) = (level(1.0), level(3.6), level(8.0));
    assert!(music_only > 0.005, "{music_only}");
    assert!(voice > 0.15, "{voice}");
    assert!(second_clip > 0.005 && second_clip < 0.1, "{second_clip}");

    // Without music or a microphone there is no audio stream at all.
    p.audio.music = None;
    p.main_mut().source.mic = None;
    let silent = tmp.path().join("silent.mp4");
    run(&dir, &p, &ff, &silent).unwrap();
    assert!(!has_audio(&ff, &silent));
}

#[test]
fn readme_presets_stay_under_the_size_limit() {
    let Some(ff) = ffmpeg() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo.cinesnap");
    let mut p = demo(&ff, &dir);
    p.cuts = vec![Cut {
        start_ms: 3000.0,
        end_ms: 99_999.0,
    }];
    p.export.format = "mp4".into();
    p.export.preset = "readme-mp4".into();
    p.export.fps = 60;
    let out = tmp.path().join("readme.mp4");
    run(&dir, &p, &ff, &out).unwrap();
    let info = ffmpeg::probe(&ff, &out).unwrap();
    assert_eq!((info.width, info.height, info.fps), (1280, 720, 30.0));
    assert!(std::fs::metadata(&out).unwrap().len() < 10 * 1024 * 1024);

    p.export.format = "gif".into();
    p.export.preset = "readme".into();
    let out = tmp.path().join("readme.gif");
    run(&dir, &p, &ff, &out).unwrap();
    assert_eq!(ffmpeg::probe(&ff, &out).unwrap().width, 800);
    assert!(std::fs::metadata(&out).unwrap().len() < 10 * 1024 * 1024);

    p.export.format = "mp4".into();
    p.export.preset = "shorts".into();
    let out = tmp.path().join("shorts.mp4");
    run(&dir, &p, &ff, &out).unwrap();
    let info = ffmpeg::probe(&ff, &out).unwrap();
    assert_eq!((info.width, info.height), (1080, 1920));
}
