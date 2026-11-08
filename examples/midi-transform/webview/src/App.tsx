import { useSyncExternalStore } from "react";
import { noteName } from "@helpermedia/beamer-api";
import { useParam, useReady } from "@helpermedia/beamer-react";
// The Beamer logo for dark grounds, a copy of docs/branding/beamer-on-dark.svg
// so that the example works on its own.
import beamerLogo from "./beamer-logo.svg";
import CcCurve from "./components/CcCurve";
import Choices from "./components/Choices";
import KeyboardMap from "./components/KeyboardMap";
import Knob from "./components/Knob";
import Readout from "./components/Readout";
import Toggle from "./components/Toggle";
import { ccName, remapsCc, usesCcScale, usesNoteRemap, usesTranspose } from "./transform";
import type { CcSettings, NoteSettings } from "./transform";
import { subscribeTransformed, transformed } from "./transformed";
import type { Cc, Note } from "./transformed";

// A velocity or a CC value from 0 to 1 as MIDI writes it, 0 to 127.
function midiValue(value: number): string {
  return String(Math.round(value * 127));
}

// A CC number with its name, where it has one.
function ccText(controller: number): string {
  const name = ccName(controller);
  return name ? `${controller} ${name}` : String(controller);
}

function noteRows(note: Note | null) {
  if (note === null) return null;
  return {
    in: [noteName(note.pitch), midiValue(note.velocity)] as [string, string],
    out:
      note.outPitch === null
        ? (["Filtered", ""] as [string, string])
        : ([noteName(note.outPitch), midiValue(note.outVelocity)] as [string, string]),
  };
}

function ccRows(cc: Cc | null) {
  if (cc === null) return null;
  return {
    in: [ccText(cc.controller), midiValue(cc.value)] as [string, string],
    out: [ccText(cc.outController), midiValue(cc.outValue)] as [string, string],
  };
}

function App() {
  const ready = useReady();
  const passed = useSyncExternalStore(subscribeTransformed, transformed);
  const noteEnabled = useParam("note/enabled");
  const noteMode = useParam("note/mode");
  const transpose = useParam("note/transpose");
  const inputNote = useParam("note/input");
  const outputNote = useParam("note/output");
  const velocity = useParam("note/velocity");
  const ccEnabled = useParam("cc/enabled");
  const ccMode = useParam("cc/mode");
  const inputCc = useParam("cc/input");
  const outputCc = useParam("cc/output");
  const scale = useParam("cc/scale");
  const bypass = useParam("bypass");

  // The parameters arrive with `ready`. Until then the panel stays empty.
  if (!ready) return null;

  // The displays follow the parameters at once, also while the host does
  // not run the processor.
  const notes: NoteSettings = {
    enabled: noteEnabled.value >= 0.5,
    mode: noteMode.param.choice(),
    transpose: transpose.param.plain(),
    inputNote: inputNote.param.plain(),
    outputNote: outputNote.param.plain(),
    velocityScale: velocity.param.plain(),
  };
  const ccs: CcSettings = {
    enabled: ccEnabled.value >= 0.5,
    mode: ccMode.param.choice(),
    inputCc: inputCc.param.plain(),
    outputCc: outputCc.param.plain(),
    valueScale: scale.param.plain(),
  };
  // The editor has no bypass switch: the host's own sets the bypass
  // parameter. The displays dim while it is on.
  const lit = bypass.value < 0.5;

  return (
    <>
      {/* The bar spans the window; its content lines up with the panel. */}
      <header className="h-[3.25rem] shrink-0 border-b border-neutral-800 bg-bar">
        <div className="mx-auto flex h-full w-[47.5rem] items-center justify-between px-5">
          <h1 className="text-[0.9375rem] font-semibold tracking-tight">MIDI Transform</h1>
          <img src={beamerLogo} alt="Beamer" className="h-3.5" />
        </div>
      </header>

      {/* Notes above, CCs below. Each section has its switch and its
          modes, a display and the knobs its modes use; the others turn
          gray. With the bar the size matches the scale in index.css. Room
          left over under the bar is shared above and below the panel. */}
      <main className="mx-auto my-auto flex h-[29.25rem] w-[47.5rem] shrink-0 flex-col px-5 pt-4 pb-4">
        <section aria-label="Notes" className="flex flex-col gap-2.5">
          <div className="flex items-center gap-3">
            <Toggle paramId="note/enabled" label="Notes" />
            <div className="flex-1">
              <Choices paramId="note/mode" />
            </div>
          </div>
          <KeyboardMap settings={notes} held={passed.held} lit={lit} />
          <div className="flex items-start">
            <Knob paramId="note/transpose" unused={!usesTranspose(notes)} />
            <Knob paramId="note/input" label="Input" unused={!usesNoteRemap(notes)} />
            <Knob paramId="note/output" label="Output" unused={!usesNoteRemap(notes)} />
            <Knob paramId="note/velocity" unused={!notes.enabled} />
            <div className="ml-auto">
              <Readout columns={["Note", "Velocity"]} rows={noteRows(passed.lastNote)} />
            </div>
          </div>
        </section>

        <section aria-label="CCs" className="mt-auto flex gap-4">
          <CcCurve settings={ccs} last={passed.lastCc} lit={lit} />
          <div className="flex flex-1 flex-col gap-2.5">
            <div className="flex items-center gap-3">
              <Toggle paramId="cc/enabled" label="CCs" />
              <div className="flex-1">
                <Choices paramId="cc/mode" />
              </div>
            </div>
            <div className="flex items-start">
              <Knob paramId="cc/input" label="Input" detail={ccName(inputCc.param.plain())} unused={!remapsCc(ccs)} />
              <Knob paramId="cc/output" label="Output" detail={ccName(outputCc.param.plain())} unused={!remapsCc(ccs)} />
              <Knob paramId="cc/scale" detail="" unused={!usesCcScale(ccs)} />
              <div className="ml-auto">
                <Readout columns={["CC", "Value"]} rows={ccRows(passed.lastCc)} />
              </div>
            </div>
          </div>
        </section>
      </main>
    </>
  );
}

export default App;
