import { useEffect, useRef } from "react";
import { runtime } from "@helpermedia/beamer-api";
import { meterPosition } from "../levels";

// How many points the history shows. The plugin pushes one every 10 ms
// (HISTORY_POINTS_PER_SECOND in lib.rs), so this is 3 seconds.
const POINTS = 300;

interface LevelHistoryProps {
  width?: number;
  height?: number;
}

// The output level of the last seconds, scrolling. The plugin pushes the
// points through a stream sender, so each event holds every point since
// the last one, oldest first.
export default function LevelHistory({ width = 240, height = 64 }: LevelHistoryProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context) return;

    // Sharp on a Retina screen.
    const scale = window.devicePixelRatio || 1;
    canvas.width = width * scale;
    canvas.height = height * scale;
    context.scale(scale, scale);

    // The newest point is the last one. The history starts silent.
    const points = new Float32Array(POINTS);
    const step = width / POINTS;
    const draw = () => {
      context.clearRect(0, 0, width, height);
      context.fillStyle = "#22d3ee";
      points.forEach((point, index) => {
        const bar = meterPosition(point) * height;
        context.fillRect(index * step, height - bar, Math.max(step, 1), bar);
      });
    };
    draw();

    return runtime().on("history", (data) => {
      if (!Array.isArray(data)) return;
      // A point that was not a finite number arrives as null.
      const fresh = data.slice(-POINTS).map((point) => Number(point) || 0);
      points.copyWithin(0, fresh.length);
      points.set(fresh, POINTS - fresh.length);
      draw();
    });
  }, [width, height]);

  return (
    <canvas
      ref={canvasRef}
      style={{ width, height }}
      className="bg-slate-800 rounded-sm"
      aria-label="Output level history"
    />
  );
}
