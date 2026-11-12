//! The input event log (`events.jsonl`).

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Header {
    pub format: String,
    pub version: u32,
    pub width: u32,
    pub height: u32,
    /// `exact`, `inferred` or `none`.
    pub clicks: String,
    /// `metadata`, `embedded` or `none`.
    pub cursor: String,
}

impl Header {
    pub fn new(width: u32, height: u32, clicks: &str, cursor: &str) -> Self {
        Self {
            format: "cinesnap-events".into(),
            version: 1,
            width,
            height,
            clicks: clicks.into(),
            cursor: cursor.into(),
        }
    }
    /// Whether the cursor position is known and the cursor is absent from the
    /// video, i.e. whether it can be redrawn.
    pub fn can_draw_cursor(&self) -> bool {
        self.cursor == "metadata"
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "k", rename_all = "lowercase")]
pub enum EventKind {
    Move {
        x: f64,
        y: f64,
    },
    Down {
        b: Button,
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        inferred: bool,
    },
    Up {
        b: Button,
        x: f64,
        y: f64,
    },
    Key,
    Scroll {
        dx: f64,
        dy: f64,
        x: f64,
        y: f64,
    },
    Cursor {
        id: u32,
        hx: f64,
        hy: f64,
        w: u32,
        h: u32,
    },
    Visible {
        v: bool,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Event {
    pub t: f64,
    #[serde(flatten)]
    pub kind: EventKind,
}

impl Event {
    pub fn position(&self) -> Option<(f64, f64)> {
        match self.kind {
            EventKind::Move { x, y }
            | EventKind::Down { x, y, .. }
            | EventKind::Up { x, y, .. }
            | EventKind::Scroll { x, y, .. } => Some((x, y)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EventLog {
    pub header: Header,
    pub events: Vec<Event>,
}

impl EventLog {
    pub fn empty(width: u32, height: u32) -> Self {
        Self { header: Header::new(width, height, "none", "none"), events: vec![] }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::read(BufReader::new(file))
    }

    pub fn read(reader: impl BufRead) -> Result<Self> {
        let mut lines = reader.lines();
        let first = lines.next().context("event log is empty")??;
        let header: Header = serde_json::from_str(&first).context("parsing event log header")?;
        anyhow::ensure!(header.format == "cinesnap-events", "not a Cinesnap event log");
        let mut events = Vec::new();
        for line in lines {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            // Unparseable lines are skipped: a crash can truncate the last one.
            match serde_json::from_str::<Event>(&line) {
                Ok(e) if !matches!(e.kind, EventKind::Unknown) && e.t.is_finite() => events.push(e),
                _ => {}
            }
        }
        // Writers promise ordering, but a stable sort makes it a guarantee.
        events.sort_by(|a, b| a.t.total_cmp(&b.t));
        Ok(Self { header, events })
    }
}

/// Append-only writer used while recording.
pub struct EventWriter {
    out: BufWriter<File>,
}

impl EventWriter {
    pub fn create(path: &Path, header: &Header) -> Result<Self> {
        let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
        let mut out = BufWriter::new(file);
        serde_json::to_writer(&mut out, header)?;
        out.write_all(b"\n")?;
        Ok(Self { out })
    }

    pub fn write(&mut self, event: &Event) -> Result<()> {
        serde_json::to_writer(&mut self.out, event)?;
        self.out.write_all(b"\n")?;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        Ok(self.out.flush()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documented_example() {
        let text = r#"{"format":"cinesnap-events","version":1,"width":2560,"height":1440,"clicks":"exact","cursor":"metadata"}
{"t":12.4,"k":"move","x":812.0,"y":430.5}
{"t":341.9,"k":"down","b":"left","x":903.0,"y":455.0}
{"t":422.3,"k":"up","b":"left","x":903.0,"y":455.0}
{"t":1310.0,"k":"key"}
{"t":1402.2,"k":"scroll","dx":0,"dy":-3,"x":903.0,"y":455.0}
{"t":2051.6,"k":"cursor","id":3,"hx":4,"hy":2,"w":32,"h":32}
{"t":3000,"k":"hologram","z":1}
{"t":5000.1,"k":"visible","v":false}
{"t":6000,"k":"mo"#;
        let log = EventLog::read(text.as_bytes()).unwrap();
        assert_eq!(log.header.width, 2560);
        assert_eq!(log.events.len(), 7, "unknown kind and truncated line are skipped");
        assert_eq!(log.events[3].kind, EventKind::Key);
        assert!(matches!(log.events[1].kind, EventKind::Down { b: Button::Left, inferred: false, .. }));
    }

    #[test]
    fn writer_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let header = Header::new(100, 100, "exact", "metadata");
        let mut w = EventWriter::create(&path, &header).unwrap();
        let ev = Event { t: 1.5, kind: EventKind::Down { b: Button::Left, x: 1.0, y: 2.0, inferred: true } };
        w.write(&ev).unwrap();
        w.write(&Event { t: 2.0, kind: EventKind::Key }).unwrap();
        w.flush().unwrap();
        let log = EventLog::load(&path).unwrap();
        assert_eq!(log.header, header);
        assert_eq!(log.events[0], ev);
        assert!(std::fs::read_to_string(&path).unwrap().contains(r#"{"t":2.0,"k":"key"}"#));
    }
}
