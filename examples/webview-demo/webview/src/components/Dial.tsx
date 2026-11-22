import { useRef, useState } from "react";
import type { ParameterId } from "@helpermedia/beamer-api";
import { openTextField } from "@helpermedia/beamer-controls";
import type { DialDrag } from "@helpermedia/beamer-controls";
import { Dial as DialControl, TextField, useParameter } from "@helpermedia/beamer-react";

interface DialProps {
  parameterId: ParameterId;
  size?: number;
  label?: string;
  className?: string;
}

// 7 o'clock position (gap at bottom).
const ARC_START = (5 / 4) * Math.PI;
// 270 degrees total.
const ARC_SWEEP = (3 / 2) * Math.PI;

// The same arc for the gestures, which count degrees clockwise on screen
// from the right: 135 and 270.
const ARC_START_DEGREES = 360 - (ARC_START * 180) / Math.PI;
const ARC_SWEEP_DEGREES = (ARC_SWEEP * 180) / Math.PI;

const CX = 32;
const CY = 32;
const R = 26;

// How close to the handle a press grabs it, larger than the visual dot.
const HANDLE_HIT_RADIUS = 8;

function polarToCartesian(angle: number): { x: number; y: number } {
  return {
    x: CX + R * Math.cos(angle),
    y: CY - R * Math.sin(angle),
  };
}

function describeArc(startAngle: number, endAngle: number): string {
  const start = polarToCartesian(startAngle);
  const end = polarToCartesian(endAngle);
  const sweep = startAngle - endAngle;
  const largeArc = sweep > Math.PI ? 1 : 0;
  return `M ${start.x} ${start.y} A ${R} ${R} 0 ${largeArc} 1 ${end.x} ${end.y}`;
}

const trackPath = describeArc(ARC_START, ARC_START - ARC_SWEEP);

// A dial for plugin parameters: the look over beamer-react's Dial.
//
// Input comes from beamer-controls:
//   - Grab handle: the handle follows the pointer around the arc.
//   - Press elsewhere: drag up and down, finer with shift.
//   - Scroll wheel, one undo step per scroll.
//   - Arrow keys once focused. Focus from the keyboard (Tab, or keys after
//     a click) brightens the track.
//   - Cmd+click or Option+click (Ctrl+click or Alt+click outside macOS):
//     reset to default.
// Double-click on the dial or its value types a value, which the parameter
// itself parses.
function Dial({ parameterId, size = 64, label, className }: DialProps) {
  const { parameter, value, info } = useParameter(parameterId);

  const svgRef = useRef<SVGSVGElement>(null);
  // Whether the handle is held, to light it up.
  const [grabbed, setGrabbed] = useState(false);

  // The arc when the press lands on the handle, vertical elsewhere. The
  // test runs in SVG coordinates, which also holds when the host scales
  // the editor (Logic Pro's zoom).
  const pickDrag = (event: PointerEvent): DialDrag => {
    const svg = svgRef.current;
    const ctm = svg?.getScreenCTM();
    if (!svg || !ctm) return "vertical";
    const point = new DOMPoint(event.clientX, event.clientY).matrixTransform(ctm.inverse());
    const handle = polarToCartesian(ARC_START - parameter.get() * ARC_SWEEP);
    const onHandle = Math.hypot(point.x - handle.x, point.y - handle.y) <= HANDLE_HIT_RADIUS;
    setGrabbed(onHandle);
    return onHandle ? "arc" : "vertical";
  };
  const displayLabel = label ?? info?.name ?? parameterId;

  const valueRef = useRef<HTMLInputElement>(null);
  const typeValue = () => {
    if (valueRef.current) openTextField(valueRef.current);
  };

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

      {/* Double-click is for typing a value, so a modifier click resets. */}
      <DialControl
        parameter={parameter}
        drag={pickDrag}
        arcStart={ARC_START_DEGREES}
        arcSweep={ARC_SWEEP_DEGREES}
        reset={["ctrl-click", "alt-click"]}
        aria-label={label}
        className="group cursor-default outline-none"
        style={{ width: size, height: size }}
        onDoubleClick={typeValue}
        onLostPointerCapture={() => setGrabbed(false)}
      >
        <svg ref={svgRef} viewBox="0 0 64 64" width={size} height={size}>
          <path
            d={trackPath}
            className="fill-none stroke-gray-700 group-data-[keyboard-focus]:stroke-gray-500"
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
          <circle cx={dotPos.x} cy={dotPos.y} r={4} className={`pointer-events-none ${grabbed ? "fill-cyan-300" : "fill-current"}`} />
        </svg>
      </DialControl>

      <TextField
        parameter={parameter}
        open="double-click"
        ref={valueRef}
        aria-label={label === undefined ? undefined : `${label} value`}
        className="w-full text-xs font-mono tabular-nums text-center border-0 outline-none rounded px-1 py-0.5 text-current bg-transparent cursor-text data-[typing]:text-black data-[typing]:bg-white"
      />
    </div>
  );
}

export default Dial;
