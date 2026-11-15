//! Global hotkeys: start and stop, pause and resume, and markers.
//!
//! The global-shortcut plugin covers Windows, macOS and X11. On Wayland only
//! the compositor can grab keys; there the GlobalShortcuts portal asks the
//! desktop to bind them, and the desktop's own settings decide the keys.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Pause,
    Marker,
}

impl Action {
    pub const ALL: [Action; 3] = [Self::Toggle, Self::Pause, Self::Marker];

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn id(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::Pause => "pause",
            Self::Marker => "marker",
        }
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn description(self) -> &'static str {
        match self {
            Self::Toggle => "Start or stop recording",
            Self::Pause => "Pause or resume recording",
            Self::Marker => "Mark this moment",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Bindings {
    pub toggle: String,
    pub pause: String,
    pub marker: String,
}

impl Default for Bindings {
    fn default() -> Self {
        Self {
            toggle: "CmdOrCtrl+Alt+R".into(),
            pause: "CmdOrCtrl+Alt+P".into(),
            marker: "CmdOrCtrl+Alt+M".into(),
        }
    }
}

impl Bindings {
    pub fn from_settings(settings: &serde_json::Value) -> Self {
        serde_json::from_value(settings["hotkeys"].clone()).unwrap_or_default()
    }

    fn get(&self, a: Action) -> &str {
        match a {
            Action::Toggle => &self.toggle,
            Action::Pause => &self.pause,
            Action::Marker => &self.marker,
        }
    }

    /// The bindings as the event log writes shortcuts, so pressing them
    /// does not show up in the keystroke overlay.
    pub fn labels(&self) -> Vec<String> {
        Action::ALL.iter().map(|&a| label(self.get(a))).collect()
    }
}

fn split(accel: &str) -> (Vec<String>, String) {
    let parts: Vec<&str> = accel
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let Some((key, mods)) = parts.split_last() else {
        return (vec![], String::new());
    };
    let key = key
        .strip_prefix("Key")
        .filter(|k| k.len() == 1)
        .unwrap_or(key);
    let mods = mods
        .iter()
        .map(|m| match m.to_ascii_lowercase().as_str() {
            "cmdorctrl" | "commandorcontrol" if cfg!(target_os = "macos") => "Meta",
            "cmdorctrl" | "commandorcontrol" | "ctrl" | "control" => "Ctrl",
            "alt" | "option" => "Alt",
            "shift" => "Shift",
            _ => "Meta",
        })
        .map(String::from)
        .collect();
    (mods, key.to_uppercase())
}

/// `CmdOrCtrl+Alt+R` as `events::shortcut` writes it: `Ctrl+Alt+R`.
pub fn label(accel: &str) -> String {
    let (mods, key) = split(accel);
    let meta = if cfg!(target_os = "macos") {
        "Cmd"
    } else if cfg!(windows) {
        "Win"
    } else {
        "Super"
    };
    let mut parts: Vec<&str> = ["Ctrl", "Alt", "Shift", "Meta"]
        .into_iter()
        .filter(|m| mods.iter().any(|x| x == m))
        .map(|m| if m == "Meta" { meta } else { m })
        .collect();
    parts.push(&key);
    parts.join("+")
}

/// `CmdOrCtrl+Alt+R` in the XDG shortcuts notation: `CTRL+ALT+r`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn portal_trigger(accel: &str) -> String {
    let (mods, key) = split(accel);
    let mut parts: Vec<String> = mods
        .iter()
        .map(|m| match m.as_str() {
            "Meta" => "LOGO".to_owned(),
            m => m.to_uppercase(),
        })
        .collect();
    parts.push(if key.len() == 1 {
        key.to_lowercase()
    } else {
        key
    });
    parts.join("+")
}

pub type Handler = Arc<dyn Fn(Action) + Send + Sync>;

/// Register the bindings, replacing earlier ones. Returns a note for the
/// user when a binding could not be made.
pub fn install(app: &AppHandle, bindings: &Bindings, on: Handler) -> Vec<String> {
    #[cfg(target_os = "linux")]
    if crate::devices::is_wayland() {
        portal::bind_once(bindings, on);
        return vec![];
    }
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut notes = Vec::new();
    for action in Action::ALL {
        let accel = bindings.get(action);
        if accel.trim().is_empty() {
            continue;
        }
        let on = on.clone();
        let result = gs.on_shortcut(accel, move |_, _, event| {
            if event.state == ShortcutState::Pressed {
                on(action);
            }
        });
        if let Err(e) = result {
            notes.push(format!("{accel} could not be used as a hotkey: {e}"));
        }
    }
    notes
}

#[cfg(target_os = "linux")]
mod portal {
    use super::*;
    use std::sync::OnceLock;

    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;

    static BOUND: OnceLock<()> = OnceLock::new();

    /// Ask the desktop for the shortcuts once per run. After that it owns
    /// them: changing keys happens in its settings.
    pub fn bind_once(bindings: &Bindings, on: Handler) {
        if BOUND.set(()).is_err() {
            return;
        }
        let shortcuts: Vec<NewShortcut> = Action::ALL
            .iter()
            .map(|&a| {
                NewShortcut::new(a.id(), a.description())
                    .preferred_trigger(portal_trigger(bindings.get(a)).as_str())
            })
            .collect();
        std::thread::spawn(move || {
            let result: Result<(), ashpd::Error> = pollster::block_on(async {
                let proxy = GlobalShortcuts::new().await?;
                let session = proxy.create_session(Default::default()).await?;
                proxy
                    .bind_shortcuts(&session, &shortcuts, None, Default::default())
                    .await?
                    .response()?;
                let mut activated = proxy.receive_activated().await?;
                while let Some(a) = activated.next().await {
                    if let Some(&action) = Action::ALL.iter().find(|x| x.id() == a.shortcut_id()) {
                        on(action);
                    }
                }
                drop(session);
                Ok(())
            });
            if let Err(e) = result {
                eprintln!("hotkeys: the desktop did not bind them: {e}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_the_event_log() {
        let expected = if cfg!(target_os = "macos") {
            "Alt+Cmd+R"
        } else {
            "Ctrl+Alt+R"
        };
        assert_eq!(label("CmdOrCtrl+Alt+R"), expected);
        assert_eq!(label("Shift+Control+KeyM"), "Ctrl+Shift+M");
        assert_eq!(label("Alt+F9"), "Alt+F9");
    }

    #[test]
    fn portal_triggers_use_xdg_names() {
        assert_eq!(portal_trigger("Ctrl+Alt+R"), "CTRL+ALT+r");
        assert_eq!(portal_trigger("Super+Shift+F9"), "LOGO+SHIFT+F9");
    }

    #[test]
    fn bindings_come_from_settings_or_defaults() {
        let s = serde_json::json!({ "hotkeys": { "marker": "Ctrl+Shift+M" } });
        let b = Bindings::from_settings(&s);
        assert_eq!(b.marker, "Ctrl+Shift+M");
        assert_eq!(b.toggle, Bindings::default().toggle);
        assert_eq!(
            Bindings::from_settings(&serde_json::json!({})),
            Bindings::default()
        );
    }
}
