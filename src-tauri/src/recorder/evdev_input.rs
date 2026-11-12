//! Button and key activity on Wayland, read from the kernel input devices.
//!
//! Wayland gives clients no global input, but `/dev/input/event*` is readable
//! by members of the `input` group. Devices are opened read-only and never
//! grabbed. Pointer *position* does not come from here (relative motion says
//! nothing about where the compositor put the cursor); it comes from the
//! screencast stream's cursor metadata and is shared through `cursor`.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cinesnap_core::events::{key_name, shortcut, Button, EventKind, KeyName, Modifiers};
use evdev::{Device, EventSummary, KeyCode, RelativeAxisCode};

use super::input::{Sink, Tracker};

fn interesting(dev: &Device) -> bool {
    dev.supported_keys()
        .is_some_and(|k| k.contains(KeyCode::BTN_LEFT) || k.contains(KeyCode::KEY_A))
}

fn devices() -> Vec<Device> {
    evdev::enumerate()
        .map(|(_, d)| d)
        .filter(interesting)
        .collect()
}

/// Whether button/key events can be observed in this session.
pub fn available() -> bool {
    let devs = devices();
    devs.iter().any(|d| {
        d.supported_keys()
            .is_some_and(|k| k.contains(KeyCode::BTN_LEFT))
    })
}

pub fn track(tracker: &mut Tracker, sink: Sink, cursor: Arc<Mutex<(f64, f64)>>, shortcuts: bool) {
    // Modifier state is shared: the modifier and the key may be on
    // different devices (a keyboard and a macro pad, say).
    let held = Arc::new(Mutex::new(Modifiers::default()));
    for mut dev in devices() {
        if dev.set_nonblocking(true).is_err() {
            continue;
        }
        let (stop, sink, cursor) = (tracker.stop_flag(), sink.clone(), cursor.clone());
        let held = held.clone();
        tracker.add(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let events: Vec<_> = match dev.fetch_events() {
                    Ok(ev) => ev.collect(),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(4));
                        continue;
                    }
                    Err(_) => return, // device unplugged
                };
                for ev in events {
                    let (x, y) = *cursor.lock().unwrap();
                    match ev.destructure() {
                        EventSummary::Key(_, code, value) => {
                            let button = match code {
                                KeyCode::BTN_LEFT => Some(Button::Left),
                                KeyCode::BTN_RIGHT => Some(Button::Right),
                                KeyCode::BTN_MIDDLE => Some(Button::Middle),
                                _ => None,
                            };
                            match (button, value) {
                                (Some(b), 1) => sink.push(EventKind::Down {
                                    b,
                                    x,
                                    y,
                                    inferred: false,
                                }),
                                (Some(b), 0) => sink.push(EventKind::Up { b, x, y }),
                                // Keyboard keys only; other BTN_* codes
                                // (touch, tool, gamepad) are not typing.
                                (None, value) if code.code() < 0x100 => {
                                    let name = key_name(&format!("{code:?}"));
                                    let mut mods = held.lock().unwrap();
                                    let down = value != 0;
                                    match name {
                                        KeyName::Ctrl => mods.ctrl = down,
                                        KeyName::Alt => mods.alt = down,
                                        KeyName::Shift => mods.shift = down,
                                        KeyName::Meta => mods.meta = down,
                                        // 1 is a press; 2 is auto-repeat.
                                        _ if value == 1 => sink.push(EventKind::Key {
                                            combo: shortcut(*mods, &name).filter(|_| shortcuts),
                                        }),
                                        _ => {}
                                    }
                                }
                                _ => {}
                            }
                        }
                        EventSummary::RelativeAxis(_, RelativeAxisCode::REL_WHEEL, v) => {
                            sink.push(EventKind::Scroll {
                                dx: 0.0,
                                dy: -(v as f64),
                                x,
                                y,
                            })
                        }
                        EventSummary::RelativeAxis(_, RelativeAxisCode::REL_HWHEEL, v) => sink
                            .push(EventKind::Scroll {
                                dx: v as f64,
                                dy: 0.0,
                                x,
                                y,
                            }),
                        _ => {}
                    }
                }
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_key_names_are_understood() {
        let name = |code: KeyCode| key_name(&format!("{code:?}"));
        assert_eq!(name(KeyCode::KEY_LEFTCTRL), KeyName::Ctrl);
        assert_eq!(name(KeyCode::KEY_RIGHTMETA), KeyName::Meta);
        assert_eq!(name(KeyCode::KEY_K), KeyName::Printing("K".into()));
        assert_eq!(name(KeyCode::KEY_ENTER), KeyName::Action("Enter".into()));
        assert_eq!(name(KeyCode::KEY_F5), KeyName::Action("F5".into()));
    }
}
