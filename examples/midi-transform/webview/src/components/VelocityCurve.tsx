import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";
import { sampleCurve, withPointAdded, withPointMoved, withPointRemoved } from "../curve";
import type { Curve } from "../model-types";
import type { Note } from "../transformed";

// The editor in SVG units: rem-sized pixels at the editor's own size, as
// the CC curve.
const SIZE = 436;
// The square of the velocities, 0 to 127 both ways, with a band for the
// axis names on the left and below.
const LEFT = 24;
const TOP = 8;
const SIDE = SIZE - LEFT - 8;
const BOTTOM = TOP + SIDE;
// The axis names, centered in their bands on the height of their capitals
// (Inter's cap height), as the CC curve's.
const LABEL_SIZE = 11;
const LABEL_CAP_HEIGHT = LABEL_SIZE * 0.727;
const LABEL_BAND = LEFT;
// A point as it shows and the circle around it that takes the pointer.
const POINT_RADIUS = 5;
const HIT_RADIUS = 12;
// How far an arrow key moves a point: one velocity step, ten with Shift.
const STEP = 1 / 127;
// How many steps the curve is drawn in, besides its points.
const DRAWN_STEPS = 200;
// Two presses on one point within this many milliseconds are a double
// click, when the first one neither added nor moved the point.
const DOUBLE_CLICK_MS = 500;
// How far in pixels a press may move and still count as a click.
const CLICK_SLOP = 3;

// On macOS a click with Control held opens the context menu. The shortcut
// key of a reset click is Command there, elsewhere Control.
const APPLE = typeof navigator !== "undefined" && /Mac|iPhone|iPad|iPod/.test(navigator.platform);

function x(value: number): number {
  return LEFT + value * SIDE;
}

function y(value: number): number {
  return BOTTOM - value * SIDE;
}

// A value from 0 to 1 as a velocity from 0 to 127.
export function velocity(value: number): number {
  return Math.round(value * 127);
}

interface VelocityCurveProps {
  curve: Curve;
  onChange: (curve: Curve) => void;
  // Called with the index of the point being dragged or focused, whose
  // values the panel shows, or null.
  onActive: (index: number | null) => void;
  // The last note that passed, null before the first.
  last: Note | null;
  // False while the plugin is bypassed.
  lit: boolean;
  // False while the curve is off and does nothing.
  used: boolean;
}

// A press on a point: which point, when and where, whether the press added
// it, whether it moved since and whether its release ends a double click.
// Tells a double click from a drag.
interface Press {
  id: number;
  time: number;
  clientX: number;
  clientY: number;
  added: boolean;
  moved: boolean;
  secondClick: boolean;
}

// The velocity curve, in to out, with its points to drag. A click where
// there is no point adds one; a double click, an Option (Alt) click or a
// Command (Control) click on a point removes it, as these reset a
// control. A press focuses its point, which then moves with the arrow
// keys and goes with Delete.
// The last note shows as a dot at its velocity in and out.
//
// Each point has an id for as long as the editor shows it, so that the
// focus and the point in hand stay with the point when another one is
// added or removed before it. The ids are the editor's own and are not
// saved.
function VelocityCurve({ curve, onChange, onActive, last, lit, used }: VelocityCurveProps) {
  const svg = useRef<SVGSVGElement>(null);
  const elements = useRef(new Map<number, SVGGElement>());
  const ids = useRef<number[]>([]);
  const nextId = useRef(0);
  // A curve that came from elsewhere with another number of points (a
  // reset, a loaded project) gets new ids.
  if (ids.current.length !== curve.points.length) {
    ids.current = curve.points.map(() => nextId.current++);
  }

  // The point the pointer drags, with the offset between the pointer and
  // the point's center where it was grabbed, in curve values.
  const drag = useRef<{ id: number; pointerId: number; dx: number; dy: number } | null>(null);
  const [draggedId, setDraggedId] = useState<number | null>(null);
  const [focusedId, setFocusedId] = useState<number | null>(null);
  // Whether the focus came from the keyboard, which shows its ring. A
  // press focuses its point from script, which shows none.
  const [keyboardFocus, setKeyboardFocus] = useState(false);
  const pressing = useRef(false);
  const lastPress = useRef<Press | null>(null);
  // A point to focus once it is drawn: one a press added, or the one that
  // took the place of a point removed with Delete.
  const focusNext = useRef<number | null>(null);
  const quarters = [0.25, 0.5, 0.75];

  const activeId = draggedId ?? focusedId;
  const activeIndex = activeId === null ? -1 : ids.current.indexOf(activeId);
  useEffect(() => {
    onActive(activeIndex < 0 ? null : activeIndex);
  }, [activeIndex, onActive]);

  useLayoutEffect(() => {
    if (focusNext.current === null) return;
    elements.current.get(focusNext.current)?.focus({ preventScroll: true });
    focusNext.current = null;
  });

  // Where a pointer event is in the values of the curve, from 0 to 1
  // inside the square.
  function position(event: PointerEvent): { x: number; y: number } {
    const rect = svg.current!.getBoundingClientRect();
    const unitX = ((event.clientX - rect.left) / rect.width) * SIZE;
    const unitY = ((event.clientY - rect.top) / rect.height) * SIZE;
    return { x: (unitX - LEFT) / SIDE, y: (BOTTOM - unitY) / SIDE };
  }

  // The id of the point an event is on, or null.
  function pointOf(target: EventTarget): number | null {
    const element = (target as Element).closest("[data-point]");
    return element ? Number(element.getAttribute("data-point")) : null;
  }

  // Remove a point and return the id of the one that takes its place, or
  // null when the curve keeps its two points.
  function remove(id: number): number | null {
    const index = ids.current.indexOf(id);
    if (index < 0 || curve.points.length <= 2) return null;
    ids.current = ids.current.filter((other) => other !== id);
    onChange(withPointRemoved(curve, index));
    return ids.current[Math.min(index, ids.current.length - 1)];
  }

  function onPointerDown(event: PointerEvent<SVGSVGElement>) {
    // The primary button only, without Control on macOS: that click opens
    // the context menu, which takes the release.
    if (event.button !== 0 || drag.current !== null || (APPLE && event.ctrlKey)) return;
    // No text selection. That also keeps the press from focusing, so the
    // point is focused here.
    event.preventDefault();
    pressing.current = true;
    setKeyboardFocus(false);
    const at = position(event);
    let id = pointOf(event.target);
    const previous = lastPress.current;

    if (id !== null && (event.altKey || (APPLE ? event.metaKey : event.ctrlKey))) {
      lastPress.current = null;
      pressing.current = false;
      remove(id);
      return;
    }
    // The second press of a double click on a point that the first press
    // neither added nor moved. Its release removes the point, unless it
    // moves in between and becomes a drag.
    const secondClick =
      id !== null &&
      previous !== null &&
      previous.id === id &&
      !previous.added &&
      !previous.moved &&
      event.timeStamp - previous.time < DOUBLE_CLICK_MS;

    let added = false;
    let dx = 0;
    let dy = 0;
    if (id === null) {
      if (at.x < 0 || at.x > 1 || at.y < 0 || at.y > 1) {
        pressing.current = false;
        return;
      }
      const result = withPointAdded(curve, at.x, at.y);
      if (result === null) {
        pressing.current = false;
        return;
      }
      id = nextId.current++;
      ids.current = [...ids.current.slice(0, result.index), id, ...ids.current.slice(result.index)];
      onChange(result.curve);
      focusNext.current = id;
      added = true;
    } else {
      // Grabbed where it is pressed, so the point does not jump to the
      // pointer.
      const point = curve.points[ids.current.indexOf(id)];
      dx = at.x - point.x;
      dy = at.y - point.y;
      elements.current.get(id)?.focus({ preventScroll: true });
    }

    lastPress.current = {
      id,
      time: event.timeStamp,
      clientX: event.clientX,
      clientY: event.clientY,
      added,
      moved: false,
      secondClick,
    };
    drag.current = { id, pointerId: event.pointerId, dx, dy };
    setDraggedId(id);
    svg.current!.setPointerCapture(event.pointerId);
  }

  function onPointerMove(event: PointerEvent<SVGSVGElement>) {
    const current = drag.current;
    if (current === null || event.pointerId !== current.pointerId) return;
    const press = lastPress.current;
    if (press && !press.moved) {
      if (Math.hypot(event.clientX - press.clientX, event.clientY - press.clientY) <= CLICK_SLOP) return;
      press.moved = true;
    }
    const index = ids.current.indexOf(current.id);
    if (index < 0) return;
    const at = position(event);
    onChange(withPointMoved(curve, index, at.x - current.dx, at.y - current.dy));
  }

  // The end of a drag: the release, a cancel, the capture lost halfway or
  // the context menu, which takes the release.
  function onPointerEnd() {
    pressing.current = false;
    if (drag.current === null) return;
    drag.current = null;
    setDraggedId(null);
  }

  // The release of a press, which ends a double click when it is the
  // second one and did not move.
  function onPointerUp(event: PointerEvent<SVGSVGElement>) {
    if (drag.current === null || event.pointerId !== drag.current.pointerId) return;
    const press = lastPress.current;
    onPointerEnd();
    if (press && press.secondClick && !press.moved) {
      lastPress.current = null;
      remove(press.id);
    }
  }

  // Shortcuts with Command, Control or Alt belong to the page.
  function onKeyDown(event: KeyboardEvent, id: number) {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const index = ids.current.indexOf(id);
    if (index < 0) return;
    const step = event.shiftKey ? 10 * STEP : STEP;
    const point = curve.points[index];
    const moves: Record<string, [number, number]> = {
      ArrowUp: [0, step],
      ArrowDown: [0, -step],
      ArrowRight: [step, 0],
      ArrowLeft: [-step, 0],
    };
    if (event.key in moves) {
      const [dx, dy] = moves[event.key];
      onChange(withPointMoved(curve, index, point.x + dx, point.y + dy));
    } else if (event.key === "Delete" || event.key === "Backspace") {
      const next = remove(id);
      if (next !== null) focusNext.current = next;
    } else {
      return;
    }
    event.preventDefault();
    setKeyboardFocus(true);
  }

  const drawn = sampleCurve(curve, DRAWN_STEPS)
    .map((point) => `${x(point.x)},${y(point.y)}`)
    .join(" ");

  return (
    <svg
      ref={svg}
      viewBox={`0 0 ${SIZE} ${SIZE}`}
      aria-label="Velocity curve, velocity in to out"
      className="display control size-[27.25rem] shrink-0 touch-none rounded-lg bg-well ring-1 ring-neutral-800 ring-inset"
      data-unlit={lit ? undefined : ""}
      data-unused={used ? undefined : ""}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerEnd}
      onLostPointerCapture={onPointerEnd}
      onContextMenu={onPointerEnd}
    >
      {quarters.map((part) => (
        <g key={part} className="stroke-neutral-800" strokeWidth={1}>
          <line x1={x(part)} y1={y(0)} x2={x(part)} y2={y(1)} />
          <line x1={x(0)} y1={y(part)} x2={x(1)} y2={y(part)} />
        </g>
      ))}
      <rect x={LEFT} y={TOP} width={SIDE} height={SIDE} className="fill-none stroke-neutral-800" strokeWidth={1} />

      {/* Unchanged velocities. */}
      <line x1={x(0)} y1={y(0)} x2={x(1)} y2={y(1)} className="stroke-neutral-700" strokeWidth={1} strokeDasharray="3 3" />
      <polyline points={drawn} className="fill-none stroke-brand-blue" strokeWidth={2} strokeLinejoin="round" />

      {last && <circle cx={x(last.velocity)} cy={y(last.outVelocity)} r={3.5} className="fill-signal" />}

      {curve.points.map((point, index) => {
        const id = ids.current[index];
        return (
          <g
            key={id}
            ref={(element) => {
              if (element) elements.current.set(id, element);
              else elements.current.delete(id);
            }}
            data-point={id}
            className="outline-none"
            tabIndex={0}
            role="slider"
            aria-label={`Point ${index + 1}`}
            aria-valuemin={0}
            aria-valuemax={127}
            aria-valuenow={velocity(point.y)}
            aria-valuetext={`In ${velocity(point.x)}, out ${velocity(point.y)}`}
            onFocus={() => {
              setFocusedId(id);
              if (!pressing.current) setKeyboardFocus(true);
            }}
            onBlur={() => setFocusedId((focused) => (focused === id ? null : focused))}
            onKeyDown={(event) => onKeyDown(event, id)}
          >
            <circle cx={x(point.x)} cy={y(point.y)} r={HIT_RADIUS} className="fill-transparent" />
            {keyboardFocus && focusedId === id && (
              <circle cx={x(point.x)} cy={y(point.y)} r={POINT_RADIUS + 4} className="fill-none stroke-paper" strokeWidth={1.5} />
            )}
            <circle
              cx={x(point.x)}
              cy={y(point.y)}
              r={POINT_RADIUS}
              className={activeId === id ? "fill-brand-blue stroke-brand-blue" : "fill-well stroke-brand-blue"}
              strokeWidth={2}
            />
          </g>
        );
      })}

      <text
        x={LEFT + SIDE / 2}
        y={BOTTOM + LABEL_BAND / 2 + LABEL_CAP_HEIGHT / 2}
        textAnchor="middle"
        fontSize={LABEL_SIZE}
        className="fill-neutral-500"
      >
        In
      </text>
      <text
        x={0}
        y={0}
        transform={`translate(${LABEL_BAND / 2 + LABEL_CAP_HEIGHT / 2} ${TOP + SIDE / 2}) rotate(-90)`}
        textAnchor="middle"
        fontSize={LABEL_SIZE}
        className="fill-neutral-500"
      >
        Out
      </text>
    </svg>
  );
}

export default VelocityCurve;
