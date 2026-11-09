import type { Param } from "@helpermedia/beamer-api";
import { thumbPart, valueAtThumbPosition } from "./thumb-position.js";

/**
 * How a drag turns a dial. "vertical" raises the value upwards,
 * "horizontal" to the right, "both" up and to the right together. With
 * "arc" the value follows the pointer around the element's center: it
 * jumps to the pointer on press. When the pointer passes through the gap
 * between the arc's ends, the value waits at the end it left until the
 * pointer comes back to that side.
 */
export type DialDrag = "vertical" | "horizontal" | "both" | "arc";

/** The direction a slider moves in: up raises an upright one, right a lying one. */
export type SliderOrientation = "vertical" | "horizontal";

/** A way to reset a value to its default: a double click, or a click with Command or Control held. */
export type ResetGesture = "double-click" | "modifier-click";

/** How a dial or a slider takes pointer, wheel and key input. */
export interface InputOptions {
  /** Normalized change per pixel of a drag or of the wheel. Default 1/200. */
  dragStep?: number;
  /** Factor for drags, the wheel and keys with shift held. Default 0.1. */
  fineFactor?: number;
  /** Normalized change per arrow key; a discrete parameter moves one choice. Default 0.01. */
  keyStep?: number;
  /** Normalized change per Page Up or Page Down. Default 0.1. */
  pageStep?: number;
  /**
   * The most pixels one wheel event counts for. A mouse notch is a few pixels
   * in WebKit but 100 in Chromium; the cap makes a notch about as far
   * everywhere. Default 10.
   */
  wheelCap?: number;
  /** A scroll ends when no wheel event came for this long, in ms. Default 300. */
  wheelRestMs?: number;
  /**
   * What resets the value to its default: a double click, a click with
   * Command or Control held, or both. An empty list turns reset off.
   * Default a double click.
   */
  reset?: ResetGesture[];
}

/** How a dial takes input: the drag and its arc, then what every control shares. */
export interface DialOptions extends InputOptions {
  /**
   * The drag, or a function that picks one for each press, for example
   * "arc" when the press lands on the dial's handle and "vertical"
   * elsewhere. Default "vertical".
   */
  drag?: DialDrag | ((event: PointerEvent) => DialDrag);
  /**
   * Where the arc starts, in degrees clockwise from the right of the center
   * (90 is straight down). Default 135, the lower left.
   */
  arcStart?: number;
  /** The degrees of the arc, clockwise from its start. Default 270. */
  arcSweep?: number;
  /**
   * Near the center an angle changes fast with little movement, so an arc
   * drag ignores the pointer within this many pixels of it. Default 5.
   */
  arcDeadZone?: number;
}

/** How a slider takes input: its direction and thumb, then what every control shares. */
export interface SliderOptions extends InputOptions {
  /**
   * The direction it moves in on screen and its `aria-orientation`. A
   * slider turned upright with a CSS rotation is "vertical". Default
   * "vertical".
   */
  orientation?: SliderOrientation;
  /**
   * The length of the thumb along the slider, as a part of the element's
   * length (0 up to, not including, 1). The thumb's center travels from
   * half of it in from one end to half of it in from the other, so the
   * pointer stays on the spot of the thumb it pressed. A part, not pixels,
   * so it holds when the editor scales. Default 0: the center travels the
   * whole length. With nothing left to travel (a thumb of 1, an element
   * without a size) the slider moves by distance, as a dial does.
   */
  thumbLength?: number;
}

/** Where a slider's thumb travels, for the drag that follows the pointer along it. */
export interface Track {
  orientation: SliderOrientation;
  thumbLength: number;
}

const DEFAULTS: Required<DialOptions> = {
  drag: "vertical",
  arcStart: 135,
  arcSweep: 270,
  arcDeadZone: 5,
  dragStep: 1 / 200,
  fineFactor: 0.1,
  keyStep: 0.01,
  pageStep: 0.1,
  wheelCap: 10,
  wheelRestMs: 300,
  reset: ["double-click"],
};

// Pixels a wheel event counts for per line and per page, when it reports
// its delta in lines or pages.
const PIXELS_PER_LINE = 8;
const PIXELS_PER_PAGE = 100;

/** The angle of a point around a center in degrees, clockwise on screen from the right. */
export function angleAround(centerX: number, centerY: number, x: number, y: number): number {
  return (Math.atan2(y - centerY, x - centerX) * 180) / Math.PI;
}

/** The change of an angle from `from` to `to` in degrees, the short way round. */
export function angleDelta(from: number, to: number): number {
  let delta = (to - from) % 360;
  if (delta > 180) delta -= 360;
  if (delta <= -180) delta += 360;
  return delta;
}

/**
 * The degrees along an arc from its start to `angle`. An angle in the gap
 * between the arc's ends counts from the nearer end: past the end it is
 * more than `arcSweep`, before the start it is negative.
 */
export function arcDegrees(angle: number, arcStart: number, arcSweep: number): number {
  const along = (((angle - arcStart) % 360) + 360) % 360;
  const gapMiddle = arcSweep + (360 - arcSweep) / 2;
  return along > gapMiddle ? along - 360 : along;
}

/**
 * The normalized value at `angle` on an arc. An angle in the gap between
 * the arc's ends gives the nearer end.
 */
export function arcPosition(angle: number, arcStart: number, arcSweep: number): number {
  return Math.min(1, Math.max(0, arcDegrees(angle, arcStart, arcSweep) / arcSweep));
}

/**
 * The pixels a pointer moved in the direction that raises the value: up
 * for "vertical", right for "horizontal", both added for "both".
 */
export function dragPixels(drag: "vertical" | "horizontal" | "both", dx: number, dy: number): number {
  switch (drag) {
    case "vertical":
      return -dy;
    case "horizontal":
      return dx;
    case "both":
      return dx - dy;
  }
}

/** A wheel event, as far as the gestures read it. */
export type WheelInput = Pick<WheelEvent, "deltaX" | "deltaY" | "deltaMode" | "shiftKey"> & {
  webkitDirectionInvertedFromDevice?: boolean;
};

/**
 * The pixels a wheel event counts for, positive when the user scrolls up,
 * whatever the system's natural scrolling does. Shift turns a mouse wheel
 * horizontal on macOS, so a horizontal delta counts without a vertical one.
 * A delta in lines or pages counts as pixels. The result is at most `cap`
 * either way.
 */
export function wheelPixels(event: WheelInput, cap: number): number {
  const raw = event.deltaY !== 0 ? event.deltaY : event.shiftKey ? event.deltaX : 0;
  if (raw === 0) return 0;
  const scale = event.deltaMode === 1 ? PIXELS_PER_LINE : event.deltaMode === 2 ? PIXELS_PER_PAGE : 1;
  // With natural scrolling the system inverts the delta; undo it, so that
  // scrolling up raises the value as on a hardware dial.
  const up = event.webkitDirectionInvertedFromDevice ? raw * scale : -raw * scale;
  return Math.max(-cap, Math.min(cap, up));
}

/** The normalized value a key sets, or undefined for a key without one. */
export function keyValue(key: string, value: number, step: number, pageStep: number): number | undefined {
  switch (key) {
    case "ArrowUp":
    case "ArrowRight":
      return value + step;
    case "ArrowDown":
    case "ArrowLeft":
      return value - step;
    case "PageUp":
      return value + pageStep;
    case "PageDown":
      return value - pageStep;
    case "Home":
      return 0;
    case "End":
      return 1;
    default:
      return undefined;
  }
}

/**
 * The change a gesture can still add to `start` and stay within 0 to 1.
 * A drag past an end therefore turns back at once.
 */
export function boundedOffset(start: number, offset: number): number {
  return Math.min(1, Math.max(0, start + offset)) - start;
}

/**
 * Make `element` control `target`: drag, wheel, arrow keys and reset. The
 * layer under `attachDial` and `attachSlider`, which add the role, the ARIA
 * values and the focus.
 *
 * Each drag and each scroll is one gesture. It moves the value from where
 * it started, so a host that snaps the value to a step in between does not
 * stop it. A key or a reset during a drag or scroll ends that one first. A
 * press focuses the element. The element gets `touch-action: none`, so
 * that a touch drag turns the control instead of scrolling the page, until
 * it is detached. Input before the parameter is known (before `ready`) is
 * ignored. Returns the function that detaches the listeners.
 */
export function attachGestures(
  element: HTMLElement | SVGElement,
  target: Param,
  options: DialOptions = {},
  track?: Track,
): () => void {
  // An option given as undefined keeps its default.
  const given = Object.fromEntries(Object.entries(options).filter(([, value]) => value !== undefined));
  const settings: Required<DialOptions> = { ...DEFAULTS, ...given };
  const listeners: Array<[string, EventListener, AddEventListenerOptions?]> = [];
  const listen = <E extends Event>(type: string, listener: (event: E) => void, opts?: AddEventListenerOptions) => {
    listeners.push([type, listener as EventListener, opts]);
    element.addEventListener(type, listener as EventListener, opts);
  };
  const touchAction = element.style.touchAction;
  element.style.touchAction = "none";

  // Dragging: one gesture from press to release. A linear drag moves from
  // the value at the press: each move adds to the offset, so pressing or
  // releasing shift halfway does not make the value jump. An arc drag
  // follows the pointer's turn in degrees along the arc, past its ends
  // too. Once the pointer has passed the middle of the gap between them,
  // the value parks at the end it left (`parked`) and waits until the
  // pointer is back on that side of the gap. A track drag (a slider's)
  // keeps the thumb under the pointer: the value is the pointer's position
  // along the track less the `offset` from the thumb's center where it
  // pressed. With shift it moves finer from `value`, the value it set,
  // while the offset follows, so releasing shift halfway does not make the
  // thumb jump.
  type LinearDrag = { kind: "linear"; mode: Exclude<DialDrag, "arc">; start: number; offset: number; lastX: number; lastY: number };
  type ArcDrag = { kind: "arc"; degrees: number; lastAngle: number | null; parked: 0 | 1 | null };
  type TrackDrag = { kind: "track"; offset: number; value: number; lastPosition: number };
  let drag: ((LinearDrag | ArcDrag | TrackDrag) & { pointerId: number }) | null = null;
  // The pointer's angle around the center, null within the dead zone.
  const angleOf = (event: PointerEvent): number | null => {
    const box = element.getBoundingClientRect();
    const centerX = box.left + box.width / 2;
    const centerY = box.top + box.height / 2;
    if (Math.hypot(event.clientX - centerX, event.clientY - centerY) < settings.arcDeadZone) return null;
    return angleAround(centerX, centerY, event.clientX, event.clientY);
  };
  // The pointer's position along the track: 0 where the thumb's center is
  // at the lowest value, 1 at the highest, beyond them past the ends. Null
  // when the element has no length to measure. The value is the inverse of
  // `thumbPosition`, which draws the thumb.
  const trackPosition = (event: PointerEvent): number | null => {
    if (!track) return null;
    const box = element.getBoundingClientRect();
    const vertical = track.orientation === "vertical";
    const length = vertical ? box.height : box.width;
    if (!(length > 0)) return null;
    const part = (vertical ? box.bottom - event.clientY : event.clientX - box.left) / length;
    return valueAtThumbPosition(part, track.thumbLength) ?? null;
  };
  // Half the thumb's length in positions along the track.
  const thumbHalf = (): number => {
    if (!track) return 0;
    const thumb = thumbPart(track.thumbLength);
    return thumb >= 1 ? 0 : thumb / 2 / (1 - thumb);
  };
  const endDrag = () => {
    if (!drag) return;
    drag = null;
    target.end();
  };
  // Whether the last click reset the value, so that its context menu stays
  // closed.
  let resetByClick = false;

  const gapHalf = Math.max(1, (360 - settings.arcSweep) / 2);
  const moveArc = (arc: ArcDrag, event: PointerEvent) => {
    // Within the dead zone nothing changes. Leaving it, the value jumps to
    // the pointer as on a press.
    const angle = angleOf(event);
    if (angle === null) {
      arc.lastAngle = null;
      arc.parked = null;
      return;
    }
    const pointer = arcDegrees(angle, settings.arcStart, settings.arcSweep);
    if (arc.parked !== null) {
      // Back on the side of the gap where the value waits.
      if (Math.abs(pointer - arc.parked * settings.arcSweep) > gapHalf) return;
      arc.parked = null;
      arc.degrees = pointer;
    } else if (arc.lastAngle === null) {
      arc.degrees = pointer;
    } else {
      arc.degrees += angleDelta(arc.lastAngle, angle);
      // Past the middle of the gap: wait at the end the pointer left.
      if (arc.degrees > settings.arcSweep + gapHalf) arc.parked = 1;
      else if (arc.degrees < -gapHalf) arc.parked = 0;
    }
    arc.lastAngle = angle;
    const value = arc.parked ?? Math.min(1, Math.max(0, arc.degrees / settings.arcSweep));
    if (value !== target.get()) target.set(value);
  };

  listen<PointerEvent>("pointerdown", (event) => {
    // The primary button only: a right click opens the context menu, which
    // takes the release.
    if (event.button !== 0 || drag || !target.info) return;
    if (settings.reset.includes("modifier-click") && (event.metaKey || event.ctrlKey)) {
      target.reset();
      resetByClick = true;
      return;
    }
    // No text selection or image drag. That also stops the click from
    // focusing the element, so focus it here.
    event.preventDefault();
    element.focus({ preventScroll: true });
    element.setPointerCapture(event.pointerId);
    target.begin(() => {
      drag = null;
    });
    const mode = typeof settings.drag === "function" ? settings.drag(event) : settings.drag;
    const position = trackPosition(event);
    if (track && position !== null) {
      // A press on the thumb grabs it where it is pressed, a press beside
      // it on the track brings the thumb's center to the pointer.
      const value = target.get();
      const onThumb = Math.abs(position - value) <= thumbHalf();
      const offset = onThumb ? position - value : 0;
      const pressed = Math.min(1, Math.max(0, position - offset));
      drag = { pointerId: event.pointerId, kind: "track", offset, value: pressed, lastPosition: position };
      if (!onThumb) target.set(pressed);
    } else if (track) {
      // Nothing to measure: move by distance, as a dial does.
      drag = { pointerId: event.pointerId, kind: "linear", mode: track.orientation, start: target.get(), offset: 0, lastX: event.clientX, lastY: event.clientY };
    } else if (mode === "arc") {
      // The value jumps to the pointer, unless the press is in the dead zone.
      const angle = angleOf(event);
      const degrees = angle === null ? target.get() * settings.arcSweep : arcDegrees(angle, settings.arcStart, settings.arcSweep);
      drag = { pointerId: event.pointerId, kind: "arc", degrees, lastAngle: angle, parked: null };
      target.set(angle === null ? target.get() : arcPosition(angle, settings.arcStart, settings.arcSweep));
    } else {
      drag = { pointerId: event.pointerId, kind: "linear", mode, start: target.get(), offset: 0, lastX: event.clientX, lastY: event.clientY };
    }
  });
  listen<PointerEvent>("pointermove", (event) => {
    if (!drag || event.pointerId !== drag.pointerId) return;
    if (drag.kind === "arc") {
      moveArc(drag, event);
      return;
    }
    if (drag.kind === "track") {
      const position = trackPosition(event);
      if (position === null) return;
      if (event.shiftKey) {
        drag.value = Math.min(1, Math.max(0, drag.value + (position - drag.lastPosition) * settings.fineFactor));
        drag.offset = position - drag.value;
      } else {
        drag.value = Math.min(1, Math.max(0, position - drag.offset));
      }
      drag.lastPosition = position;
      target.set(drag.value);
      return;
    }
    const fine = event.shiftKey ? settings.fineFactor : 1;
    const pixels = dragPixels(drag.mode, event.clientX - drag.lastX, event.clientY - drag.lastY);
    const change = pixels * settings.dragStep * fine;
    drag.lastX = event.clientX;
    drag.lastY = event.clientY;
    drag.offset = boundedOffset(drag.start, drag.offset + change);
    target.set(drag.start + drag.offset);
  });
  listen("pointerup", endDrag);
  listen("pointercancel", endDrag);
  // The capture also ends when the window loses the pointer halfway.
  listen("lostpointercapture", endDrag);

  // On macOS a click with Control held opens the context menu, which takes
  // the release: end the drag it began. A click that reset the value shows
  // no menu.
  listen<MouseEvent>("contextmenu", (event) => {
    endDrag();
    if (resetByClick) {
      event.preventDefault();
      resetByClick = false;
    }
  });

  // Scrolling moves the value by the distance, as far as a drag would. A
  // trackpad sends many small events, so one gesture lasts until the wheel
  // rests. Like a drag it moves from the value where it started.
  let scroll: { start: number; offset: number; rest?: ReturnType<typeof setTimeout> } | null = null;
  const settleScroll = () => {
    if (!scroll) return;
    clearTimeout(scroll.rest);
    scroll = null;
    target.end();
  };
  listen<WheelEvent>("wheel", (event) => {
    event.preventDefault();
    if (drag || !target.info) return;
    const pixels = wheelPixels(event, settings.wheelCap);
    if (pixels === 0) return;
    if (scroll) {
      clearTimeout(scroll.rest);
    } else {
      target.begin(() => {
        if (scroll) clearTimeout(scroll.rest);
        scroll = null;
      });
      scroll = { start: target.get(), offset: 0 };
    }
    const fine = event.shiftKey ? settings.fineFactor : 1;
    scroll.offset = boundedOffset(scroll.start, scroll.offset + pixels * settings.dragStep * fine);
    target.set(scroll.start + scroll.offset);
    scroll.rest = setTimeout(settleScroll, settings.wheelRestMs);
  }, { passive: false });

  listen<KeyboardEvent>("keydown", (event) => {
    // Shortcuts with Command, Control or Alt belong to the page.
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    // A discrete parameter moves one choice per key, whatever the step.
    const steps = target.info?.steps ?? 0;
    const keyStep = steps > 0 ? 1 / steps : settings.keyStep;
    const step = event.shiftKey && steps === 0 ? keyStep * settings.fineFactor : keyStep;
    const next = keyValue(event.key, target.get(), step, Math.max(settings.pageStep, keyStep));
    if (next === undefined) return;
    event.preventDefault();
    target.edit(next);
  });

  if (settings.reset.includes("double-click")) {
    listen("dblclick", () => target.reset());
  }

  return () => {
    endDrag();
    settleScroll();
    for (const [type, listener, opts] of listeners) {
      element.removeEventListener(type, listener, opts);
    }
    element.style.touchAction = touchAction;
  };
}
