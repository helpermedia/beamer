import { useRef, useCallback, useState } from "react";
import { useBeamerControl } from "../hooks/useBeamerControl";

interface SliderProps {
  type?: "circular" | "horizontal" | "vertical";
  paramId: string;
  size?: number;
  label?: string;
  className?: string;
}

// 7 o'clock position (gap at bottom).
const ARC_START = (5 / 4) * Math.PI;
// 270 degrees total.
const ARC_SWEEP = (3 / 2) * Math.PI;

const CX = 32;
const CY = 32;
const R = 26;

const SENSITIVITY = 200;
const FINE_SENSITIVITY = 800;
const SCROLL_STEP = 0.01;
const FINE_SCROLL_STEP = 0.002;

// Hit radius for grabbing the handle (larger than the visual dot).
const HANDLE_HIT_RADIUS = 8;

function polarToCartesian(angle: number): { x: number; y: number } {
  return {
    x: CX + R * Math.cos(angle),
    y: CY - R * Math.sin(angle),
  };
}

// Convert SVG-local (x, y) to a normalized value [0, 1] along the arc.
function svgPointToValue(x: number, y: number): number {
  // atan2 with flipped Y to match SVG coordinate system.
  const angle = Math.atan2(CY - y, x - CX);
  // Distance from ARC_START, normalized into [0, 2pi).
  let delta = ARC_START - angle;
  if (delta < 0) delta += 2 * Math.PI;
  if (delta >= 2 * Math.PI) delta -= 2 * Math.PI;

  // Pointer is in the dead zone. Snap to nearest end.
  if (delta > ARC_SWEEP) {
    const gapMid = ARC_SWEEP + (2 * Math.PI - ARC_SWEEP) / 2;
    return delta < gapMid ? 1 : 0;
  }

  return delta / ARC_SWEEP;
}

function describeArc(startAngle: number, endAngle: number): string {
  const start = polarToCartesian(startAngle);
  const end = polarToCartesian(endAngle);
  const sweep = startAngle - endAngle;
  const largeArc = sweep > Math.PI ? 1 : 0;
  return `M ${start.x} ${start.y} A ${R} ${R} 0 ${largeArc} 1 ${end.x} ${end.y}`;
}

// Adaptive decimal precision based on parameter range.
function formatDisplayValue(
  displayValue: number,
  info: BeamerParamInfo | undefined,
): string {
  if (!info) return fixNegZero(displayValue.toFixed(1));
  if (info.steps > 0) return Math.round(displayValue).toString();
  const range = info.max - info.min;
  if (range <= 1) return fixNegZero(displayValue.toFixed(3));
  if (range <= 10) return fixNegZero(displayValue.toFixed(2));
  return fixNegZero(displayValue.toFixed(1));
}

// Strip the sign from "-0", "-0.0", "-0.00" etc. AU hosts round-trip
// parameter values through f32, which can produce tiny negative values
// that format as negative zero.
function fixNegZero(s: string): string {
  return s.charCodeAt(0) === 45 && +s === 0 ? s.slice(1) : s;
}

const trackPath = describeArc(ARC_START, ARC_START - ARC_SWEEP);

// Circular knob slider for plugin parameters.
//
// Interaction modes:
//   - Grab handle: angular tracking (handle follows mouse around the arc).
//   - Click elsewhere: linear mode (vertical drag adjusts value).
//   - Shift+drag: fine adjustment.
//   - Cmd+click: reset to default.
//   - Scroll wheel: step adjustment (debounced for DAW undo grouping).
//   - Double-click: type a value directly.
function Slider({ type = "circular", paramId, size = 64, label, className }: SliderProps) {
  const { value, displayValue, info, beginEdit, set, endEdit, resetToDefault } =
    useBeamerControl(paramId);

  const svgRef = useRef<SVGSVGElement>(null);
  const dragState = useRef<{
    mode: "linear" | "angular";
    startY: number;
    startValue: number;
    pointerId: number;
  } | null>(null);
  // Debounce timer for wheel endEdit, so consecutive ticks form a single DAW undo entry.
  const wheelTimeout = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Convert client coordinates to SVG-local coordinates.
  // Handles host-side scaling (e.g. Logic Pro's MATTScaleView).
  const toSvgPoint = useCallback((clientX: number, clientY: number): { x: number; y: number } => {
    const svg = svgRef.current;
    if (!svg) return { x: clientX, y: clientY };
    const pt = svg.createSVGPoint();
    pt.x = clientX;
    pt.y = clientY;
    const ctm = svg.getScreenCTM();
    if (!ctm) return { x: clientX, y: clientY };
    const svgPt = pt.matrixTransform(ctm.inverse());
    return { x: svgPt.x, y: svgPt.y };
  }, []);

  const displayLabel = label ?? info?.name ?? paramId;
  const units = info?.units ?? "";

  const [dragging, setDragging] = useState(false);
  const [editing, setEditing] = useState(false);
  const [editText, setEditText] = useState("");

  const inputRefCallback = useCallback((el: HTMLInputElement | null) => {
    if (el) {
      el.focus();
      el.select();
    }
  }, []);

  const handlePointerDown = useCallback(
    (e: React.PointerEvent) => {
      if (e.metaKey) {
        resetToDefault();
        return;
      }
      e.preventDefault();
      (e.target as Element).setPointerCapture(e.pointerId);

      const pt = toSvgPoint(e.clientX, e.clientY);
      const endAngle = ARC_START - value * ARC_SWEEP;
      const handlePos = polarToCartesian(endAngle);
      const dx = pt.x - handlePos.x;
      const dy = pt.y - handlePos.y;
      const isOnHandle = dx * dx + dy * dy <= HANDLE_HIT_RADIUS * HANDLE_HIT_RADIUS;

      dragState.current = {
        mode: isOnHandle ? "angular" : "linear",
        startY: pt.y,
        startValue: value,
        pointerId: e.pointerId,
      };
      beginEdit();
      setDragging(true);
    },
    [value, toSvgPoint, beginEdit, resetToDefault],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (!dragState.current) return;
      const pt = toSvgPoint(e.clientX, e.clientY);

      if (dragState.current.mode === "angular") {
        set(svgPointToValue(pt.x, pt.y));
      } else {
        const deltaY = dragState.current.startY - pt.y;
        const sens = e.shiftKey ? FINE_SENSITIVITY : SENSITIVITY;
        const newValue = dragState.current.startValue + deltaY / sens;
        set(newValue);
      }
    },
    [toSvgPoint, set],
  );

  const handlePointerUp = useCallback(
    (e: React.PointerEvent) => {
      if (!dragState.current) return;
      (e.target as Element).releasePointerCapture(e.pointerId);
      dragState.current = null;
      endEdit();
      setDragging(false);
    },
    [endEdit],
  );

  const handleWheel = useCallback(
    (e: React.WheelEvent) => {
      // Undo natural scrolling inversion so physical scroll-up always increases.
      const rawDeltaY = (e.nativeEvent as any).webkitDirectionInvertedFromDevice ? -e.deltaY : e.deltaY;
      const step = e.shiftKey ? FINE_SCROLL_STEP : SCROLL_STEP;
      const delta = rawDeltaY > 0 ? -step : step;
      if (!wheelTimeout.current) beginEdit();
      else clearTimeout(wheelTimeout.current);
      set(value + delta);
      wheelTimeout.current = setTimeout(() => {
        wheelTimeout.current = null;
        endEdit();
      }, 300);
    },
    [value, beginEdit, set, endEdit],
  );

  const handleDoubleClick = useCallback(() => {
    setEditText(formatDisplayValue(displayValue, info));
    setEditing(true);
  }, [displayValue, info]);

  const commitEdit = useCallback(() => {
    setEditing(false);
    const parsed = parseFloat(editText);
    if (isNaN(parsed) || !info) return;
    const normalized = (parsed - info.min) / (info.max - info.min);
    beginEdit();
    set(normalized);
    endEdit();
  }, [editText, info, beginEdit, set, endEdit]);

  const handleEditKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") commitEdit();
      if (e.key === "Escape") setEditing(false);
    },
    [commitEdit],
  );

  if (type === "circular") {
    const endAngle = ARC_START - value * ARC_SWEEP;
    const dotPos = polarToCartesian(endAngle);

    // Detect bipolar parameters (symmetric range around zero, e.g. pan -1..+1).
    const bipolarCenter = info && info.min < 0 && info.max > 0
      && Math.abs(info.min + info.max) < 0.001
      ? -info.min / (info.max - info.min)
      : null;

    let valuePath = "";
    if (bipolarCenter !== null) {
      const centerAngle = ARC_START - bipolarCenter * ARC_SWEEP;
      if (Math.abs(value - bipolarCenter) > 0.001) {
        // Swap so the larger angle is always the start argument.
        valuePath = value >= bipolarCenter
          ? describeArc(centerAngle, endAngle)
          : describeArc(endAngle, centerAngle);
      }
    } else {
      valuePath = value > 0.001 ? describeArc(ARC_START, endAngle) : "";
    }

    return (
      <div
        className={`flex flex-col items-center gap-1 select-none touch-none ${className ?? "text-cyan-400"}`}
        style={{ width: size }}
      >
        <span className="text-xs text-gray-400 truncate w-full text-center">
          {displayLabel}
        </span>

        <svg
          ref={svgRef}
          viewBox="0 0 64 64"
          width={size}
          height={size}
          className="cursor-default"
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={handlePointerUp}
          onPointerCancel={handlePointerUp}
          onDoubleClick={handleDoubleClick}
          onWheel={handleWheel}
        >
          <path
            d={trackPath}
            className="fill-none stroke-gray-700"
            strokeWidth={4}
            strokeLinecap="round"
          />
          {valuePath && (
            <path
              d={valuePath}
              className="fill-none stroke-current"
              strokeWidth={4}
              strokeLinecap={bipolarCenter !== null ? "butt" : "round"}
            />
          )}
          {bipolarCenter !== null && (() => {
            const ca = ARC_START - bipolarCenter * ARC_SWEEP;
            const on = polarToCartesian(ca);
            // Center tick extending equally above and below the track.
            const dx = on.x - CX;
            const dy = on.y - CY;
            const len = Math.sqrt(dx * dx + dy * dy);
            const unit = { x: dx / len, y: dy / len };
            const start = { x: on.x + unit.x * 4, y: on.y + unit.y * 4 };
            const end = { x: on.x - unit.x * 4, y: on.y - unit.y * 4 };
            return (
              <line
                x1={start.x} y1={start.y}
                x2={end.x} y2={end.y}
                className="stroke-gray-500"
                strokeWidth={1.5}
                strokeLinecap="round"
              />
            );
          })()}
          {/* Invisible hit area for easier handle grabbing. */}
          <circle cx={dotPos.x} cy={dotPos.y} r={HANDLE_HIT_RADIUS} className="fill-transparent" />
          <circle cx={dotPos.x} cy={dotPos.y} r={4} className={`pointer-events-none ${dragging && dragState.current?.mode === "angular" ? "fill-cyan-300" : "fill-current"}`} />
        </svg>

        <input
          ref={editing ? inputRefCallback : undefined}
          type="text"
          readOnly={!editing}
          value={editing ? editText : `${formatDisplayValue(displayValue, info)} ${units}`}
          onChange={(e) => setEditText(e.target.value)}
          onKeyDown={editing ? handleEditKeyDown : undefined}
          onBlur={editing ? commitEdit : undefined}
          onDoubleClick={handleDoubleClick}
          className={`w-full text-xs font-mono tabular-nums text-center border-0 outline-none rounded px-1 py-0.5 ${editing ? "text-black bg-white" : "text-current bg-transparent cursor-text"}`}
        />
      </div>
    );
  }

  return null;
}

export default Slider;
