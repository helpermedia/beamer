import { useRef } from "react";
import { useGestures, useParam } from "@helpermedia/beamer-react";
import ValueText from "./ValueText";

// The arc of a knob in degrees clockwise from the right, as the gestures
// count them: 270, open at the bottom.
const ARC_START = 135;
const ARC_SWEEP = 270;

// The knob in SVG units: rem-sized pixels at the editor's own size.
const SIZE = 48;
const CENTER = SIZE / 2;
const RING = 21;

function pointAt(degrees: number, radius: number): [number, number] {
  const radians = (degrees * Math.PI) / 180;
  return [CENTER + radius * Math.cos(radians), CENTER + radius * Math.sin(radians)];
}

// An SVG path along the ring, from its start to `part` of it.
function ringPath(part: number): string {
  const [x0, y0] = pointAt(ARC_START, RING);
  const sweep = part * ARC_SWEEP;
  const [x1, y1] = pointAt(ARC_START + sweep, RING);
  return `M ${x0} ${y0} A ${RING} ${RING} 0 ${sweep > 180 ? 1 : 0} 1 ${x1} ${y1}`;
}

const TRACK = ringPath(1);

interface KnobProps {
  paramId: string;
  // The name under the knob, the parameter's own name by default.
  label?: string;
}

// A knob for a continuous parameter. The input comes from beamer-gestures:
// drag up and down (finer with shift), scroll, arrow keys once focused and
// double click to reset. A click on the value types one.
function Knob({ paramId, label }: KnobProps) {
  const { param, value, text, info } = useParam(paramId);
  const svgRef = useRef<SVGSVGElement>(null);
  useGestures(svgRef, param);

  const name = label ?? info?.name ?? paramId;
  const angle = ARC_START + value * ARC_SWEEP;
  const [x0, y0] = pointAt(angle, 7);
  const [x1, y1] = pointAt(angle, 14);

  return (
    <div className="flex w-21 flex-col items-center portrait:w-20">
      <svg
        ref={svgRef}
        viewBox={`0 0 ${SIZE} ${SIZE}`}
        tabIndex={0}
        role="slider"
        aria-label={name}
        aria-valuemin={0}
        aria-valuemax={1}
        aria-valuenow={value}
        aria-valuetext={text}
        className="group size-12 outline-none"
      >
        <path
          d={TRACK}
          className="fill-none stroke-neutral-800 group-data-[keyboard-focus]:stroke-neutral-600"
          strokeWidth={3}
          strokeLinecap="round"
        />
        {value > 0.001 && (
          <path d={ringPath(value)} className="fill-none stroke-brand-blue" strokeWidth={3} strokeLinecap="round" />
        )}
        <circle cx={CENTER} cy={CENTER} r={16} className="fill-neutral-800 stroke-neutral-700" strokeWidth={1} />
        <line x1={x0} y1={y0} x2={x1} y2={y1} className="stroke-paper" strokeWidth={2.5} strokeLinecap="round" />
      </svg>
      <span className="mt-1 text-[0.6875rem] text-neutral-400">{name}</span>
      <ValueText param={param} text={text} name={name} />
    </div>
  );
}

export default Knob;
