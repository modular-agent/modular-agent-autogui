use std::time::Duration;

use enigo::Mouse;
use modular_agent_core::im::hashmap;
use modular_agent_core::{
    AsModule, Error, ModularAgent, Module, ModuleContext, ModuleData, ModuleOutput, ModuleSpec,
    Result, Value, async_trait, modular_agent,
};
use serde::Deserialize;

use crate::action::{self, Action, MouseButton, Options, Point};

static CATEGORY_KEYBOARD: &str = "AutoGUI/Keyboard";
static CATEGORY_MOUSE: &str = "AutoGUI/Mouse";
static CATEGORY: &str = "AutoGUI";

static PORT_ACTION: &str = "action";
static PORT_TEXT: &str = "text";
static PORT_UNIT: &str = "unit";
static PORT_DONE: &str = "done";
static PORT_POSITION: &str = "position";

static CONFIG_PAUSE_MS: &str = "pause_ms";
static CONFIG_INTERVAL_MS: &str = "interval_ms";
static CONFIG_KEYS: &str = "keys";
static CONFIG_SCALE: &str = "scale";
static CONFIG_FAILSAFE: &str = "failsafe";
static CONFIG_DURATION_MS: &str = "duration_ms";
static CONFIG_RELATIVE: &str = "relative";
static CONFIG_BUTTON: &str = "button";
static CONFIG_CLICKS: &str = "clicks";

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
/// - `{"action": "move", "x", "y", "relative", "duration_ms", "origin"}`: Move the mouse
/// - `{"action": "click", "x", "y", "button", "clicks", "origin"}`: Click, first moving to `x`/`y` when given
/// - `{"action": "down" | "up", "button"}`: Press or release a mouse button
/// - `{"action": "drag", "x", "y", "button", "relative", "duration_ms", "origin"}`: Press, move to `x`/`y`, release
/// - `{"action": "scroll", "dx", "dy"}`: Scroll by wheel clicks; positive `dy` scrolls down
/// - `{"action": "type", "text"}`: Type text
/// - `{"action": "key", "key", "presses"}`: Press and release a key `presses` times (default 1)
/// - `{"action": "key_down" | "key_up", "key"}`: Press or release a key
/// - `{"action": "hotkey", "keys"}`: Press `keys` in order, release in reverse
/// - `{"action": "wait", "ms"}`: Wait
///
/// `relative: true` moves by `x`/`y` from the current position. `duration_ms`
/// moves in a straight line over that time instead of jumping, for menus that
/// open on hover and applications that ignore an instant drag. `origin` is
/// `{"x", "y"}`, the screen position of the image the coordinates were read
/// from, such as the `x`/`y` of a Screen Capture event; it cannot be combined
/// with `relative`.
///
/// `button` is `"left"` (default), `"right"` or `"middle"`. Key names follow
/// pyautogui: `ctrl`, `shift`, `alt`, `win`/`cmd` (with `ctrlleft`,
/// `shiftright`, … for one side), `enter`, `tab`, `esc`, `backspace`,
/// `delete`, `space`, `up`/`down`/`left`/`right`, `home`, `end`, `pageup`,
/// `pagedown`, `f1`–`f24`, `num0`–`num9`, `volumeup`, `playpause`, `kanji`,
/// `convert`, `nonconvert`, or any single character. Some names exist only on
/// some platforms.
///
/// Coordinates are pixels on the primary monitor. Keys still held by
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
/// - `scale`: Coordinates are divided by this before use, and `origin` is added after. Set it to the `scale` of the Screen Capture the coordinates were read from (default: 1.0)
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

/// `x` and `y` stay optional so that Click can take any value as a trigger.
#[derive(Deserialize)]
struct PositionInput {
    x: Option<f64>,
    y: Option<f64>,
    origin: Option<Point>,
}

fn parse_position(value: &Value) -> Result<PositionInput> {
    if !value.is_object() {
        return Ok(PositionInput {
            x: None,
            y: None,
            origin: None,
        });
    }
    serde_json::from_value(value.to_json())
        .map_err(|e| Error::InvalidValue(format!("Invalid position: {e}")))
}

fn mouse_options(module: &impl Module) -> Result<Options> {
    Ok(Options {
        pause: Duration::ZERO,
        failsafe: failsafe(module)?,
        scale: scale(module)?,
    })
}

/// Moves the mouse to the input position.
///
/// The input is `{"x", "y"}`, so the output of Mouse Position or a point
/// picked by an LLM can be connected directly. An optional `origin`
/// (`{"x", "y"}`) in the input is added to the position, for coordinates read
/// from a Screen Capture of a window: set it to the `x`/`y` of that capture's
/// event.
///
/// # Ports
/// - Input `position`: `{"x": number, "y": number}`, optionally with `origin`
/// - Output `done`: The input value, sent after the mouse has moved
///
/// # Configuration
/// - `duration_ms`: Move in a straight line over this time instead of jumping. Use it for menus that open on hover (default: 0)
/// - `relative`: Move by `x`/`y` from the current position (default: false)
/// - `scale`: The position is divided by this before use. Set it to the `scale` of the Screen Capture the position was read from (default: 1.0)
/// - `failsafe`: Stop with an error when the mouse is in a screen corner (default: true)
#[modular_agent(
    title = "Move Mouse",
    category = CATEGORY_MOUSE,
    inputs = [PORT_POSITION],
    outputs = [PORT_DONE],
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
        let position = parse_position(&value)?;
        let (Some(x), Some(y)) = (position.x, position.y) else {
            return Err(Error::InvalidValue(
                "position needs numeric x and y".to_string(),
            ));
        };
        let configs = self.configs()?;
        let move_action = Action::Move {
            x,
            y,
            relative: configs.get_bool_or(CONFIG_RELATIVE, false),
            duration_ms: configs.get_integer_or(CONFIG_DURATION_MS, 0).max(0) as u64,
            origin: position.origin,
        };
        action::validate(&move_action)?;
        run_actions(vec![move_action], mouse_options(self)?).await?;
        self.output(ctx, PORT_DONE, value).await
    }
}

/// Clicks a mouse button, at the input position when it has one.
///
/// An input with `x`/`y` moves the mouse there first, taking `origin` into
/// account as Move Mouse does. Any other value clicks where the mouse is.
///
/// # Ports
/// - Input `position`: `{"x": number, "y": number}`, or any value to click in place
/// - Output `done`: The input value, sent after clicking
///
/// # Configuration
/// - `button`: `left`, `right` or `middle` (default: left)
/// - `clicks`: Number of clicks; 2 for a double click (default: 1)
/// - `scale`: The position is divided by this before use. Set it to the `scale` of the Screen Capture the position was read from (default: 1.0)
/// - `failsafe`: Stop with an error when the mouse is in a screen corner (default: true)
#[modular_agent(
    title = "Click",
    category = CATEGORY_MOUSE,
    inputs = [PORT_POSITION],
    outputs = [PORT_DONE],
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
        let position = parse_position(&value)?;
        let configs = self.configs()?;
        let click = Action::Click {
            x: position.x,
            y: position.y,
            button: MouseButton::parse(&configs.get_string_or(CONFIG_BUTTON, "left"))?,
            clicks: configs.get_integer_or(CONFIG_CLICKS, 1).max(0) as u32,
            origin: position.origin,
        };
        run_actions(vec![click], mouse_options(self)?).await?;
        self.output(ctx, PORT_DONE, value).await
    }
}

/// Outputs the current mouse position.
///
/// # Ports
/// - Input `unit`: Any value triggers a reading
/// - Output `position`: `{"x": integer, "y": integer}` in pixels on the primary monitor, multiplied by `scale`
///
/// # Configuration
/// - `scale`: The position is multiplied by this, so it can be compared with a Screen Capture of the same `scale` (default: 1.0)
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
        let (x, y) = action::with_enigo(|enigo| {
            enigo
                .location()
                .map_err(|e| Error::Other(format!("Failed to read mouse position: {e}")))
        })
        .await?;
        let position = Value::object(hashmap! {
            "x".to_string() => Value::integer((x as f64 * scale).round() as i64),
            "y".to_string() => Value::integer((y as f64 * scale).round() as i64),
        });
        self.output(ctx, PORT_POSITION, position).await
    }
}
