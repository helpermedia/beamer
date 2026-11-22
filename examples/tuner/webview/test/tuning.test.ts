import { expect, test } from "bun:test";
import { Tuning } from "../src/tuning";

// The ratio of `cents`.
const ratio = (cents: number) => 2 ** (cents / 1200);

// How far `hz` is from MIDI note `note` in cents, with A4 at `referenceHz`.
const centsFrom = (hz: number, note: number, referenceHz = 440) =>
  1200 * Math.log2(hz / (referenceHz * 2 ** ((note - 69) / 12)));

// Pitches of `hz` every 20 ms from `start` for `ms`, as the processor sends
// them, with a frame every 16 ms, as a screen refreshes. Returns what the
// last frame shows.
function play(tuning: Tuning, hz: number, start: number, ms: number, referenceHz = 440) {
  let shown = tuning.frame(start, referenceHz);
  for (let now = start; now <= start + ms; now += 4) {
    if ((now - start) % 20 === 0) tuning.arrive(hz, now);
    if ((now - start) % 16 === 0) shown = tuning.frame(now, referenceHz);
  }
  return shown;
}

test("a new note snaps once the median follows it", () => {
  const tuning = new Tuning();
  let shown = play(tuning, 82.41 * ratio(-18), 0, 500);
  expect(shown.phase).toBe("live");
  expect(shown.note).toBe(40);
  expect(Math.round(shown.cents)).toBe(-18);
  // E2 to A4 10 cents sharp, without a pause.
  shown = play(tuning, 440 * ratio(10), 504, 100);
  expect(shown.note).toBe(69);
  expect(shown.cents).toBeCloseTo(10, 0);
});

test("within a note the needle glides and settles", () => {
  const tuning = new Tuning();
  play(tuning, 440, 0, 300);
  const gliding = play(tuning, 440 * ratio(20), 304, 48);
  expect(gliding.cents).toBeGreaterThan(5);
  expect(gliding.cents).toBeLessThan(19);
  const settled = play(tuning, 440 * ratio(20), 352, 800);
  expect(settled.cents).toBeCloseTo(20, 1);
});

test("the note changes 10 cents past the middle between two notes", () => {
  const tuning = new Tuning();
  play(tuning, 440, 0, 300);
  let shown = play(tuning, 440 * ratio(55), 304, 800);
  expect(shown.note).toBe(69);
  expect(shown.cents).toBeCloseTo(55, 0);
  shown = play(tuning, 440 * ratio(65), 1108, 800);
  expect(shown.note).toBe(70);
  expect(shown.cents).toBeCloseTo(-35, 0);
});

test("the last note is held for 1.5 s, then the display rests", () => {
  const tuning = new Tuning();
  play(tuning, 110, 0, 300);
  expect(tuning.frame(450, 440).phase).toBe("live");
  expect(tuning.frame(550, 440).phase).toBe("held");
  // The reference applies to the held note at once.
  const held = tuning.frame(1300, 442.5);
  expect(held.phase).toBe("held");
  expect(held.cents).toBeCloseTo(centsFrom(110, 45, 442.5), 1);
  expect(tuning.frame(2050, 440).phase).toBe("rest");
});

test("pitches without a frequency do not count", () => {
  const tuning = new Tuning();
  play(tuning, 0, 0, 300);
  expect(tuning.frame(300, 440).phase).toBe("rest");
});

test("the strobe drifts up when sharp, down when flat and stands in tune", () => {
  // 3 cents off: 0.3 stripes a second, 0.15 in 0.5 s.
  const sharp = new Tuning();
  play(sharp, 440 * ratio(3), 0, 300);
  const sharpFrom = play(sharp, 440 * ratio(3), 304, 0).strobe;
  const sharpTo = play(sharp, 440 * ratio(3), 308, 500).strobe;
  expect((sharpTo - sharpFrom + 1) % 1).toBeCloseTo(0.15, 1);

  const flat = new Tuning();
  play(flat, 440 * ratio(-3), 0, 300);
  const flatFrom = play(flat, 440 * ratio(-3), 304, 0).strobe;
  const flatTo = play(flat, 440 * ratio(-3), 308, 500).strobe;
  expect((flatFrom - flatTo + 1) % 1).toBeCloseTo(0.15, 1);

  const tuned = new Tuning();
  play(tuned, 440, 0, 300);
  const tunedFrom = play(tuned, 440, 304, 0).strobe;
  const tunedTo = play(tuned, 440, 308, 500).strobe;
  expect(Math.abs(tunedTo - tunedFrom)).toBeLessThan(1e-9);
});

test("a single wrong reading does not move the needle", () => {
  const tuning = new Tuning();
  play(tuning, 440, 0, 300);
  tuning.arrive(880, 304);
  // The frame right after the wrong reading, before the next right one.
  const shown = tuning.frame(308, 440);
  expect(shown.note).toBe(69);
  expect(Math.abs(shown.cents)).toBeLessThan(0.5);
});

test("after a pause a second reading an octave off does not show", () => {
  const tuning = new Tuning();
  expect(tuning.arrive(110, 0)).toBe(true);
  tuning.arrive(220, 20);
  expect(tuning.frame(24, 440).note).toBe(45);
  expect(tuning.arrive(0, 40)).toBe(false);
});

test("the strobe does not jump on the first frame after rest", () => {
  const tuning = new Tuning();
  play(tuning, 440 * ratio(40), 0, 300);
  const resting = tuning.frame(3000, 440);
  expect(resting.phase).toBe("rest");
  tuning.arrive(440 * ratio(40), 3020);
  const first = tuning.frame(3024, 440);
  expect(first.phase).toBe("live");
  expect(Math.abs(first.strobe - resting.strobe)).toBeLessThan(1e-9);
});
