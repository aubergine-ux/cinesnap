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
