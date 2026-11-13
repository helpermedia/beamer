import { useSyncExternalStore } from "react";
import { noteName } from "@helpermedia/beamer-api";
import { useParameter, useReady } from "@helpermedia/beamer-react";
// The Beamer logo for dark grounds, a copy of docs/branding/beamer-on-dark.svg
// so that the example works on its own.
import beamerLogo from "./beamer-logo.svg";
import CcCurve from "./components/CcCurve";
import Choices from "./components/Choices";
import KeyboardMap from "./components/KeyboardMap";
import Dial from "./components/Dial";
import Readout from "./components/Readout";
import Switch from "./components/Switch";
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
  const noteEnabled = useParameter("note/enabled");
  const noteMode = useParameter("note/mode");
  const transpose = useParameter("note/transpose");
  const inputNote = useParameter("note/input");
  const outputNote = useParameter("note/output");
  const velocity = useParameter("note/velocity");
  const ccEnabled = useParameter("cc/enabled");
  const ccMode = useParameter("cc/mode");
  const inputCc = useParameter("cc/input");
  const outputCc = useParameter("cc/output");
  const scale = useParameter("cc/scale");
  const bypass = useParameter("bypass");

  // The parameters arrive with `ready`. Until then the panel stays empty.
  if (!ready) return null;

  // The displays follow the parameters at once, also while the host does
  // not run the processor.
  const notes: NoteSettings = {
    enabled: noteEnabled.value >= 0.5,
    mode: noteMode.parameter.choice(),
    transpose: transpose.parameter.plain(),
    inputNote: inputNote.parameter.plain(),
    outputNote: outputNote.parameter.plain(),
    velocityScale: velocity.parameter.plain(),
  };
  const ccs: CcSettings = {
    enabled: ccEnabled.value >= 0.5,
    mode: ccMode.parameter.choice(),
    inputCc: inputCc.parameter.plain(),
    outputCc: outputCc.parameter.plain(),
    valueScale: scale.parameter.plain(),
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
          modes, a display and the dials its modes use; the others turn
          gray. With the bar the size matches the scale in index.css. Room
          left over under the bar is shared above and below the panel. */}
      <main className="mx-auto my-auto flex h-[29.25rem] w-[47.5rem] shrink-0 flex-col px-5 pt-4 pb-4">
        <section aria-label="Notes" className="flex flex-col gap-2.5">
          <div className="flex items-center gap-3">
            <Switch parameterId="note/enabled" label="Notes" />
            <div className="flex-1">
              <Choices parameterId="note/mode" />
            </div>
          </div>
          <KeyboardMap settings={notes} held={passed.held} lit={lit} />
          <div className="flex items-start">
            <Dial parameterId="note/transpose" unused={!usesTranspose(notes)} />
            <Dial parameterId="note/input" label="Input" unused={!usesNoteRemap(notes)} />
            <Dial parameterId="note/output" label="Output" unused={!usesNoteRemap(notes)} />
            <Dial parameterId="note/velocity" unused={!notes.enabled} />
            <div className="ml-auto">
              <Readout columns={["Note", "Velocity"]} rows={noteRows(passed.lastNote)} />
            </div>
          </div>
        </section>

        <section aria-label="CCs" className="mt-auto flex gap-4">
          <CcCurve settings={ccs} last={passed.lastCc} lit={lit} />
          <div className="flex flex-1 flex-col gap-2.5">
            <div className="flex items-center gap-3">
              <Switch parameterId="cc/enabled" label="CCs" />
              <div className="flex-1">
                <Choices parameterId="cc/mode" />
              </div>
            </div>
            <div className="flex items-start">
              <Dial parameterId="cc/input" label="Input" detail={ccName(inputCc.parameter.plain())} unused={!remapsCc(ccs)} />
              <Dial parameterId="cc/output" label="Output" detail={ccName(outputCc.parameter.plain())} unused={!remapsCc(ccs)} />
              <Dial parameterId="cc/scale" detail="" unused={!usesCcScale(ccs)} />
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
