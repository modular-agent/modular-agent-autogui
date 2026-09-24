use std::time::Duration;

use enigo::Mouse;
use modular_agent_core::im::hashmap;
use modular_agent_core::{
    AsModule, Error, ModularAgent, Module, ModuleContext, ModuleData, ModuleOutput, ModuleSpec,
    Result, Value, async_trait, modular_agent,
};

use crate::action::{self, Action, Options};

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
/// Actions (fields other than `action` are optional unless listed):
/// - `{"action": "move", "x", "y"}`: Move the mouse
/// - `{"action": "click", "x", "y", "button", "clicks"}`: Click, first moving to `x`/`y` when given
/// - `{"action": "down" | "up", "button"}`: Press or release a mouse button
/// - `{"action": "drag", "x", "y", "button"}`: Press, move to `x`/`y`, release
/// - `{"action": "scroll", "dx", "dy"}`: Scroll by wheel clicks; positive `dy` scrolls down
/// - `{"action": "type", "text"}`: Type text
/// - `{"action": "key" | "key_down" | "key_up", "key"}`: Press and release, press, or release a key
/// - `{"action": "hotkey", "keys"}`: Press `keys` in order, release in reverse
/// - `{"action": "wait", "ms"}`: Wait
///
/// `button` is `"left"` (default), `"right"` or `"middle"`. Key names follow
/// pyautogui: `ctrl`, `shift`, `alt`, `win`/`cmd`, `enter`, `tab`, `esc`,
/// `backspace`, `delete`, `space`, `up`/`down`/`left`/`right`, `home`, `end`,
/// `pageup`, `pagedown`, `f1`–`f12`, or any single character.
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
