use std::time::Duration;

use enigo::Mouse;
use modular_agent_core::im::hashmap;
use modular_agent_core::{
    AsModule, Error, ModularAgent, Module, ModuleContext, ModuleData, ModuleOutput, ModuleSpec,
    Result, Value, async_trait, modular_agent,
};
use serde::Deserialize;

use crate::action::{self, Action, MouseButton, Options};
use crate::window::{self, Placement, Query, WindowInfo};

static CATEGORY_KEYBOARD: &str = "AutoGUI/Keyboard";
static CATEGORY_MOUSE: &str = "AutoGUI/Mouse";
static CATEGORY_WINDOW: &str = "AutoGUI/Window";
static CATEGORY: &str = "AutoGUI";

static PORT_ACTION: &str = "action";
static PORT_TEXT: &str = "text";
static PORT_UNIT: &str = "unit";
static PORT_DONE: &str = "done";
static PORT_POSITION: &str = "position";
static PORT_WINDOW: &str = "window";

static CONFIG_PAUSE_MS: &str = "pause_ms";
static CONFIG_INTERVAL_MS: &str = "interval_ms";
static CONFIG_KEYS: &str = "keys";
static CONFIG_SCALE: &str = "scale";
static CONFIG_FAILSAFE: &str = "failsafe";
static CONFIG_DURATION_MS: &str = "duration_ms";
static CONFIG_RELATIVE: &str = "relative";
static CONFIG_BUTTON: &str = "button";
static CONFIG_CLICKS: &str = "clicks";
static CONFIG_X: &str = "x";
static CONFIG_Y: &str = "y";
static CONFIG_ANCHOR: &str = "anchor";
static CONFIG_TITLE: &str = "title";
static CONFIG_PROCESS_NAME: &str = "process_name";
static CONFIG_TIMEOUT_MS: &str = "timeout_ms";
static CONFIG_ACTIVATE: &str = "activate";
static CONFIG_WIDTH: &str = "width";
static CONFIG_HEIGHT: &str = "height";

fn failsafe(module: &impl Module) -> Result<bool> {
    Ok(module.configs()?.get_bool_or(CONFIG_FAILSAFE, true))
}

fn scale(module: &impl Module) -> Result<f64> {
    let scale = module.configs()?.get_number_or(CONFIG_SCALE, 1.0);
    if scale <= 0.0 {
        return Err(Error::InvalidConfig("scale must be positive".to_string()));
    }
    Ok(scale)
}

async fn run_actions(actions: Vec<Action>, opts: Options) -> Result<()> {
    action::with_enigo(move |enigo| action::run(enigo, &actions, &opts)).await
}

/// Sends keyboard and mouse actions to the operating system.
///
/// Each input is one action object or an array of them, run in order. The
/// whole input is checked before anything is sent: an unknown action or key
/// name fails without moving the mouse or pressing a key.
///
/// Actions (`x`/`y` of `move` and `drag` are required, other fields optional):
/// - `{"action": "move", "x", "y", "relative", "duration_ms"}`: Move the mouse
/// - `{"action": "click", "x", "y", "button", "clicks"}`: Click, first moving to `x`/`y` when given
/// - `{"action": "down" | "up", "button"}`: Press or release a mouse button
/// - `{"action": "drag", "x", "y", "button", "relative", "duration_ms"}`: Press, move to `x`/`y`, release
/// - `{"action": "scroll", "dx", "dy"}`: Scroll by wheel clicks; positive `dy` scrolls down
/// - `{"action": "type", "text"}`: Type text
/// - `{"action": "key", "key", "presses"}`: Press and release a key `presses` times (default 1)
/// - `{"action": "key_down" | "key_up", "key"}`: Press or release a key
/// - `{"action": "hotkey", "keys"}`: Press `keys` in order, release in reverse
/// - `{"action": "wait", "ms"}`: Wait
///
/// `relative: true` moves by `x`/`y` from the current position. `duration_ms`
/// moves in a straight line over that time instead of jumping, for menus that
/// open on hover and applications that ignore an instant drag.
///
/// `button` is `"left"` (default), `"right"` or `"middle"`. Key names follow
/// pyautogui: `ctrl`, `shift`, `alt`, `win`/`cmd` (with `ctrlleft`,
/// `shiftright`, … for one side), `enter`, `tab`, `esc`, `backspace`,
/// `delete`, `space`, `up`/`down`/`left`/`right`, `home`, `end`, `pageup`,
/// `pagedown`, `f1`–`f24`, `num0`–`num9`, `volumeup`, `playpause`, `kanji`,
/// `convert`, `nonconvert`, or any single character. Some names exist only on
/// some platforms.
///
/// Coordinates are pixels on the primary monitor; a point outside it fails
/// before anything is sent. Keys still held by
/// `key_down` are released when the input finishes. Windows does not deliver
/// input to windows of applications running as administrator unless this app
/// runs as administrator too; on macOS the app needs the Accessibility
/// permission.
///
/// # Ports
/// - Input `action`: An action object or an array of action objects
/// - Output `done`: The input value, sent after all actions have run
///
/// # Configuration
/// - `pause_ms`: Wait between consecutive actions, in milliseconds (default: 50)
/// - `scale`: Coordinates are divided by this before use. Set it to the `scale` of the Screen Capture the coordinates were read from (default: 1.0)
/// - `failsafe`: Stop with an error when the mouse is in a screen corner before an action. Move the mouse to a corner to abort a running sequence (default: true)
///
/// # Example
/// `[{"action": "click", "x": 200, "y": 100}, {"action": "type", "text": "hello"}, {"action": "key", "key": "enter"}]`
/// clicks at (200, 100), types "hello" and presses Enter.
#[modular_agent(
    title = "GUI Action",
    category = CATEGORY,
    inputs = [PORT_ACTION],
    outputs = [PORT_DONE],
    integer_config(name = CONFIG_PAUSE_MS, default = 50),
    number_config(name = CONFIG_SCALE, default = 1.0),
    boolean_config(name = CONFIG_FAILSAFE, default = true),
)]
struct GuiActionModule {
    data: ModuleData,
}

#[async_trait]
impl AsModule for GuiActionModule {
    fn new(ma: ModularAgent, id: String, spec: ModuleSpec) -> Result<Self> {
        Ok(Self {
            data: ModuleData::new(ma, id, spec),
        })
    }

    async fn process(&mut self, ctx: ModuleContext, _port: String, value: Value) -> Result<()> {
        let actions = action::parse_actions(&value)?;
        let pause_ms = self.configs()?.get_integer_or(CONFIG_PAUSE_MS, 50).max(0);
        let opts = Options {
            pause: Duration::from_millis(pause_ms as u64),
            failsafe: failsafe(self)?,
            scale: scale(self)?,
        };
        run_actions(actions, opts).await?;
        self.output(ctx, PORT_DONE, value).await
    }
}

/// Types the input text as keyboard input.
///
/// Types into whatever has keyboard focus. Characters are sent as text, so
/// any Unicode text can be typed regardless of the keyboard layout.
///
/// # Ports
/// - Input `text`: The text to type
/// - Output `done`: The input text, sent after typing finishes
///
/// # Configuration
/// - `interval_ms`: Wait between characters, in milliseconds. 0 types the whole text at once (default: 0)
/// - `failsafe`: Stop with an error when the mouse is in a screen corner (default: true)
#[modular_agent(
    title = "Type Text",
    category = CATEGORY_KEYBOARD,
    inputs = [PORT_TEXT],
    outputs = [PORT_DONE],
    integer_config(name = CONFIG_INTERVAL_MS, default = 0),
    boolean_config(name = CONFIG_FAILSAFE, default = true),
)]
struct TypeTextModule {
    data: ModuleData,
}

#[async_trait]
impl AsModule for TypeTextModule {
    fn new(ma: ModularAgent, id: String, spec: ModuleSpec) -> Result<Self> {
        Ok(Self {
            data: ModuleData::new(ma, id, spec),
        })
    }

    async fn process(&mut self, ctx: ModuleContext, _port: String, value: Value) -> Result<()> {
        let text = value
            .as_str()
            .ok_or_else(|| Error::InvalidValue("text must be a string".to_string()))?
            .to_string();
        let interval_ms = self.configs()?.get_integer_or(CONFIG_INTERVAL_MS, 0).max(0);
        // With an interval, each character becomes its own action so the
        // pause between actions supplies the interval.
        let actions = if interval_ms > 0 {
            text.chars()
                .map(|c| Action::Type {
                    text: c.to_string(),
                })
                .collect()
        } else {
            vec![Action::Type { text }]
        };
        let opts = Options {
            pause: Duration::from_millis(interval_ms as u64),
            failsafe: failsafe(self)?,
            scale: 1.0,
        };
        run_actions(actions, opts).await?;
        self.output(ctx, PORT_DONE, value).await
    }
}

/// Presses a key combination.
///
/// The keys are pressed in order and released in reverse, like pressing a
/// shortcut by hand.
///
/// # Ports
/// - Input `unit`: Any value triggers the key combination
/// - Output `done`: The input value, sent after the keys are released
///
/// # Configuration
/// - `keys`: Key names joined with `+`, such as `ctrl+c` or `alt+tab`. Names are those accepted by GUI Action
/// - `failsafe`: Stop with an error when the mouse is in a screen corner (default: true)
#[modular_agent(
    title = "Hotkey",
    category = CATEGORY_KEYBOARD,
    inputs = [PORT_UNIT],
    outputs = [PORT_DONE],
    string_config(name = CONFIG_KEYS),
    boolean_config(name = CONFIG_FAILSAFE, default = true),
)]
struct HotkeyModule {
    data: ModuleData,
}

#[async_trait]
impl AsModule for HotkeyModule {
    fn new(ma: ModularAgent, id: String, spec: ModuleSpec) -> Result<Self> {
        Ok(Self {
            data: ModuleData::new(ma, id, spec),
        })
    }

    async fn process(&mut self, ctx: ModuleContext, _port: String, value: Value) -> Result<()> {
        let keys = self.configs()?.get_string_or_default(CONFIG_KEYS);
        let hotkey = action::parse_hotkey(&keys)?;
        let opts = Options {
            pause: Duration::ZERO,
            failsafe: failsafe(self)?,
            scale: 1.0,
        };
        run_actions(vec![hotkey], opts).await?;
        self.output(ctx, PORT_DONE, value).await
    }
}

/// The window record Find Window outputs, as far as clicking into it needs.
#[derive(Debug, Deserialize)]
struct WindowRecord {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    dpi_scale: f64,
}

/// What Move Mouse and Click act on: a window record from Find Window, or
/// otherwise a point. `x` and `y` of a point stay optional so that Click can
/// take any value as a trigger.
enum Target {
    Window(WindowRecord),
    Point { x: Option<f64>, y: Option<f64> },
}

fn parse_target(value: &Value) -> Result<Target> {
    if !value.is_object() {
        return Ok(Target::Point { x: None, y: None });
    }
    let invalid = |e: serde_json::Error| Error::InvalidValue(format!("Invalid position: {e}"));
    if value.get("type").and_then(Value::as_str) == Some("window") {
        return serde_json::from_value(value.to_json())
            .map(Target::Window)
            .map_err(invalid);
    }
    #[derive(Deserialize)]
    struct Point {
        x: Option<f64>,
        y: Option<f64>,
    }
    let Point { x, y } = serde_json::from_value(value.to_json()).map_err(invalid)?;
    Ok(Target::Point { x, y })
}

/// Fractions of the client width and height that `anchor` names.
fn anchor_factors(anchor: &str) -> Result<(f64, f64)> {
    Ok(match anchor {
        "top_left" => (0.0, 0.0),
        "top_right" => (1.0, 0.0),
        "bottom_left" => (0.0, 1.0),
        "bottom_right" => (1.0, 1.0),
        "center" => (0.5, 0.5),
        _ => return Err(Error::InvalidConfig(format!("Unknown anchor: {anchor}"))),
    })
}

/// The screen point, in physical pixels, at a logical offset from an anchor
/// of the window's client area.
fn window_point(window: &WindowRecord, anchor: (f64, f64), offset: (f64, f64)) -> (f64, f64) {
    (
        window.x + window.width * anchor.0 + offset.0 * window.dpi_scale,
        window.y + window.height * anchor.1 + offset.1 * window.dpi_scale,
    )
}

/// Resolves a window input to a screen point from the `x`, `y` and `anchor`
/// configs, which then needs no further scaling.
fn point_in_window(module: &impl Module, window: &WindowRecord) -> Result<(f64, f64)> {
    let configs = module.configs()?;
    let anchor = anchor_factors(&configs.get_string_or(CONFIG_ANCHOR, "top_left"))?;
    let offset = (
        configs.get_number_or(CONFIG_X, 0.0),
        configs.get_number_or(CONFIG_Y, 0.0),
    );
    Ok(window_point(window, anchor, offset))
}

fn mouse_options(module: &impl Module, scale: f64) -> Result<Options> {
    Ok(Options {
        pause: Duration::ZERO,
        failsafe: failsafe(module)?,
        scale,
    })
}

/// Moves the mouse to the input position, or to a spot in the input window.
///
/// The input is dispatched on its shape:
/// - A window record from Find Window (`"type": "window"`): moves to the
///   point `x`/`y` logical pixels from the window's `anchor`. Logical pixels
///   are physical pixels at 100% display scaling, so the same offsets hit the
///   same control on displays of any scaling.
/// - `{"x", "y"}`: moves to that screen point, so the output of Mouse Position
///   or a point picked by an LLM can be connected directly.
///
/// # Ports
/// - Input `position`: A window record, or `{"x": number, "y": number}`
/// - Output `done`: The input value, sent after the mouse has moved
///
/// # Configuration
/// - `x`, `y`: Offset in the window, in logical pixels. Mouse Position reports it for the window under the cursor. Unused for a point input (default: 0)
/// - `anchor`: The corner of the window's client area the offset is measured from: `top_left`, `top_right`, `bottom_left`, `bottom_right` or `center`. Measure from the corner a control stays next to when the window is resized (default: top_left)
/// - `duration_ms`: Move in a straight line over this time instead of jumping. Use it for menus that open on hover (default: 0)
/// - `relative`: Move by the input `x`/`y` from the current position. Not for a window input (default: false)
/// - `scale`: A point input is divided by this before use. Set it to the `scale` of the Screen Capture the point was read from (default: 1.0)
/// - `failsafe`: Stop with an error when the mouse is in a screen corner (default: true)
#[modular_agent(
    title = "Move Mouse",
    category = CATEGORY_MOUSE,
    inputs = [PORT_POSITION],
    outputs = [PORT_DONE],
    number_config(name = CONFIG_X, default = 0.0),
    number_config(name = CONFIG_Y, default = 0.0),
    string_config(name = CONFIG_ANCHOR, default = "top_left"),
    integer_config(name = CONFIG_DURATION_MS, default = 0),
    boolean_config(name = CONFIG_RELATIVE, default = false),
    number_config(name = CONFIG_SCALE, default = 1.0),
    boolean_config(name = CONFIG_FAILSAFE, default = true),
)]
struct MoveMouseModule {
    data: ModuleData,
}

#[async_trait]
impl AsModule for MoveMouseModule {
    fn new(ma: ModularAgent, id: String, spec: ModuleSpec) -> Result<Self> {
        Ok(Self {
            data: ModuleData::new(ma, id, spec),
        })
    }

    async fn process(&mut self, ctx: ModuleContext, _port: String, value: Value) -> Result<()> {
        let relative = self.configs()?.get_bool_or(CONFIG_RELATIVE, false);
        let ((x, y), scale) = match parse_target(&value)? {
            Target::Window(window) => {
                if relative {
                    return Err(Error::InvalidConfig(
                        "relative cannot be used with a window input".to_string(),
                    ));
                }
                (point_in_window(self, &window)?, 1.0)
            }
            Target::Point {
                x: Some(x),
                y: Some(y),
            } => ((x, y), scale(self)?),
            Target::Point { .. } => {
                return Err(Error::InvalidValue(
                    "position needs numeric x and y".to_string(),
                ));
            }
        };
        let duration_ms = self.configs()?.get_integer_or(CONFIG_DURATION_MS, 0).max(0);
        let move_action = Action::Move {
            x,
            y,
            relative,
            duration_ms: duration_ms as u64,
        };
        run_actions(vec![move_action], mouse_options(self, scale)?).await?;
        self.output(ctx, PORT_DONE, value).await
    }
}

/// Clicks a mouse button at the input position, at a spot in the input window,
/// or where the mouse is.
///
/// The input is dispatched as in Move Mouse: a window record from Find Window
/// clicks `x`/`y` logical pixels from the window's `anchor`, and `{"x", "y"}`
/// clicks that screen point. Any other value clicks where the mouse is.
///
/// # Ports
/// - Input `position`: A window record, `{"x": number, "y": number}`, or any value to click in place
/// - Output `done`: The input value, sent after clicking
///
/// # Configuration
/// - `x`, `y`: Offset in the window, in logical pixels. Mouse Position reports it for the window under the cursor. Unused for other inputs (default: 0)
/// - `anchor`: The corner of the window's client area the offset is measured from: `top_left`, `top_right`, `bottom_left`, `bottom_right` or `center` (default: top_left)
/// - `button`: `left`, `right` or `middle` (default: left)
/// - `clicks`: Number of clicks; 2 for a double click (default: 1)
/// - `scale`: A point input is divided by this before use. Set it to the `scale` of the Screen Capture the point was read from (default: 1.0)
/// - `failsafe`: Stop with an error when the mouse is in a screen corner (default: true)
#[modular_agent(
    title = "Click",
    category = CATEGORY_MOUSE,
    inputs = [PORT_POSITION],
    outputs = [PORT_DONE],
    number_config(name = CONFIG_X, default = 0.0),
    number_config(name = CONFIG_Y, default = 0.0),
    string_config(name = CONFIG_ANCHOR, default = "top_left"),
    string_config(name = CONFIG_BUTTON, default = "left"),
    integer_config(name = CONFIG_CLICKS, default = 1),
    number_config(name = CONFIG_SCALE, default = 1.0),
    boolean_config(name = CONFIG_FAILSAFE, default = true),
)]
struct ClickModule {
    data: ModuleData,
}

#[async_trait]
impl AsModule for ClickModule {
    fn new(ma: ModularAgent, id: String, spec: ModuleSpec) -> Result<Self> {
        Ok(Self {
            data: ModuleData::new(ma, id, spec),
        })
    }

    async fn process(&mut self, ctx: ModuleContext, _port: String, value: Value) -> Result<()> {
        let (x, y, scale) = match parse_target(&value)? {
            Target::Window(window) => {
                let (x, y) = point_in_window(self, &window)?;
                (Some(x), Some(y), 1.0)
            }
            Target::Point { x, y } => (x, y, scale(self)?),
        };
        let configs = self.configs()?;
        let click = Action::Click {
            x,
            y,
            button: MouseButton::parse(&configs.get_string_or(CONFIG_BUTTON, "left"))?,
            clicks: configs.get_integer_or(CONFIG_CLICKS, 1).max(0) as u32,
        };
        run_actions(vec![click], mouse_options(self, scale)?).await?;
        self.output(ctx, PORT_DONE, value).await
    }
}

/// Outputs the current mouse position, and where it is in the window under it.
///
/// `window` gives the position in the window's client area in logical
/// pixels, the numbers to put in `x`/`y` of Move Mouse or Click to reach the
/// same spot through Find Window. Point at a control and read its offset off
/// here. Its `width`/`height` help to work out an offset from another
/// `anchor`: for `bottom_right`, subtract them from `x`/`y`.
///
/// # Ports
/// - Input `unit`: Any value triggers a reading
/// - Output `position`: `{"x": integer, "y": integer, "window": {"title", "process_name", "x", "y", "width", "height"}}`. `x`/`y` are pixels on the primary monitor, multiplied by `scale`. `window` is present on Windows only
///
/// # Configuration
/// - `scale`: The screen position is multiplied by this, so it can be compared with a Screen Capture of the same `scale` (default: 1.0)
#[modular_agent(
    title = "Mouse Position",
    category = CATEGORY_MOUSE,
    inputs = [PORT_UNIT],
    outputs = [PORT_POSITION],
    number_config(name = CONFIG_SCALE, default = 1.0),
)]
struct MousePositionModule {
    data: ModuleData,
}

#[async_trait]
impl AsModule for MousePositionModule {
    fn new(ma: ModularAgent, id: String, spec: ModuleSpec) -> Result<Self> {
        Ok(Self {
            data: ModuleData::new(ma, id, spec),
        })
    }

    async fn process(&mut self, ctx: ModuleContext, _port: String, _value: Value) -> Result<()> {
        let scale = scale(self)?;
        let ((x, y), under) = action::with_enigo(|enigo| {
            let (x, y) = enigo
                .location()
                .map_err(|e| Error::Other(format!("Failed to read mouse position: {e}")))?;
            Ok(((x, y), window::at_point(x, y)?))
        })
        .await?;
        let mut position = hashmap! {
            "x".to_string() => Value::integer((x as f64 * scale).round() as i64),
            "y".to_string() => Value::integer((y as f64 * scale).round() as i64),
        };
        if let Some(w) = under {
            let logical = |v: i32| Value::integer((v as f64 / w.dpi_scale).round() as i64);
            position.insert(
                "window".to_string(),
                Value::object(hashmap! {
                    "title".to_string() => Value::string(&w.title),
                    "process_name".to_string() => Value::string(&w.process_name),
                    "x".to_string() => logical(x - w.x),
                    "y".to_string() => logical(y - w.y),
                    "width".to_string() => logical(w.width),
                    "height".to_string() => logical(w.height),
                }),
            );
        }
        self.output(ctx, PORT_POSITION, Value::object(position))
            .await
    }
}

/// How often Find Window looks again while waiting for the window.
const FIND_POLL: Duration = Duration::from_millis(100);

/// Finds a window by title or process, and optionally brings it to the front
/// and fixes its size.
///
/// The topmost visible window whose title and process name contain the
/// configured text (ignoring case) is used. Fixing the size keeps controls at
/// the same place in the window, so Move Mouse and Click can reach them by
/// fixed offsets from the output. Nothing is done to a window that is already
/// at the requested size and in front, so it is not held up waiting for the
/// window to redraw.
///
/// Windows only.
///
/// # Ports
/// - Input `unit`: Any value starts the search
/// - Output `window`: `{"type": "window", "id", "title", "process_name", "pid", "x", "y", "width", "height", "dpi_scale"}`. `x`/`y`/`width`/`height` are the client area in physical screen pixels, and `dpi_scale` is physical pixels per logical pixel. Connect it to Move Mouse or Click
///
/// # Configuration
/// - `title`: Text the window title contains
/// - `process_name`: Text the executable name contains, such as `notepad.exe`
/// - `timeout_ms`: Keep looking this long for the window to appear before failing, such as after launching the application. 0 looks once (default: 0)
/// - `activate`: Bring the window to the front, restoring it if minimized (default: true)
/// - `width`, `height`: Resize the client area to this, in logical pixels, restoring the window if maximized. 0 leaves the size alone (default: 0)
#[modular_agent(
    title = "Find Window",
    category = CATEGORY_WINDOW,
    inputs = [PORT_UNIT],
    outputs = [PORT_WINDOW],
    string_config(name = CONFIG_TITLE),
    string_config(name = CONFIG_PROCESS_NAME),
    integer_config(name = CONFIG_TIMEOUT_MS, default = 0),
    boolean_config(name = CONFIG_ACTIVATE, default = true),
    number_config(name = CONFIG_WIDTH, default = 0.0),
    number_config(name = CONFIG_HEIGHT, default = 0.0),
)]
struct FindWindowModule {
    data: ModuleData,
}

#[async_trait]
impl AsModule for FindWindowModule {
    fn new(ma: ModularAgent, id: String, spec: ModuleSpec) -> Result<Self> {
        Ok(Self {
            data: ModuleData::new(ma, id, spec),
        })
    }

    async fn process(&mut self, ctx: ModuleContext, _port: String, _value: Value) -> Result<()> {
        let configs = self.configs()?;
        let query = Query {
            title: configs.get_string_or_default(CONFIG_TITLE),
            process_name: configs.get_string_or_default(CONFIG_PROCESS_NAME),
        };
        if query.title.is_empty() && query.process_name.is_empty() {
            return Err(Error::InvalidConfig(
                "Set title or process_name".to_string(),
            ));
        }
        let size = match (
            configs.get_number_or(CONFIG_WIDTH, 0.0),
            configs.get_number_or(CONFIG_HEIGHT, 0.0),
        ) {
            (w, h) if w <= 0.0 && h <= 0.0 => None,
            (w, h) if w > 0.0 && h > 0.0 => Some((w, h)),
            _ => {
                return Err(Error::InvalidConfig(
                    "Set both width and height, or neither".to_string(),
                ));
            }
        };
        let placement = Placement {
            activate: configs.get_bool_or(CONFIG_ACTIVATE, true),
            size,
        };
        let timeout =
            Duration::from_millis(configs.get_integer_or(CONFIG_TIMEOUT_MS, 0).max(0) as u64);

        let deadline = tokio::time::Instant::now() + timeout;
        let found = loop {
            let q = query.clone();
            if let Some(found) = action::run_blocking(move || window::find(&q)).await? {
                break found;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Other(format!(
                    "No window found for title {:?}, process_name {:?}",
                    query.title, query.process_name
                )));
            }
            tokio::time::sleep(FIND_POLL).await;
        };

        let placed = if placement.activate || placement.size.is_some() {
            let id = found.id;
            action::with_enigo(move |enigo| window::place(enigo, id, &placement)).await?
        } else {
            found
        };
        if let Some(size) = size
            && !window::size_matches(&placed, size)
        {
            log::warn!(
                "Window {:?} is {}x{} instead of {}x{} physical pixels; the application may limit its size",
                placed.title,
                placed.width,
                placed.height,
                (size.0 * placed.dpi_scale).round(),
                (size.1 * placed.dpi_scale).round(),
            );
        }
        self.output(ctx, PORT_WINDOW, window_value(&placed)).await
    }
}

fn window_value(w: &WindowInfo) -> Value {
    Value::object(hashmap! {
        "type".to_string() => Value::string("window"),
        "id".to_string() => Value::integer(w.id),
        "title".to_string() => Value::string(&w.title),
        "process_name".to_string() => Value::string(&w.process_name),
        "pid".to_string() => Value::integer(w.pid as i64),
        "x".to_string() => Value::integer(w.x as i64),
        "y".to_string() => Value::integer(w.y as i64),
        "width".to_string() => Value::integer(w.width as i64),
        "height".to_string() => Value::integer(w.height as i64),
        "dpi_scale".to_string() => Value::number(w.dpi_scale),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> WindowRecord {
        WindowRecord {
            x: 100.0,
            y: 50.0,
            width: 1200.0,
            height: 900.0,
            dpi_scale: 1.5,
        }
    }

    #[test]
    fn window_offsets_are_logical() {
        let w = window();
        assert_eq!(window_point(&w, (0.0, 0.0), (20.0, 10.0)), (130.0, 65.0));
        assert_eq!(
            window_point(&w, anchor_factors("bottom_right").unwrap(), (-20.0, -10.0)),
            (1270.0, 935.0)
        );
        assert_eq!(
            window_point(&w, anchor_factors("center").unwrap(), (0.0, 0.0)),
            (700.0, 500.0)
        );
        assert!(anchor_factors("middle").is_err());
    }

    #[test]
    fn target_dispatch() {
        let window = Value::from_json(serde_json::json!({
            "type": "window", "id": 1, "title": "t", "process_name": "p", "pid": 1,
            "x": 100, "y": 50, "width": 1200, "height": 900, "dpi_scale": 1.5,
        }))
        .unwrap();
        assert!(matches!(parse_target(&window).unwrap(), Target::Window(_)));

        let point = Value::from_json(serde_json::json!({"x": 1, "y": 2})).unwrap();
        assert!(matches!(
            parse_target(&point).unwrap(),
            Target::Point {
                x: Some(1.0),
                y: Some(2.0)
            }
        ));
        assert!(matches!(
            parse_target(&Value::unit()).unwrap(),
            Target::Point { x: None, y: None }
        ));
    }
}
