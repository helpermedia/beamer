import { noteName } from "@helpermedia/beamer-api";
import { isBlack, MIDDLE_C, NoteMode, transformPitch } from "../transform";
import type { NoteSettings } from "../transform";
import type { Note } from "../transformed";

// The display in SVG units: rem-sized pixels at the editor's own size.
const WIDTH = 720;
const HEIGHT = 120;
// Room on the left for the names of the keyboards.
const KEYS_X = 34;
const KEYS_RIGHT = WIDTH - 8;
// All 128 notes: 75 white keys.
const WHITE_KEYS = 75;
const WHITE = (KEYS_RIGHT - KEYS_X) / WHITE_KEYS;
const BLACK = WHITE * 0.62;

// The keyboard of what comes in hangs from the top, the one of what goes
// out stands on the bottom; their fronts face the lines between them.
const IN_TOP = 8;
const KEY_HEIGHT = 30;
const BLACK_HEIGHT = 18;
const IN_FRONT = IN_TOP + KEY_HEIGHT;
const OUT_BOTTOM = HEIGHT - 8;
const OUT_FRONT = OUT_BOTTOM - KEY_HEIGHT;

// The white keys below each note of an octave.
const WHITE_BELOW = [0, 0, 1, 1, 2, 3, 3, 4, 4, 5, 5, 6];

// The left edge of a white key, or the middle of a black one.
function keyX(pitch: number): number {
  const whites = Math.floor(pitch / 12) * 7 + WHITE_BELOW[pitch % 12];
  return KEYS_X + whites * WHITE + (isBlack(pitch) ? WHITE : 0);
}

// Where a line meets a key.
function keyCenter(pitch: number): number {
  return isBlack(pitch) ? keyX(pitch) : keyX(pitch) + WHITE / 2;
}

const PITCHES = Array.from({ length: 128 }, (_, pitch) => pitch);
const WHITE_PITCHES = PITCHES.filter((pitch) => !isBlack(pitch));
const BLACK_PITCHES = PITCHES.filter(isBlack);
// Every C, the notes the map draws a line for.
const OCTAVES = PITCHES.filter((pitch) => pitch % 12 === 0);

interface KeyboardProps {
  // Which edge of the keys faces the lines: their fronts.
  side: "in" | "out";
  // The notes that sound on this keyboard.
  lit: ReadonlySet<number>;
  // The notes this keyboard shows dark: incoming notes that are filtered out.
  dark?: ReadonlySet<number>;
}

function Keyboard({ side, lit, dark }: KeyboardProps) {
  const top = side === "in" ? IN_TOP : OUT_FRONT;
  const blackTop = side === "in" ? IN_TOP : OUT_BOTTOM - BLACK_HEIGHT;
  const fill = (pitch: number, base: string) =>
    lit.has(pitch) ? "fill-signal" : dark?.has(pitch) ? (isBlack(pitch) ? "fill-ink" : "fill-neutral-800") : base;

  return (
    <g>
      {WHITE_PITCHES.map((pitch) => (
        <rect
          key={pitch}
          x={keyX(pitch) + 0.5}
          y={top}
          width={WHITE - 1}
          height={KEY_HEIGHT}
          rx={1}
          className={`key ${fill(pitch, "fill-neutral-600")}`}
        />
      ))}
      {BLACK_PITCHES.map((pitch) => (
        <rect
          key={pitch}
          x={keyX(pitch) - BLACK / 2}
          y={blackTop}
          width={BLACK}
          height={BLACK_HEIGHT}
          rx={1}
          className={`key ${fill(pitch, "fill-neutral-900")} stroke-well`}
          strokeWidth={1}
        />
      ))}
      {/* Middle C, for finding one's way. */}
      <circle
        cx={keyCenter(MIDDLE_C)}
        cy={side === "in" ? IN_FRONT - 4 : OUT_FRONT + 4}
        r={1.6}
        className={`key ${lit.has(MIDDLE_C) ? "fill-ink" : "fill-neutral-900"}`}
      />
    </g>
  );
}

interface KeyboardMapProps {
  settings: NoteSettings;
  // The notes that sound, as they came in and went out. A map, not an
  // iterator: React renders twice in development and an iterator reads
  // only once.
  held: ReadonlyMap<string, Note>;
  // Whether the display is lit: dark while the plugin is bypassed.
  lit: boolean;
}

// Where each note goes: the incoming keyboard above, the outgoing one
// below and a line from every C to where it goes out, blue where the
// settings move it. The notes that sound light up with their own line.
// Incoming notes that would leave the MIDI range are dark: they are
// filtered out. While the plugin is bypassed the map turns gray, but the
// notes stay lit: they still pass, straight down.
//
// Text sizes are in the units of the viewBox, which scales with the
// window: a size in rem would grow with the window twice.
function KeyboardMap({ settings, held, lit }: KeyboardMapProps) {
  const litIn = new Set<number>();
  const litOut = new Set<number>();
  const playing: Note[] = [];
  for (const note of held.values()) {
    litIn.add(note.pitch);
    if (note.outPitch !== null) litOut.add(note.outPitch);
    playing.push(note);
  }
  const filtered = new Set(PITCHES.filter((pitch) => transformPitch(pitch, settings) === null));

  // The lines of the map: every C, plus the remapped note in Remap mode.
  const mapped = [...OCTAVES];
  if (settings.enabled && settings.mode === NoteMode.Remap && !mapped.includes(settings.inputNote)) {
    mapped.push(settings.inputNote);
  }

  return (
    <svg
      viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
      role="img"
      aria-label={`Note map${playing.length > 0 ? `, playing ${playing.map((note) => noteName(note.pitch)).join(" ")}` : ""}`}
      className="display w-full rounded-lg bg-well ring-1 ring-neutral-800 ring-inset"
      data-unlit={lit ? undefined : ""}
    >
      <text x={8} y={IN_TOP + KEY_HEIGHT / 2} dominantBaseline="central" fontSize={10} className="fill-neutral-500 font-medium">
        In
      </text>
      <text x={8} y={OUT_FRONT + KEY_HEIGHT / 2} dominantBaseline="central" fontSize={10} className="fill-neutral-500 font-medium">
        Out
      </text>

      {mapped.map((pitch) => {
        const out = transformPitch(pitch, settings);
        if (out === null) return null;
        const moved = out !== pitch;
        return (
          <line
            key={pitch}
            x1={keyCenter(pitch)}
            y1={IN_FRONT}
            x2={keyCenter(out)}
            y2={OUT_FRONT}
            className={moved ? "stroke-brand-blue" : "stroke-neutral-700"}
            strokeWidth={moved ? 1.25 : 1}
          />
        );
      })}

      {/* The notes that sound, over the lines of the map. */}
      {playing.map((note, index) =>
        note.outPitch === null ? null : (
          <line
            key={index}
            x1={keyCenter(note.pitch)}
            y1={IN_FRONT}
            x2={keyCenter(note.outPitch)}
            y2={OUT_FRONT}
            className="stroke-signal"
            strokeWidth={1.5}
            strokeLinecap="round"
          />
        ),
      )}

      <Keyboard side="in" lit={litIn} dark={filtered} />
      <Keyboard side="out" lit={litOut} />
    </svg>
  );
}

export default KeyboardMap;
