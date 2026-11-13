import { afterEach, expect, test } from "bun:test";
import { allNotesOff, noteName, noteOff, noteOn, playNote } from "../src/index";
import type { Beamer } from "../src/types";

function install(middleC: Beamer["middleC"]) {
  globalThis.__BEAMER__ = { middleC } as Beamer;
}

afterEach(() => {
  globalThis.__BEAMER__ = undefined;
});

test("names note 60 C3 by default and before ready", () => {
  install(null);
  expect(noteName(60)).toBe("C3");
  install("C3");
  expect([noteName(0), noteName(36), noteName(61), noteName(127)]).toEqual(["C-2", "C1", "C#3", "G8"]);
});

test("names note 60 C4 for a plugin with middle C as C4", () => {
  install("C4");
  expect([noteName(0), noteName(60), noteName(69), noteName(127)]).toEqual(["C-1", "C4", "A4", "G9"]);
});

// A stand-in for the runtime that records the notes it is asked to play.
function installPlayer(): string[] {
  const played: string[] = [];
  globalThis.__BEAMER__ = {
    middleC: "C3",
    noteOn: (note: number, velocity?: number, channel?: number) => played.push(`on ${note} ${velocity} ${channel}`),
    noteOff: (note: number, velocity?: number, channel?: number) => played.push(`off ${note} ${velocity} ${channel}`),
    allNotesOff: () => played.push("all off"),
  } as unknown as Beamer;
  return played;
}

test("noteOn and noteOff pass the note and its options on", () => {
  const played = installPlayer();
  noteOn(36);
  noteOn(38, { velocity: 0.5, channel: 9 });
  noteOff(38, { velocity: 0.25, channel: 9 });
  noteOff(36);
  allNotesOff();
  expect(played).toEqual([
    "on 36 undefined undefined",
    "on 38 0.5 9",
    "off 38 0.25 9",
    "off 36 undefined undefined",
    "all off",
  ]);
});

test("playNote ends the note after its duration", async () => {
  const played = installPlayer();
  playNote(36, { velocity: 0.8, channel: 9, duration: 10 });
  expect(played).toEqual(["on 36 0.8 9"]);
  await Bun.sleep(30);
  expect(played).toEqual(["on 36 0.8 9", "off 36 undefined 9"]);
});

test("playNote lasts 100 ms by default", async () => {
  const played = installPlayer();
  playNote(36);
  await Bun.sleep(60);
  expect(played).toEqual(["on 36 undefined undefined"]);
  await Bun.sleep(80);
  expect(played).toEqual(["on 36 undefined undefined", "off 36 undefined undefined"]);
});

test("a newer start of a note keeps an older timer from ending it", async () => {
  const played = installPlayer();
  playNote(36, { duration: 10 });
  playNote(36, { duration: 40 });
  // Another channel is another note.
  playNote(36, { channel: 1, duration: 10 });
  await Bun.sleep(25);
  expect(played).toEqual(["on 36 undefined undefined", "on 36 undefined undefined", "on 36 undefined 1", "off 36 undefined 1"]);
  await Bun.sleep(40);
  expect(played.at(-1)).toBe("off 36 undefined undefined");
  expect(played.length).toBe(5);

  // A note held with noteOn is not ended by the timer of a playNote before it.
  played.length = 0;
  playNote(40, { duration: 10 });
  noteOn(40);
  await Bun.sleep(25);
  expect(played).toEqual(["on 40 undefined undefined", "on 40 undefined undefined"]);
});

test("noteOff and allNotesOff end the timers of playNote", async () => {
  const played = installPlayer();
  playNote(36, { duration: 10 });
  noteOff(36);
  playNote(38, { duration: 10 });
  allNotesOff();
  await Bun.sleep(25);
  expect(played).toEqual(["on 36 undefined undefined", "off 36 undefined undefined", "on 38 undefined undefined", "all off"]);
});
