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
        #[serde(default)]
        relative: bool,
        #[serde(default)]
        duration_ms: u64,
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
        #[serde(default)]
        relative: bool,
        #[serde(default)]
        duration_ms: u64,
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
        #[serde(default = "one")]
        presses: u32,
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

impl MouseButton {
    pub(crate) fn parse(name: &str) -> Result<Self> {
        serde_json::from_value(serde_json::Value::String(name.to_string()))
            .map_err(|_| Error::InvalidConfig(format!("Unknown mouse button: {name}")))
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
            Action::Key { key, .. } | Action::KeyDown { key } | Action::KeyUp { key } => {
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
        "ctrlleft" => Key::LControl,
        "ctrlright" => Key::RControl,
        "shift" => Key::Shift,
        "shiftleft" => Key::LShift,
        "shiftright" => Key::RShift,
        "alt" | "option" => Key::Alt,
        #[cfg(not(target_os = "macos"))]
        "altleft" => Key::LMenu,
        #[cfg(target_os = "windows")]
        "altright" => Key::RMenu,
        #[cfg(target_os = "macos")]
        "optionleft" => Key::Option,
        #[cfg(target_os = "macos")]
        "optionright" => Key::ROption,
        "win" | "cmd" | "command" | "meta" | "super" => Key::Meta,
        #[cfg(target_os = "windows")]
        "winleft" => Key::LWin,
        #[cfg(target_os = "windows")]
        "winright" => Key::RWin,
        #[cfg(target_os = "macos")]
        "cmdright" => Key::RCommand,
        #[cfg(target_os = "windows")]
        "apps" => Key::Apps,
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
        "numlock" => Key::Numlock,
        #[cfg(target_os = "windows")]
        "scrolllock" => Key::Scroll,
        #[cfg(all(unix, not(target_os = "macos")))]
        "scrolllock" => Key::ScrollLock,
        #[cfg(not(target_os = "macos"))]
        "pause" => Key::Pause,
        #[cfg(not(target_os = "macos"))]
        "insert" => Key::Insert,
        #[cfg(not(target_os = "macos"))]
        "printscreen" | "prtsc" | "prtscr" | "prntscrn" => Key::PrintScr,
        "num0" => Key::Numpad0,
        "num1" => Key::Numpad1,
        "num2" => Key::Numpad2,
        "num3" => Key::Numpad3,
        "num4" => Key::Numpad4,
        "num5" => Key::Numpad5,
        "num6" => Key::Numpad6,
        "num7" => Key::Numpad7,
        "num8" => Key::Numpad8,
        "num9" => Key::Numpad9,
        "add" => Key::Add,
        "subtract" => Key::Subtract,
        "multiply" => Key::Multiply,
        "divide" => Key::Divide,
        "decimal" => Key::Decimal,
        #[cfg(target_os = "windows")]
        "separator" => Key::Separator,
        "volumeup" => Key::VolumeUp,
        "volumedown" => Key::VolumeDown,
        "volumemute" => Key::VolumeMute,
        "playpause" => Key::MediaPlayPause,
        "nexttrack" => Key::MediaNextTrack,
        "prevtrack" => Key::MediaPrevTrack,
        #[cfg(not(target_os = "macos"))]
        "stop" => Key::MediaStop,
        #[cfg(not(target_os = "macos"))]
        "kanji" => Key::Kanji,
        #[cfg(target_os = "windows")]
        "kana" => Key::Kana,
        #[cfg(target_os = "windows")]
        "convert" => Key::Convert,
        #[cfg(target_os = "windows")]
        "nonconvert" => Key::NonConvert,
        #[cfg(target_os = "windows")]
        "imeon" => Key::IMEOn,
        #[cfg(target_os = "windows")]
        "imeoff" => Key::IMEOff,
        #[cfg(not(target_os = "macos"))]
        "hangul" | "hanguel" => Key::Hangul,
        #[cfg(not(target_os = "macos"))]
        "hanja" => Key::Hanja,
        #[cfg(target_os = "windows")]
        "junja" => Key::Junja,
        #[cfg(target_os = "windows")]
        "final" => Key::Final,
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
        "f13" => Key::F13,
        "f14" => Key::F14,
        "f15" => Key::F15,
        "f16" => Key::F16,
        "f17" => Key::F17,
        "f18" => Key::F18,
        "f19" => Key::F19,
        "f20" => Key::F20,
        #[cfg(not(target_os = "macos"))]
        "f21" => Key::F21,
        #[cfg(not(target_os = "macos"))]
        "f22" => Key::F22,
        #[cfg(not(target_os = "macos"))]
        "f23" => Key::F23,
        #[cfg(not(target_os = "macos"))]
        "f24" => Key::F24,
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

    run_blocking(move || {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| Error::Other(format!("Failed to initialize input: {e}")))?;
        f(&mut enigo)
    })
    .await
}

/// Runs `f` on a blocking thread made DPI aware, so screen coordinates read
/// there are physical pixels.
pub(crate) async fn run_blocking<T, F>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        init_dpi_awareness();
        f()
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
    // Relative moves are resolved to an absolute target here rather than sent
    // as `Coordinate::Rel`, which Windows scales by the mouse acceleration
    // setting. The target is checked before any button is pressed, so a drag
    // does not fail with the button held.
    let target = |enigo: &Enigo, x: f64, y: f64, relative: bool| {
        let (x, y) = if relative {
            let (cx, cy) = enigo.location().map_err(input_error)?;
            (cx + to_screen(x), cy + to_screen(y))
        } else {
            (to_screen(x), to_screen(y))
        };
        ensure_on_screen(enigo, x, y)?;
        Ok((x, y))
    };
    match action {
        Action::Move {
            x,
            y,
            relative,
            duration_ms,
        } => {
            let to = target(enigo, *x, *y, *relative)?;
            glide(enigo, to, Duration::from_millis(*duration_ms))?;
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
                ensure_on_screen(enigo, x, y)?;
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
        Action::Drag {
            x,
            y,
            button,
            relative,
            duration_ms,
        } => {
            let to = target(enigo, *x, *y, *relative)?;
            // Short pauses let the target application register the press
            // before the move, and the move before the release.
            let step = Duration::from_millis(50);
            enigo
                .button((*button).into(), Direction::Press)
                .map_err(input_error)?;
            thread::sleep(step);
            glide(enigo, to, Duration::from_millis(*duration_ms))?;
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
        Action::Key { key, presses } => {
            let key = parse_key(key)?;
            for _ in 0..*presses {
                enigo.key(key, Direction::Click).map_err(input_error)?;
            }
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

/// enigo maps absolute coordinates onto the primary monitor only, so a point
/// elsewhere would land somewhere unrelated.
fn ensure_on_screen(enigo: &Enigo, x: i32, y: i32) -> Result<()> {
    let (w, h) = enigo.main_display().map_err(input_error)?;
    if !(0..w).contains(&x) || !(0..h).contains(&y) {
        return Err(Error::InvalidValue(format!(
            "({x}, {y}) is outside the primary monitor ({w}x{h})"
        )));
    }
    Ok(())
}

fn move_to(enigo: &mut Enigo, x: i32, y: i32) -> Result<()> {
    enigo.move_mouse(x, y, Coordinate::Abs).map_err(input_error)
}

const GLIDE_STEP: Duration = Duration::from_millis(10);

/// Moves to `to` in a straight line over `duration`, so the path passes
/// through intermediate positions as a hand-moved mouse would.
fn glide(enigo: &mut Enigo, to: (i32, i32), duration: Duration) -> Result<()> {
    if duration.is_zero() {
        return move_to(enigo, to.0, to.1);
    }
    let from = enigo.location().map_err(input_error)?;
    let steps = (duration.as_millis() / GLIDE_STEP.as_millis()).max(1) as u32;
    for (x, y) in glide_path(from, to, steps) {
        thread::sleep(GLIDE_STEP);
        move_to(enigo, x, y)?;
    }
    Ok(())
}

/// `steps` evenly spaced points after `from`, ending at `to`.
fn glide_path(from: (i32, i32), to: (i32, i32), steps: u32) -> impl Iterator<Item = (i32, i32)> {
    let lerp = move |a: i32, b: i32, i: u32| {
        (a as f64 + (b - a) as f64 * i as f64 / steps as f64).round() as i32
    };
    (1..=steps).map(move |i| (lerp(from.0, to.0, i), lerp(from.1, to.1, i)))
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
        assert_eq!(
            one,
            vec![Action::Move {
                x: 10.0,
                y: 20.0,
                relative: false,
                duration_ms: 0
            }]
        );

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
    fn move_options() {
        let actions = parse(serde_json::json!([
            {"action": "move", "x": 5, "y": -5, "relative": true, "duration_ms": 200},
            {"action": "drag", "x": 1, "y": 2},
            {"action": "key", "key": "tab", "presses": 3},
        ]))
        .unwrap();
        assert_eq!(
            actions,
            vec![
                Action::Move {
                    x: 5.0,
                    y: -5.0,
                    relative: true,
                    duration_ms: 200
                },
                Action::Drag {
                    x: 1.0,
                    y: 2.0,
                    button: MouseButton::Left,
                    relative: false,
                    duration_ms: 0
                },
                Action::Key {
                    key: "tab".to_string(),
                    presses: 3
                },
            ]
        );
    }

    #[test]
    fn glide_path_ends_at_target() {
        let path: Vec<_> = glide_path((0, 0), (10, -4), 4).collect();
        assert_eq!(path, vec![(3, -1), (5, -2), (8, -3), (10, -4)]);
        assert_eq!(
            glide_path((7, 7), (1, 2), 1).collect::<Vec<_>>(),
            vec![(1, 2)]
        );
    }

    #[test]
    fn mouse_button_names() {
        assert_eq!(MouseButton::parse("right").unwrap(), MouseButton::Right);
        assert!(MouseButton::parse("Right").is_err());
    }

    #[test]
    fn key_names() {
        assert_eq!(parse_key("Ctrl").unwrap(), Key::Control);
        assert_eq!(parse_key("a").unwrap(), Key::Unicode('a'));
        assert_eq!(parse_key("あ").unwrap(), Key::Unicode('あ'));
        assert_eq!(parse_key("num5").unwrap(), Key::Numpad5);
        assert_eq!(parse_key("f20").unwrap(), Key::F20);
        assert_eq!(parse_key("volumemute").unwrap(), Key::VolumeMute);
        #[cfg(target_os = "windows")]
        assert_eq!(parse_key("kanji").unwrap(), Key::Kanji);
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
