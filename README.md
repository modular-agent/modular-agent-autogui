# AutoGUI Modules for Modular Agent

Keyboard and mouse control for Modular Agent, in the spirit of pyautogui.

| Module | Purpose |
| --- | --- |
| GUI Action | Runs action objects (`move`, `click`, `drag`, `scroll`, `type`, `key`, `hotkey`, `wait`, …), one or an array |
| Type Text | Types the input string |
| Hotkey | Presses a key combination such as `ctrl+c` |
| Move Mouse | Moves the mouse to an input `{x, y}`, or to an offset in a window from Find Window |
| Click | Clicks at an input `{x, y}`, at an offset in a window from Find Window, or in place |
| Mouse Position | Outputs the mouse position, on screen and in the window under it |
| Find Window | Finds a window by title or process, brings it to the front and fixes its size (Windows only) |
| Capture Window | Captures a window's client area, in the same logical pixels as Click's offsets (Windows only) |

Coordinates are pixels on the primary monitor. Pair `scale` with the `scale` of
lifelog's Screen Capture to click at coordinates read from a scaled screenshot.

Moving the mouse into a screen corner aborts a running sequence (`failsafe`,
on by default).

## Automating an application

To click a control of an application, address it relative to its window
rather than the screen:

1. **Find Window** locates the window, brings it to the front and resizes its
   client area to a fixed size, so controls stay at the same place in it. A
   window already at that size is left alone.
2. **Mouse Position**, with the pointer on the control, reports its offset in
   the window under `window`.
3. **Click** (or Move Mouse) takes the window from Find Window and clicks at
   that offset, set in its `x`/`y` configs, from the `anchor` corner.

**Capture Window** shows the window as the application draws it, even when
other windows cover it. At `scale` 1.0 a point on the image is the offset to
give Click, so offsets can be measured on a capture as well.

Lifelog's Screen Capture serves a different purpose: it records what is on
screen, such as the active window's region as the user sees it. Capture
Window targets one application and keeps its coordinates in step with
Click.

Window offsets and sizes are logical pixels: physical pixels at 100% display
scaling. Applications scale their controls with the display, so the same
numbers reach the same control at any scaling.

## Example

[`examples/showcase.json`](examples/showcase.json) exercises every module:
reading the mouse position, a GUI Action sequence that opens Notepad and types
into it (Windows only), Type Text followed by a Hotkey, a Move Mouse glide
followed by a right Click, and Find Window fixing Notepad's size before a
Click into it and a Capture Window of it. Each row starts from its own Unit Input.

## Platform notes

- **Windows**: input does not reach windows of applications running as
  administrator unless this app runs as administrator too. Windows may refuse
  to bring a window to the front; Find Window then fails.
- **macOS**: the app needs the Accessibility permission.
- **Linux**: X11 is supported by default.
