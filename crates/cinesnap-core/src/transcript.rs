//! Working with the transcript: caption lines, subtitle files, and finding
//! filler words and silences worth cutting.

use serde::Serialize;

use crate::motion::Kept;
use crate::project::{Cut, Word};

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptionWord {
    pub text: String,
    pub start_ms: f64,
    pub end_ms: f64,
}

/// One caption as it appears on screen, in output time.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptionLine {
    pub start_ms: f64,
    pub end_ms: f64,
    pub words: Vec<CaptionWord>,
}

impl CaptionLine {
    pub fn text(&self) -> String {
        self.words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A pause longer than this starts a new caption.
const BREAK_GAP_MS: f64 = 700.0;
/// How long a caption lingers after its last word.
const LINGER_MS: f64 = 350.0;

/// Group the words that survive the cuts into caption lines.
pub fn caption_lines(words: &[Word], kept: &Kept, max_chars: usize) -> Vec<CaptionLine> {
    let max_chars = max_chars.max(8);
    let mut lines: Vec<CaptionLine> = Vec::new();
    let mut chars = 0;
    for w in words {
        let text = w.text.trim();
        if text.is_empty() || !kept.contains((w.t0 + w.t1) / 2.0) {
            continue;
        }
        let start = kept.map(w.t0);
        let end = kept.map(w.t1).max(start + 1.0);
        let len = text.chars().count();
        let new_line = match lines.last() {
            None => true,
            Some(line) => {
                let last = line.words.last().unwrap();
                let sentence_end = last.text.ends_with(['.', '?', '!']);
                chars + 1 + len > max_chars
                    || start - last.end_ms > BREAK_GAP_MS
                    || (sentence_end && chars * 2 >= max_chars)
            }
        };
        if new_line {
            lines.push(CaptionLine {
                start_ms: start,
                end_ms: end,
                words: vec![],
            });
            chars = 0;
        } else {
            chars += 1;
        }
        chars += len;
        let line = lines.last_mut().unwrap();
        line.end_ms = end;
        line.words.push(CaptionWord {
            text: text.to_owned(),
            start_ms: start,
            end_ms: end,
        });
    }
    // Hold each caption a moment, but never into the next one.
    for i in 0..lines.len() {
        let next = lines.get(i + 1).map_or(f64::MAX, |l| l.start_ms);
        lines[i].end_ms = (lines[i].end_ms + LINGER_MS).min(next);
    }
    lines
}

fn stamp(ms: f64, sep: char) -> String {
    let ms = ms.max(0.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}{sep}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// SubRip subtitles.
pub fn srt(lines: &[CaptionLine]) -> String {
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        out += &format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            stamp(l.start_ms, ','),
            stamp(l.end_ms, ','),
            l.text()
        );
    }
    out
}

/// WebVTT subtitles.
pub fn vtt(lines: &[CaptionLine]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for l in lines {
        // "-->" inside a cue would end it early.
        let text = l.text().replace("-->", "->");
        out += &format!(
            "{} --> {}\n{}\n\n",
            stamp(l.start_ms, '.'),
            stamp(l.end_ms, '.'),
            text
        );
    }
    out
}

/// A cut the user may want, shown for review before anything changes.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// `filler` or `silence`.
    pub kind: &'static str,
    pub start_ms: f64,
    pub end_ms: f64,
    pub label: String,
}

impl Suggestion {
    pub fn cut(&self) -> Cut {
        Cut {
            start_ms: self.start_ms,
            end_ms: self.end_ms,
        }
    }
}

const FILLERS: [&str; 14] = [
    "um", "umm", "uh", "uhh", "uhm", "er", "erm", "hmm", "hm", "mm", "mmm", "mhm", "ah", "eh",
];

fn bare(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn is_filler(word: &str) -> bool {
    FILLERS.contains(&bare(word).as_str())
}

/// Filler words that are not already cut. Each cut covers the word and half
/// of the pause on either side, so the sentence closes up naturally.
pub fn find_fillers(words: &[Word], kept: &Kept) -> Vec<Suggestion> {
    const MAX_PAD_MS: f64 = 120.0;
    let mut out = Vec::new();
    for (i, w) in words.iter().enumerate() {
        if !is_filler(&w.text) || !kept.contains((w.t0 + w.t1) / 2.0) {
            continue;
        }
        let before = i.checked_sub(1).map_or(w.t0, |j| words[j].t1.min(w.t0));
        let after = words.get(i + 1).map_or(w.t1, |n| n.t0.max(w.t1));
        let start = w.t0 - ((w.t0 - before) / 2.0).min(MAX_PAD_MS);
        let end = w.t1 + ((after - w.t1) / 2.0).min(MAX_PAD_MS);
        if end - start >= 40.0 {
            out.push(Suggestion {
                kind: "filler",
                start_ms: start,
                end_ms: end,
                label: w.text.trim().to_owned(),
            });
        }
    }
    out
}

/// Turn detected silences (timeline time) into cuts that leave `keep_ms` of
/// quiet at each end. Parts that are already cut are skipped.
pub fn silence_cuts(
    silences: &[(f64, f64)],
    kept: &Kept,
    min_ms: f64,
    keep_ms: f64,
) -> Vec<Suggestion> {
    let mut out = Vec::new();
    for &(s, e) in silences {
        // Only the part of the silence that is still in the video counts.
        for &(ks, ke) in &kept.ranges {
            let (a, b) = (s.max(ks), e.min(ke));
            if b - a < min_ms {
                continue;
            }
            // Silence touching the very start or end needs no lead-in there.
            let start = if a <= ks + 1.0 { a } else { a + keep_ms };
            let end = if b >= ke - 1.0 { b } else { b - keep_ms };
            if end - start >= 100.0 {
                out.push(Suggestion {
                    kind: "silence",
                    start_ms: start,
                    end_ms: end,
                    label: format!("{:.1} s of silence", (b - a) / 1000.0),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(t0: f64, t1: f64, text: &str) -> Word {
        Word {
            t0,
            t1,
            text: text.into(),
        }
    }

    fn words(text: &str) -> Vec<Word> {
        text.split(' ')
            .enumerate()
            .map(|(i, w)| word(i as f64 * 300.0, i as f64 * 300.0 + 250.0, w))
            .collect()
    }

    #[test]
    fn lines_respect_length_pauses_and_cuts() {
        let mut w = words("one two three four five six seven eight nine ten");
        let kept = Kept::new(10_000.0, &[]);
        let lines = caption_lines(&w, &kept, 16);
        assert!(lines.iter().all(|l| l.text().len() <= 16), "{lines:?}");
        assert_eq!(lines[0].text(), "one two three");
        assert_eq!(lines.iter().map(|l| l.words.len()).sum::<usize>(), w.len());
        // Lines never overlap.
        for pair in lines.windows(2) {
            assert!(pair[0].end_ms <= pair[1].start_ms);
        }

        // A long pause splits a line that would otherwise fit.
        for x in &mut w[2..] {
            x.t0 += 2000.0;
            x.t1 += 2000.0;
        }
        let lines = caption_lines(&w, &kept, 60);
        assert_eq!(lines[0].text(), "one two");

        // Cut words disappear and later words move earlier.
        let kept = Kept::new(
            10_000.0,
            &[Cut {
                start_ms: 0.0,
                end_ms: 600.0,
            }],
        );
        let lines = caption_lines(&words("a b c d"), &kept, 60);
        assert_eq!(lines[0].text(), "c d");
        assert_eq!(lines[0].start_ms, 0.0);
    }

    #[test]
    fn subtitle_files() {
        let kept = Kept::new(100_000.0, &[]);
        let lines = caption_lines(&[word(61_500.0, 62_000.0, "Hello --> there.")], &kept, 40);
        assert_eq!(
            srt(&lines),
            "1\n00:01:01,500 --> 00:01:02,350\nHello --> there.\n\n"
        );
        assert_eq!(
            vtt(&lines),
            "WEBVTT\n\n00:01:01.500 --> 00:01:02.350\nHello -> there.\n\n"
        );
        assert_eq!(srt(&[]), "");
    }

    #[test]
    fn fillers_are_found_with_padding() {
        let w = vec![
            word(0.0, 400.0, "So,"),
            word(600.0, 900.0, "Um,"),
            word(1000.0, 1400.0, "hello"),
            word(1500.0, 1800.0, "umbrella"),
            word(5000.0, 5200.0, "uh"),
        ];
        let kept = Kept::new(10_000.0, &[]);
        let f = find_fillers(&w, &kept);
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].start_ms, f[0].end_ms), (500.0, 950.0));
        assert_eq!(f[0].label, "Um,");
        // Padding is capped where the pause is long.
        assert_eq!((f[1].start_ms, f[1].end_ms), (4880.0, 5200.0));
        // Already cut: nothing to suggest.
        let kept = Kept::new(
            10_000.0,
            &[Cut {
                start_ms: 500.0,
                end_ms: 1000.0,
            }],
        );
        assert_eq!(find_fillers(&w, &kept).len(), 1);
    }

    #[test]
    fn silences_keep_a_breath() {
        let kept = Kept::new(
            20_000.0,
            &[Cut {
                start_ms: 9000.0,
                end_ms: 12_000.0,
            }],
        );
        let s = silence_cuts(
            &[
                (0.0, 1500.0),
                (3000.0, 3400.0),
                (5000.0, 7000.0),
                (8000.0, 13_000.0),
            ],
            &kept,
            1000.0,
            200.0,
        );
        let ranges: Vec<_> = s.iter().map(|x| (x.start_ms, x.end_ms)).collect();
        // Leading silence is cut from the very start; the short one is left
        // alone; the one straddling an existing cut only counts outside it.
        assert_eq!(
            ranges,
            vec![
                (0.0, 1300.0),
                (5200.0, 6800.0),
                (8200.0, 9000.0),
                (12_000.0, 12_800.0)
            ]
        );
    }
}
