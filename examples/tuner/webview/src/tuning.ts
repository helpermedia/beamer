// From the pitches the processor sends to what the display shows. The
// processor measures a frequency about 50 times a second while the editor
// is open; the note and the cents are worked out here, with the reference
// of the moment, so the display follows the reference dial at once.

/** How long a pitch counts after it arrives, in ms. */
const LIVE_MS = 200;

/** How long the last note stays, dimmed, once no pitch comes, in ms. */
const HOLD_MS = 1500;

/** The time constant of the needle's smoothing, in ms. */
const SMOOTHING_MS = 100;

/** A jump of more than this, in octaves (a semitone), snaps instead of gliding. */
const SNAP_OCTAVES = 1 / 12;

/** How far past the middle between two notes the note changes, in semitones. */
const HYSTERESIS = 0.1;

/** How many stripes of the strobe pass a second per cent off. */
const STROBE_STRIPES_PER_CENT = 0.1;

/** Within this many cents the note counts as in tune. */
export const IN_TUNE_CENTS = 2;

/** What the display shows in a frame. */
export interface Shown {
  /** A pitch is coming, the last note is held after one, or nothing is. */
  phase: "live" | "held" | "rest";
  /** The MIDI note number named. */
  note: number;
  /** How far the pitch is from the note, in cents: up to 60 either way with the hysteresis. */
  cents: number;
  /** The smoothed frequency in Hz. */
  frequencyHz: number;
  /** Where the strobe's stripes are, as a part of one stripe from 0 to 1. */
  strobe: number;
}

export class Tuning {
  // The last three frequencies, for their median: a single wrong reading
  // does not move the needle.
  #recent: number[] = [];
  // When the last pitch with a frequency arrived.
  #arrived = -Infinity;
  // The smoothed pitch in octaves (log2 of the frequency), null at rest.
  // In octaves rather than in notes, so it does not depend on the reference.
  #pitch: number | null = null;
  #note = 69;
  #strobe = 0;
  #lastFrame: number | null = null;

  /**
   * A pitch arrived at `now` (ms): its frequency, 0 for none. Returns
   * whether it counts, so the display knows whether something moves.
   */
  arrive(frequencyHz: number, now: number): boolean {
    if (frequencyHz <= 0) return false;
    // A note after a pause starts its own median.
    if (now - this.#arrived > LIVE_MS) this.#recent = [];
    this.#recent.push(frequencyHz);
    if (this.#recent.length > 3) this.#recent.shift();
    this.#arrived = now;
    return true;
  }

  /** What to show at `now` (ms) with the reference pitch of A4 in Hz. */
  frame(now: number, referenceHz: number): Shown {
    const elapsed = Math.min(100, now - (this.#lastFrame ?? now));
    this.#lastFrame = now;
    const since = now - this.#arrived;
    const phase = since <= LIVE_MS ? "live" : since <= LIVE_MS + HOLD_MS ? "held" : "rest";

    if (phase === "live") {
      const target = Math.log2(median(this.#recent));
      if (this.#pitch === null || Math.abs(target - this.#pitch) > SNAP_OCTAVES) {
        this.#pitch = target;
        this.#note = Math.round(this.#noteNumber(referenceHz));
      } else {
        this.#pitch += (target - this.#pitch) * (1 - Math.exp(-elapsed / SMOOTHING_MS));
      }
    } else if (phase === "rest") {
      this.#pitch = null;
    }
    if (this.#pitch === null) {
      // The next frame after rest starts without time gone by, so the
      // strobe does not jump.
      this.#lastFrame = null;
      return { phase: "rest", note: 69, cents: 0, frequencyHz: 0, strobe: this.#strobe };
    }

    const noteNumber = this.#noteNumber(referenceHz);
    if (Math.abs(noteNumber - this.#note) > 0.5 + HYSTERESIS) this.#note = Math.round(noteNumber);
    const cents = 100 * (noteNumber - this.#note);
    // The stripes move only while a pitch comes.
    if (phase === "live") {
      this.#strobe = (((this.#strobe + (cents * STROBE_STRIPES_PER_CENT * elapsed) / 1000) % 1) + 1) % 1;
    }
    return { phase, note: this.#note, cents, frequencyHz: 2 ** this.#pitch, strobe: this.#strobe };
  }

  // The smoothed pitch as a MIDI note number, A4 (69) at the reference.
  #noteNumber(referenceHz: number): number {
    return 69 + 12 * ((this.#pitch ?? 0) - Math.log2(referenceHz));
  }
}

// The median of up to three values. Of two the older one: a second reading
// an octave off after a pause does not show at once.
function median(values: number[]): number {
  if (values.length < 3) return values[0];
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[1];
}
