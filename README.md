# AutoGUI Modules for Modular Agent

Keyboard and mouse control for Modular Agent, in the spirit of pyautogui.

| Module | Purpose |
| --- | --- |
| GUI Action | Runs action objects (`move`, `click`, `drag`, `scroll`, `type`, `key`, `hotkey`, `wait`, …), one or an array |
| Type Text | Types the input string |
| Hotkey | Presses a key combination such as `ctrl+c` |
| Mouse Position | Outputs the current mouse position |

Coordinates are pixels on the primary monitor. Pair `scale` with the `scale` of
lifelog's Screen Capture to click at coordinates read from a scaled screenshot.

Moving the mouse into a screen corner aborts a running sequence (`failsafe`,
on by default).

## Platform notes

- **Windows**: input does not reach windows of applications running as
  administrator unless this app runs as administrator too.
- **macOS**: the app needs the Accessibility permission.
- **Linux**: X11 is supported by default.
