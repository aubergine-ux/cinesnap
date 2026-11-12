//! Input tracking: collecting cursor, button and key activity while
//! recording. Events are buffered with their capture instant and written to
//! the event log once the video's start time is known.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cinesnap_core::events::{Button, EventKind};

#[derive(Clone, Default)]
pub struct Sink(Arc<Mutex<Vec<(Instant, EventKind)>>>);

impl Sink {
    pub fn push(&self, kind: EventKind) {
        self.0.lock().unwrap().push((Instant::now(), kind));
    }
    pub fn take(&self) -> Vec<(Instant, EventKind)> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

/// Background input threads; stopped (and joined) on drop.
#[derive(Default)]
pub struct Tracker {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Tracker {
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }
    pub fn add(&mut self, thread: JoinHandle<()>) {
        self.threads.push(thread);
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// Maps a global pointer position to source-video pixels.
#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    pub origin: (f64, f64),
    pub scale: f64,
}

impl Mapping {
    fn apply(&self, x: i32, y: i32) -> (f64, f64) {
        (
            (x as f64 - self.origin.0) * self.scale,
            (y as f64 - self.origin.1) * self.scale,
        )
    }
}

/// Poll the global pointer, buttons and keyboard. Used on Windows, macOS and
/// X11, where the OS lets any process ask for this state.
///
/// Only the fact that a key went down is recorded, never which key.
pub fn poll(tracker: &mut Tracker, sink: Sink, map: Mapping) {
    use device_query::{DeviceQuery, DeviceState};
    let stop = tracker.stop_flag();
    tracker.add(std::thread::spawn(move || {
        let Some(device) = DeviceState::checked_new() else {
            return;
        };
        const BUTTONS: [(usize, Button); 3] =
            [(1, Button::Left), (2, Button::Right), (3, Button::Middle)];
        let mut last_pos = (i32::MIN, i32::MIN);
        let mut down = [false; 3];
        let mut keys = 0usize;
        let mut tick = 0u32;
        while !stop.load(Ordering::Relaxed) {
            let mouse = device.get_mouse();
            let (x, y) = map.apply(mouse.coords.0, mouse.coords.1);
            if mouse.coords != last_pos {
                last_pos = mouse.coords;
                sink.push(EventKind::Move { x, y });
            }
            for (i, (idx, b)) in BUTTONS.iter().enumerate() {
                let pressed = mouse.button_pressed.get(*idx).copied().unwrap_or(false);
                if pressed != down[i] {
                    down[i] = pressed;
                    sink.push(if pressed {
                        EventKind::Down {
                            b: *b,
                            x,
                            y,
                            inferred: false,
                        }
                    } else {
                        EventKind::Up { b: *b, x, y }
                    });
                }
            }
            // Keyboard state is comparatively expensive to query; 60 Hz is
            // plenty to notice typing.
            tick = tick.wrapping_add(1);
            if tick.is_multiple_of(4) {
                let n = device.get_keys().len();
                if n > keys {
                    sink.push(EventKind::Key);
                }
                keys = n;
            }
            std::thread::sleep(Duration::from_micros(4167));
        }
    }));
}
