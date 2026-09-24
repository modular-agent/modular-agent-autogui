use std::sync::{Mutex, Once};
use std::thread;
use std::time::Duration;

use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use modular_agent_core::{Error, Result, Value};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Action {
    Move {
        x: f64,
        y: f64,
    },
    Click {
        x: Option<f64>,
        y: Option<f64>,
        #[serde(default)]
        button: MouseButton,
        #[serde(default = "one")]
        clicks: u32,
    },
    Down {
        #[serde(default)]
        button: MouseButton,
    },
    Up {
        #[serde(default)]
        button: MouseButton,
    },
    Drag {
        x: f64,
        y: f64,
        #[serde(default)]
        button: MouseButton,
    },
    Scroll {
        #[serde(default)]
        dx: i32,
        #[serde(default)]
        dy: i32,
    },
    Type {
        text: String,
    },
    Key {
        key: String,
    },
    KeyDown {
        key: String,
    },
    KeyUp {
        key: String,
    },
    Hotkey {
        keys: Vec<String>,
    },
    Wait {
        ms: u64,
    },
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

impl From<MouseButton> for Button {
    fn from(b: MouseButton) -> Self {
        match b {
            MouseButton::Left => Button::Left,
            MouseButton::Right => Button::Right,
            MouseButton::Middle => Button::Middle,
        }
    }
}

/// Accepts a single action object or an array of them.
pub(crate) fn parse_actions(value: &Value) -> Result<Vec<Action>> {
    let json = value.to_json();
    let actions = if json.is_array() {
        serde_json::from_value::<Vec<Action>>(json)
    } else {
        serde_json::from_value::<Action>(json).map(|a| vec![a])
    }
    .map_err(|e| Error::InvalidValue(format!("Invalid input action: {e}")))?;

    // Reject unknown key names before anything is sent, so a typo late in a
    // sequence does not leave it half-executed.
    for action in &actions {
        match action {
            Action::Key { key } | Action::KeyDown { key } | Action::KeyUp { key } => {
                parse_key(key)?;
            }
            Action::Hotkey { keys } => {
                for key in keys {
                    parse_key(key)?;
                }
            }
            _ => {}
        }
    }
    Ok(actions)
}

/// Parses a hotkey string such as `"ctrl+shift+esc"`.
pub(crate) fn parse_hotkey(s: &str) -> Result<Action> {
    let keys: Vec<String> = s
        .split('+')
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();
    if keys.is_empty() {
        return Err(Error::InvalidConfig("keys is empty".to_string()));
    }
    for key in &keys {
        parse_key(key)?;
    }
    Ok(Action::Hotkey { keys })
}

/// Maps pyautogui-style key names to enigo keys. A single character is typed
/// as that character.
pub(crate) fn parse_key(name: &str) -> Result<Key> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Ok(Key::Unicode(c));
    }
    let key = match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Key::Control,
        "shift" => Key::Shift,
        "alt" | "option" => Key::Alt,
        "win" | "cmd" | "command" | "meta" | "super" => Key::Meta,
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "space" => Key::Space,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "capslock" => Key::CapsLock,
        #[cfg(not(target_os = "macos"))]
        "insert" => Key::Insert,
        #[cfg(not(target_os = "macos"))]
        "printscreen" | "prtsc" => Key::PrintScr,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        _ => return Err(Error::InvalidValue(format!("Unknown key: {name}"))),
    };
    Ok(key)
}

pub(crate) struct Options {
    pub pause: Duration,
    pub failsafe: bool,
    /// Coordinates in actions are divided by this to get screen pixels, so it
    /// matches the `scale` of the screen capture the coordinates came from.
    pub scale: f64,
}

/// Runs `f` with a fresh `Enigo` on a blocking thread.
///
/// Enigo is synchronous and not `Send` on every platform, so it is created
/// on the thread that uses it. The lock keeps actions from concurrent
/// modules from interleaving.
pub(crate) async fn with_enigo<T, F>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&mut Enigo) -> Result<T> + Send + 'static,
{
    static LOCK: Mutex<()> = Mutex::new(());

    tokio::task::spawn_blocking(move || {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        init_dpi_awareness();
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| Error::Other(format!("Failed to initialize input: {e}")))?;
        f(&mut enigo)
    })
    .await
    .map_err(|e| Error::Other(format!("Input task failed: {e}")))?
}

// Without DPI awareness Windows hands enigo logical coordinates on a scaled
// display, while Screen Capture (xcap) captures physical pixels. The desktop
// app is already DPI aware via its manifest, so the call fails there and the
// error is ignored.
fn init_dpi_awareness() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        #[cfg(target_os = "windows")]
        let _ = enigo::set_dpi_awareness();
    });
}

pub(crate) fn run(enigo: &mut Enigo, actions: &[Action], opts: &Options) -> Result<()> {
    for (i, action) in actions.iter().enumerate() {
        if i > 0 && !opts.pause.is_zero() {
            thread::sleep(opts.pause);
        }
        if opts.failsafe {
            check_failsafe(enigo)?;
        }
        run_one(enigo, action, opts)?;
    }
    Ok(())
}

fn check_failsafe(enigo: &Enigo) -> Result<()> {
    let (x, y) = enigo.location().map_err(input_error)?;
    let (w, h) = enigo.main_display().map_err(input_error)?;
    let at_edge_x = x <= 0 || x >= w - 1;
    let at_edge_y = y <= 0 || y >= h - 1;
    if at_edge_x && at_edge_y {
        return Err(Error::Other(
            "Fail-safe triggered: the mouse is in a screen corner".to_string(),
        ));
    }
    Ok(())
}

fn run_one(enigo: &mut Enigo, action: &Action, opts: &Options) -> Result<()> {
    let to_screen = |v: f64| (v / opts.scale).round() as i32;
    match action {
        Action::Move { x, y } => {
            move_to(enigo, to_screen(*x), to_screen(*y))?;
        }
        Action::Click {
            x,
            y,
            button,
            clicks,
        } => {
            if x.is_some() || y.is_some() {
                let (cx, cy) = enigo.location().map_err(input_error)?;
                let x = x.map_or(cx, to_screen);
                let y = y.map_or(cy, to_screen);
                move_to(enigo, x, y)?;
            }
            for _ in 0..*clicks {
                enigo
                    .button((*button).into(), Direction::Click)
                    .map_err(input_error)?;
            }
        }
        Action::Down { button } => {
            enigo
                .button((*button).into(), Direction::Press)
                .map_err(input_error)?;
        }
        Action::Up { button } => {
            enigo
                .button((*button).into(), Direction::Release)
                .map_err(input_error)?;
        }
        Action::Drag { x, y, button } => {
            // Short pauses let the target application register the press
            // before the move, and the move before the release.
            let step = Duration::from_millis(50);
            enigo
                .button((*button).into(), Direction::Press)
                .map_err(input_error)?;
            thread::sleep(step);
            move_to(enigo, to_screen(*x), to_screen(*y))?;
            thread::sleep(step);
            enigo
                .button((*button).into(), Direction::Release)
                .map_err(input_error)?;
        }
        Action::Scroll { dx, dy } => {
            if *dx != 0 {
                enigo.scroll(*dx, Axis::Horizontal).map_err(input_error)?;
            }
            if *dy != 0 {
                enigo.scroll(*dy, Axis::Vertical).map_err(input_error)?;
            }
        }
        Action::Type { text } => {
            enigo.text(text).map_err(input_error)?;
        }
        Action::Key { key } => {
            enigo
                .key(parse_key(key)?, Direction::Click)
                .map_err(input_error)?;
        }
        Action::KeyDown { key } => {
            enigo
                .key(parse_key(key)?, Direction::Press)
                .map_err(input_error)?;
        }
        Action::KeyUp { key } => {
            enigo
                .key(parse_key(key)?, Direction::Release)
                .map_err(input_error)?;
        }
        Action::Hotkey { keys } => {
            let keys = keys
                .iter()
                .map(|k| parse_key(k))
                .collect::<Result<Vec<_>>>()?;
            for key in &keys {
                enigo.key(*key, Direction::Press).map_err(input_error)?;
            }
            for key in keys.iter().rev() {
                enigo.key(*key, Direction::Release).map_err(input_error)?;
            }
        }
        Action::Wait { ms } => {
            thread::sleep(Duration::from_millis(*ms));
        }
    }
    Ok(())
}

fn move_to(enigo: &mut Enigo, x: i32, y: i32) -> Result<()> {
    enigo.move_mouse(x, y, Coordinate::Abs).map_err(input_error)
}

fn input_error(e: enigo::InputError) -> Error {
    Error::Other(format!("Input failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: serde_json::Value) -> Result<Vec<Action>> {
        parse_actions(&Value::from_json(json)?)
    }

    #[test]
    fn single_and_sequence() {
        let one = parse(serde_json::json!({"action": "move", "x": 10, "y": 20})).unwrap();
        assert_eq!(one, vec![Action::Move { x: 10.0, y: 20.0 }]);

        let seq = parse(serde_json::json!([
            {"action": "click"},
            {"action": "type", "text": "hi"},
        ]))
        .unwrap();
        assert_eq!(
            seq,
            vec![
                Action::Click {
                    x: None,
                    y: None,
                    button: MouseButton::Left,
                    clicks: 1
                },
                Action::Type {
                    text: "hi".to_string()
                },
            ]
        );
    }

    #[test]
    fn rejects_unknown_action_and_key() {
        assert!(parse(serde_json::json!({"action": "teleport"})).is_err());
        assert!(parse(serde_json::json!({"action": "move", "x": 1})).is_err());
        assert!(
            parse(serde_json::json!([
                {"action": "type", "text": "a"},
                {"action": "key", "key": "entr"},
            ]))
            .is_err()
        );
    }

    #[test]
    fn key_names() {
        assert_eq!(parse_key("Ctrl").unwrap(), Key::Control);
        assert_eq!(parse_key("a").unwrap(), Key::Unicode('a'));
        assert_eq!(parse_key("あ").unwrap(), Key::Unicode('あ'));
        assert!(parse_key("hyper").is_err());
    }

    #[test]
    fn hotkey_string() {
        assert_eq!(
            parse_hotkey(" ctrl + shift+esc ").unwrap(),
            Action::Hotkey {
                keys: vec!["ctrl".into(), "shift".into(), "esc".into()]
            }
        );
        assert!(parse_hotkey("+").is_err());
        assert!(parse_hotkey("ctrl+nope").is_err());
    }
}
