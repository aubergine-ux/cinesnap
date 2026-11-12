//! Core of Cinesnap: the project format, the motion model, the frame
//! compositor and the export pipeline. Nothing here depends on the GUI.

pub mod audio;
pub mod cli;
pub mod clips;
pub mod events;
pub mod export;
pub mod ffmpeg;
pub mod layout;
pub mod motion;
pub mod music;
pub mod presets;
pub mod project;
pub mod render;
pub mod share;
pub mod synth;
pub mod text;
pub mod transcribe;
pub mod transcript;

#[cfg(test)]
pub(crate) mod testutil {
    use crate::project::*;

    pub fn project(width: u32, height: u32, duration_ms: f64) -> Project {
        Project::new(
            Source {
                video: VideoTrack {
                    file: VIDEO_FILE.into(),
                    width,
                    height,
                    fps: 60.0,
                    duration_ms,
                },
                mic: None,
                webcam: None,
                events: FileRef {
                    file: EVENTS_FILE.into(),
                },
                platform: "test".into(),
                target: "screen".into(),
            },
            "2026-01-01T00:00:00Z".into(),
        )
    }
}
