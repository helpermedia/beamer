import { useRef, useState, useSyncExternalStore } from "react";
import type { KeyboardEvent } from "react";
import { noteName } from "@helpermedia/beamer-api";
import { useParameter, useReady, useStateField } from "@helpermedia/beamer-react";
// The Beamer logo for dark grounds, a copy of docs/branding/beamer-on-dark.svg
// so that the example works on its own.
import beamerLogo from "./beamer-logo.svg";
import CcCurve from "./components/CcCurve";
import Dial from "./components/Dial";
import KeyboardMap from "./components/KeyboardMap";
import LastEvent from "./components/LastEvent";
import RadioGroup from "./components/RadioGroup";
import ShapeChoice from "./components/ShapeChoice";
import Switch from "./components/Switch";
import VelocityCurve, { velocity } from "./components/VelocityCurve";
import { IDENTITY } from "./curve";
import type { Tab } from "./model-types";
import { ccName, remapsCc, usesCcScale, usesNoteRemap, usesTranspose } from "./transform";
import type { CcSettings, NoteSettings } from "./transform";
import { subscribeTransformed, transformed } from "./transformed";
import type { Cc, Note } from "./transformed";

const TABS: { id: Tab; label: string }[] = [
  { id: "notes_and_ccs", label: "Notes & CCs" },
  { id: "velocity", label: "Velocity" },
];

// A velocity or a CC value from 0 to 1 as MIDI writes it, 0 to 127.
function midiValue(value: number): string {
  return String(velocity(value));
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
  const curveEnabled = useParameter("velocity/enabled");
  const ccEnabled = useParameter("cc/enabled");
  const ccMode = useParameter("cc/mode");
  const inputCc = useParameter("cc/input");
  const outputCc = useParameter("cc/output");
  const scale = useParameter("cc/scale");
  const bypass = useParameter("bypass");
  // The tab the editor shows and the velocity curve, both `State` fields
  // saved with the project.
  const [tab, setTab] = useStateField("tab");
  const [savedCurve, setCurve] = useStateField("velocity/curve");
  // The point of the curve being dragged or focused.
  const [activePoint, setActivePoint] = useState<number | null>(null);
  const tabButtons = useRef<(HTMLButtonElement | null)[]>([]);

  // The parameters arrive with `ready`. Until then the panel stays empty.
  if (!ready) return null;

  const shown: Tab = tab ?? "notes_and_ccs";
  const curve = savedCurve ?? IDENTITY;
  const point = activePoint === null ? undefined : curve.points[activePoint];

  // The arrow keys go to the tab before or after, around at the ends; Home
  // and End go to the first and the last. The tab they reach shows at once.
  // Shortcuts with Command, Control or Alt belong to the page.
  function onTabKeyDown(event: KeyboardEvent, index: number) {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const last = TABS.length - 1;
    const next =
      event.key === "ArrowRight"
        ? (index + 1) % TABS.length
        : event.key === "ArrowLeft"
          ? (index + last) % TABS.length
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? last
              : -1;
    if (next < 0) return;
    event.preventDefault();
    setTab(TABS[next].id);
    tabButtons.current[next]?.focus();
  }

  // The displays follow the parameters at once, also while the host does
  // not run the processor.
  const notes: NoteSettings = {
    enabled: noteEnabled.value >= 0.5,
    mode: noteMode.parameter.choice(),
    transpose: transpose.parameter.plain(),
    inputNote: inputNote.parameter.plain(),
    outputNote: outputNote.parameter.plain(),
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
  const isIdentity = JSON.stringify(curve) === JSON.stringify(IDENTITY);

  return (
    <>
      {/* The bar spans the window; its content lines up with the panel. */}
      <header className="h-[3.25rem] shrink-0 border-b border-neutral-800 bg-bar">
        <div className="mx-auto flex h-full w-[47.5rem] items-center px-5">
          <h1 className="text-[0.9375rem] font-semibold tracking-tight">MIDI Transform</h1>
          {/* A segmented control in a well, as the drum synth's, so that
              the tabs read as a control, not as a title. */}
          <div role="tablist" aria-label="View" className="ml-6 flex gap-0.5 rounded-md bg-panel p-0.5 ring-1 ring-neutral-800 ring-inset">
            {TABS.map(({ id, label }, index) => (
              <button
                key={id}
                ref={(element) => {
                  tabButtons.current[index] = element;
                }}
                type="button"
                role="tab"
                id={`tab-${id}`}
                aria-controls={`panel-${id}`}
                aria-selected={shown === id}
                tabIndex={shown === id ? 0 : -1}
                className="rounded px-3 py-1 text-xs font-medium text-neutral-400 outline-none hover:text-paper focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-solid focus-visible:outline-brand-blue aria-selected:bg-neutral-700 aria-selected:text-paper"
                onClick={() => setTab(id)}
                onKeyDown={(event) => onTabKeyDown(event, index)}
              >
                {label}
              </button>
            ))}
          </div>
          <img src={beamerLogo} alt="Beamer" className="ml-auto h-3.5" />
        </div>
      </header>

      <main className="mx-auto my-auto h-[29.25rem] w-[47.5rem] shrink-0 px-5 pt-4 pb-4">
        {/* Notes above, CCs below. Each section has its switch and its
            modes, a display and the dials its modes use; the others turn
            gray. With the bar the size matches the scale in index.css.
            Room left over under the bar is shared above and below the
            panel. */}
        <div role="tabpanel" id="panel-notes_and_ccs" aria-labelledby="tab-notes_and_ccs" hidden={shown !== "notes_and_ccs"} className="flex h-full flex-col">
          <section aria-label="Notes" className="flex flex-col gap-2.5">
            <div className="flex items-center gap-3">
              <Switch parameterId="note/enabled" label="Notes" />
              <div className="flex-1">
                <RadioGroup parameterId="note/mode" />
              </div>
            </div>
            <KeyboardMap settings={notes} held={passed.held} lit={lit} />
            <div className="flex items-start">
              <Dial parameterId="note/transpose" unused={!usesTranspose(notes)} />
              <Dial parameterId="note/input" label="Input" unused={!usesNoteRemap(notes)} />
              <Dial parameterId="note/output" label="Output" unused={!usesNoteRemap(notes)} />
              <div className="ml-auto">
                <LastEvent columns={["Note", "Velocity"]} rows={noteRows(passed.lastNote)} />
              </div>
            </div>
          </section>

          <section aria-label="CCs" className="mt-auto flex gap-4">
            <CcCurve settings={ccs} last={passed.lastCc} lit={lit} />
            <div className="flex flex-1 flex-col gap-2.5">
              <div className="flex items-center gap-3">
                <Switch parameterId="cc/enabled" label="CCs" />
                <div className="flex-1">
                  <RadioGroup parameterId="cc/mode" />
                </div>
              </div>
              <div className="flex items-start">
                <Dial parameterId="cc/input" label="Input" detail={ccName(inputCc.parameter.plain())} unused={!remapsCc(ccs)} />
                <Dial parameterId="cc/output" label="Output" detail={ccName(outputCc.parameter.plain())} unused={!remapsCc(ccs)} />
                <Dial parameterId="cc/scale" detail="" unused={!usesCcScale(ccs)} />
                <div className="ml-auto">
                  <LastEvent columns={["CC", "Value"]} rows={ccRows(passed.lastCc)} />
                </div>
              </div>
            </div>
          </section>
        </div>

        {/* The velocity curve with its points to drag. Beside it its
            switch, its shape, the values of the point in hand and the last
            note. */}
        <div role="tabpanel" id="panel-velocity" aria-labelledby="tab-velocity" hidden={shown !== "velocity"} className="flex h-full gap-6">
          <VelocityCurve
            curve={curve}
            onChange={setCurve}
            onActive={setActivePoint}
            last={passed.lastNote}
            lit={lit}
            used={curveEnabled.value >= 0.5}
          />
          <div className="flex flex-1 flex-col gap-3">
            <div className="flex items-center gap-3">
              <Switch parameterId="velocity/enabled" label="Velocity curve" titled={false} />
              <div className="flex-1">
                <ShapeChoice shape={curve.shape} onChange={(shape) => setCurve({ ...curve, shape })} />
              </div>
            </div>
            <dl className="flex gap-4 px-2 text-[0.6875rem]">
              <div className="flex gap-2">
                <dt className="text-neutral-500">Point in</dt>
                <dd className={`font-medium tabular-nums ${point ? "text-paper" : "text-neutral-600"}`}>{point ? velocity(point.x) : "-"}</dd>
              </div>
              <div className="flex gap-2">
                <dt className="text-neutral-500">Out</dt>
                <dd className={`font-medium tabular-nums ${point ? "text-paper" : "text-neutral-600"}`}>{point ? velocity(point.y) : "-"}</dd>
              </div>
            </dl>
            <p className="px-2 text-[0.6875rem] leading-relaxed text-neutral-500">
              Click to add a point and drag it where it goes. Double-click a point to remove it. A focused point moves with
              the arrow keys and Delete removes it. The curve shapes how hard each note starts while it is on.
            </p>
            <button
              type="button"
              disabled={isIdentity}
              className="self-start rounded-lg bg-well px-2.5 py-1.5 text-xs font-medium text-paper ring-1 ring-neutral-800 ring-inset outline-none hover:bg-neutral-800 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-solid focus-visible:outline-brand-blue disabled:text-neutral-600 disabled:hover:bg-well"
              onClick={() => setCurve(IDENTITY)}
            >
              Reset Curve
            </button>
            <div className="mt-auto">
              <LastEvent columns={["Note", "Velocity"]} rows={noteRows(passed.lastNote)} />
            </div>
          </div>
        </div>
      </main>
    </>
  );
}

export default App;
