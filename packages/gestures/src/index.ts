// Input handling for the controls of a Beamer web UI: drag, wheel, keys and
// reset, each change one undo step in the host. It draws nothing.

export { attachGestures, KEYBOARD_FOCUS } from "./gestures.js";
export type { DragMode, GestureOptions, ResetGesture } from "./gestures.js";
