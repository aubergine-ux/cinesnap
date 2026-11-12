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
    /// A key went down. Typed characters are never recorded; `combo` is
    /// only present for shortcuts and navigation keys (`Ctrl+K`, `Enter`).
    Key {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        combo: Option<String>,
    },
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

/// Modifier keys held while a key goes down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

/// What a platform key name means for the keystroke overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyName {
    Ctrl,
    Alt,
    Shift,
    Meta,
    /// A key that types a character; the label is what is printed on it.
    Printing(String),
    /// A key that does something: Enter, Esc, arrows, function keys.
    Action(String),
    /// Not worth showing (lock keys, media keys, unknown codes).
    Other,
}

/// Classify a key from its platform name: the kernel's `KEY_LEFTCTRL` style
/// or the `LControl` style used on the other platforms.
pub fn key_name(raw: &str) -> KeyName {
    let upper = raw.trim().to_ascii_uppercase();
    let name = upper.strip_prefix("KEY_").unwrap_or(&upper);
    let action = |s: &str| KeyName::Action(s.into());
    let printing = |s: &str| KeyName::Printing(s.into());
    match name {
        "LEFTCTRL" | "RIGHTCTRL" | "LCONTROL" | "RCONTROL" => KeyName::Ctrl,
        "LEFTALT" | "RIGHTALT" | "LALT" | "RALT" | "LOPTION" | "ROPTION" => KeyName::Alt,
        "LEFTSHIFT" | "RIGHTSHIFT" | "LSHIFT" | "RSHIFT" => KeyName::Shift,
        "LEFTMETA" | "RIGHTMETA" | "LMETA" | "RMETA" | "COMMAND" | "RCOMMAND" => KeyName::Meta,
        "ENTER" | "KPENTER" | "NUMPADENTER" => action("Enter"),
        "ESC" | "ESCAPE" => action("Esc"),
        "TAB" => action("Tab"),
        "BACKSPACE" => action("Backspace"),
        "DELETE" => action("Delete"),
        "INSERT" => action("Insert"),
        "SPACE" => action("Space"),
        "UP" => action("Up"),
        "DOWN" => action("Down"),
        "LEFT" => action("Left"),
        "RIGHT" => action("Right"),
        "HOME" => action("Home"),
        "END" => action("End"),
        "PAGEUP" => action("PageUp"),
        "PAGEDOWN" => action("PageDown"),
        "MINUS" => printing("-"),
        "EQUAL" => printing("="),
        "COMMA" => printing(","),
        "DOT" => printing("."),
        "SLASH" => printing("/"),
        "SEMICOLON" => printing(";"),
        "APOSTROPHE" => printing("'"),
        "LEFTBRACE" | "LEFTBRACKET" => printing("["),
        "RIGHTBRACE" | "RIGHTBRACKET" => printing("]"),
        "BACKSLASH" => printing("\\"),
        "GRAVE" => printing("`"),
        _ => {
            let digit = name.strip_prefix("KEY").unwrap_or(name);
            if name.len() == 1 && name.chars().all(|c| c.is_ascii_alphanumeric()) {
                printing(name)
            } else if digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit()) {
                printing(digit)
            } else if name.len() <= 3
                && name.starts_with('F')
                && name[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n))
            {
                action(name)
            } else {
                KeyName::Other
            }
        }
    }
}

/// The label to show for a key press, or `None` when it must not be
/// recorded. Only shortcuts (a key with Ctrl, Alt or the OS key held) and
/// action keys are kept, so nothing that is typed — passwords included —
/// ever reaches the log.
pub fn shortcut(mods: Modifiers, key: &KeyName) -> Option<String> {
    let chord = mods.ctrl || mods.alt || mods.meta;
    let label = match key {
        KeyName::Printing(k) if chord => k,
        // Space and Backspace on their own are typing.
        KeyName::Action(k) if chord || !matches!(k.as_str(), "Space" | "Backspace") => k,
        _ => return None,
    };
    let meta = if cfg!(target_os = "macos") {
        "Cmd"
    } else if cfg!(windows) {
        "Win"
    } else {
        "Super"
    };
    let mut parts = Vec::new();
    for (held, name) in [
        (mods.ctrl, "Ctrl"),
        (mods.alt, "Alt"),
        (mods.shift, "Shift"),
        (mods.meta, meta),
    ] {
        if held {
            parts.push(name);
        }
    }
    parts.push(label);
    Some(parts.join("+"))
}

#[derive(Clone, Debug)]
pub struct EventLog {
    pub header: Header,
    pub events: Vec<Event>,
}

impl EventLog {
    pub fn empty(width: u32, height: u32) -> Self {
        Self {
            header: Header::new(width, height, "none", "none"),
            events: vec![],
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::read(BufReader::new(file))
    }

    pub fn read(reader: impl BufRead) -> Result<Self> {
        let mut lines = reader.lines();
        let first = lines.next().context("event log is empty")??;
        let header: Header = serde_json::from_str(&first).context("parsing event log header")?;
        anyhow::ensure!(
            header.format == "cinesnap-events",
            "not a Cinesnap event log"
        );
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

/// Guess clicks from cursor movement alone, for sessions where button state
/// is not available (Wayland without access to input devices).
///
/// People move to a target, stop, click, and move on. A candidate is a stop
/// of at least `DWELL_MS` that follows a deliberate move; the click is placed
/// shortly after the cursor comes to rest. Returns `down` events marked
/// `inferred`.
pub fn infer_clicks(events: &[Event]) -> Vec<Event> {
    const DWELL_MS: f64 = 350.0;
    const STILL_PX: f64 = 6.0;
    const TRAVEL_PX: f64 = 80.0;
    const CLICK_DELAY_MS: f64 = 120.0;

    let moves: Vec<(f64, f64, f64)> = events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::Move { x, y } => Some((e.t, x, y)),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut travelled = 0.0;
    while i < moves.len() {
        let (t0, x0, y0) = moves[i];
        // Extend the rest period while the cursor stays near where it stopped.
        let mut j = i;
        while j + 1 < moves.len() && (moves[j + 1].1 - x0).hypot(moves[j + 1].2 - y0) <= STILL_PX {
            j += 1;
        }
        // The rest lasts until the next sample that leaves the spot.
        let rest_end = moves.get(j + 1).map_or(moves[j].0, |m| m.0);
        if rest_end - t0 >= DWELL_MS && travelled >= TRAVEL_PX {
            out.push(Event {
                t: t0 + CLICK_DELAY_MS,
                kind: EventKind::Down {
                    b: Button::Left,
                    x: x0,
                    y: y0,
                    inferred: true,
                },
            });
            travelled = 0.0;
        }
        if let Some(next) = moves.get(j + 1) {
            travelled += (next.1 - x0).hypot(next.2 - y0);
        }
        i = j + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_shortcuts_and_action_keys_are_labelled() {
        let none = Modifiers::default();
        let ctrl = Modifiers { ctrl: true, ..none };
        let shift = Modifiers {
            shift: true,
            ..none
        };
        let label = |m: Modifiers, raw: &str| shortcut(m, &key_name(raw));
        // Typing is never recorded, with or without Shift.
        for raw in [
            "KEY_A",
            "a",
            "Key7",
            "KEY_7",
            "KEY_SPACE",
            "Backspace",
            "KEY_DOT",
        ] {
            assert_eq!(label(none, raw), None, "{raw}");
            assert_eq!(label(shift, raw), None, "{raw}");
        }
        assert_eq!(label(ctrl, "KEY_K").as_deref(), Some("Ctrl+K"));
        assert_eq!(label(ctrl, "Key1").as_deref(), Some("Ctrl+1"));
        assert_eq!(label(ctrl, "Slash").as_deref(), Some("Ctrl+/"));
        assert_eq!(label(ctrl, "Space").as_deref(), Some("Ctrl+Space"));
        assert_eq!(
            label(
                Modifiers {
                    ctrl: true,
                    alt: true,
                    shift: true,
                    meta: false
                },
                "P"
            )
            .as_deref(),
            Some("Ctrl+Alt+Shift+P")
        );
        assert_eq!(label(none, "KEY_ENTER").as_deref(), Some("Enter"));
        assert_eq!(label(none, "Escape").as_deref(), Some("Esc"));
        assert_eq!(label(shift, "KEY_TAB").as_deref(), Some("Shift+Tab"));
        assert_eq!(label(none, "F5").as_deref(), Some("F5"));
        assert_eq!(label(none, "KEY_F12").as_deref(), Some("F12"));
        // Modifiers on their own, and keys with no sensible label.
        for raw in [
            "KEY_LEFTCTRL",
            "LShift",
            "KEY_CAPSLOCK",
            "KEY_VOLUMEUP",
            "F99",
            "",
        ] {
            assert_eq!(label(ctrl, raw), None, "{raw}");
        }
    }

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
        assert_eq!(
            log.events.len(),
            7,
            "unknown kind and truncated line are skipped"
        );
        assert_eq!(log.events[3].kind, EventKind::Key { combo: None });
        assert!(matches!(
            log.events[1].kind,
            EventKind::Down {
                b: Button::Left,
                inferred: false,
                ..
            }
        ));
    }

    #[test]
    fn writer_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let header = Header::new(100, 100, "exact", "metadata");
        let mut w = EventWriter::create(&path, &header).unwrap();
        let ev = Event {
            t: 1.5,
            kind: EventKind::Down {
                b: Button::Left,
                x: 1.0,
                y: 2.0,
                inferred: true,
            },
        };
        w.write(&ev).unwrap();
        w.write(&Event {
            t: 2.0,
            kind: EventKind::Key { combo: None },
        })
        .unwrap();
        w.flush().unwrap();
        let log = EventLog::load(&path).unwrap();
        assert_eq!(log.header, header);
        assert_eq!(log.events[0], ev);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains(r#"{"t":2.0,"k":"key"}"#));
    }

    #[test]
    fn infers_clicks_from_move_stop_patterns() {
        let mv = |t: f64, x: f64, y: f64| Event {
            t,
            kind: EventKind::Move { x, y },
        };
        let mut ev = vec![mv(0.0, 0.0, 0.0)];
        // Sweep to (400, 300) over 400 ms, rest 600 ms, tiny wobble, sweep on
        // without a long stop, then a final long rest.
        ev.extend((1..=40).map(|i| mv(i as f64 * 10.0, i as f64 * 10.0, i as f64 * 7.5)));
        ev.push(mv(700.0, 402.0, 301.0));
        ev.extend((1..=30).map(|i| mv(1000.0 + i as f64 * 10.0, 400.0 + i as f64 * 15.0, 300.0)));
        ev.push(mv(1400.0, 900.0, 320.0)); // only a 100 ms pause at (850, 300)
        ev.push(mv(2400.0, 900.0, 321.0));
        let clicks = infer_clicks(&ev);
        assert_eq!(clicks.len(), 2, "{clicks:?}");
        assert_eq!(clicks[0].t, 520.0);
        assert!(
            matches!(clicks[0].kind, EventKind::Down { x, y, inferred: true, .. } if x == 400.0 && y == 300.0)
        );
        assert_eq!(clicks[1].t, 1520.0);
        // A cursor that never travels far produces nothing.
        assert!(infer_clicks(&[
            mv(0.0, 5.0, 5.0),
            mv(2000.0, 6.0, 5.0),
            mv(4000.0, 5.0, 6.0)
        ])
        .is_empty());
    }
}
