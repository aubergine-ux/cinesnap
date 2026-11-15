//! Export presets: output size, frame rate and limits for the places demo
//! videos usually end up.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SizeRule {
    /// Fix the shorter side.
    Short(u32),
    /// Fix the width.
    Width(u32),
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    /// `mp4` or `gif`.
    pub format: &'static str,
    pub size: SizeRule,
    /// Overrides the project's frame rate.
    pub fps: Option<u32>,
    /// Overrides the project's aspect ratio.
    pub aspect: Option<&'static str>,
    /// The file is re-encoded smaller until it fits.
    pub max_bytes: Option<u64>,
    /// Longer videos get a warning; the destination would reject them.
    pub max_seconds: Option<u32>,
}

const MB: u64 = 1024 * 1024;

const fn plain(id: &'static str, label: &'static str, short: u32) -> ExportPreset {
    ExportPreset {
        id,
        label,
        hint: "",
        format: "mp4",
        size: SizeRule::Short(short),
        fps: None,
        aspect: None,
        max_bytes: None,
        max_seconds: None,
    }
}

pub const PRESETS: [ExportPreset; 10] = [
    ExportPreset {
        id: "readme",
        label: "GitHub README",
        hint: "GIF, 800 px wide, under GitHub's 10 MB image limit",
        format: "gif",
        size: SizeRule::Width(800),
        fps: Some(15),
        aspect: None,
        max_bytes: Some(10 * MB),
        max_seconds: None,
    },
    ExportPreset {
        id: "readme-mp4",
        label: "GitHub video",
        hint: "MP4, 720p, under GitHub's 10 MB upload limit",
        format: "mp4",
        size: SizeRule::Short(720),
        fps: Some(30),
        aspect: None,
        max_bytes: Some(10 * MB),
        max_seconds: None,
    },
    ExportPreset {
        id: "x",
        label: "X / Twitter",
        hint: "MP4, 1080p, 30 fps, at most 2:20 and 512 MB",
        format: "mp4",
        size: SizeRule::Short(1080),
        fps: Some(30),
        aspect: None,
        max_bytes: Some(512 * MB),
        max_seconds: Some(140),
    },
    ExportPreset {
        id: "youtube",
        label: "YouTube",
        hint: "MP4, 1440p, 60 fps (YouTube keeps more detail above 1080p)",
        format: "mp4",
        size: SizeRule::Short(1440),
        fps: Some(60),
        aspect: None,
        max_bytes: None,
        max_seconds: None,
    },
    ExportPreset {
        id: "shorts",
        label: "TikTok / Shorts",
        hint: "MP4, vertical 1080×1920, 30 fps, at most 3 minutes",
        format: "mp4",
        size: SizeRule::Short(1080),
        fps: Some(30),
        aspect: Some("9:16"),
        max_bytes: None,
        max_seconds: Some(180),
    },
    plain("4k", "4K", 2160),
    plain("1440p", "1440p", 1440),
    plain("1080p", "1080p", 1080),
    plain("720p", "720p", 720),
    ExportPreset {
        id: "small",
        label: "Small GIF",
        hint: "GIF, 480 px wide",
        format: "gif",
        size: SizeRule::Width(480),
        fps: Some(12),
        aspect: None,
        max_bytes: None,
        max_seconds: None,
    },
];

/// Look a preset up by id. `tiktok`, `twitter` and `github` are accepted as
/// other names for `shorts`, `x` and `readme`.
pub fn find(id: &str) -> Option<&'static ExportPreset> {
    let id = match id {
        "tiktok" => "shorts",
        "twitter" => "x",
        "github" => "readme",
        other => other,
    };
    PRESETS.iter().find(|p| p.id == id)
}

/// The preset for an id, or the default for the format when it is unknown.
pub fn resolve(id: &str, format: &str) -> &'static ExportPreset {
    find(id)
        .filter(|p| p.format == format)
        .unwrap_or_else(|| find(if format == "gif" { "readme" } else { "1080p" }).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_aliases_resolve() {
        let mut ids: Vec<_> = PRESETS.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len());
        assert_eq!(find("tiktok").unwrap().id, "shorts");
        assert_eq!(find("twitter").unwrap().id, "x");
        assert!(find("nope").is_none());
        assert_eq!(resolve("readme", "mp4").id, "1080p");
        assert_eq!(resolve("1080p", "gif").id, "readme");
        assert_eq!(resolve("shorts", "mp4").aspect, Some("9:16"));
    }
}
