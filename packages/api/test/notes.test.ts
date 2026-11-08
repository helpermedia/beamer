import { afterEach, expect, test } from "bun:test";
import { noteName } from "../src/index";
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
