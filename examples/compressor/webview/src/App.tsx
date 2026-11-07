import { useParam, useReady } from "@helpermedia/beamer-react";
// The Beamer logo for dark grounds, a copy of docs/branding/beamer-on-dark.svg
// so that the example works on its own.
import beamerLogo from "./beamer-logo.svg";
import AutoMakeup from "./components/AutoMakeup";
import Choices from "./components/Choices";
import Knob from "./components/Knob";
import ReductionMeter from "./components/ReductionMeter";
import Toggle from "./components/Toggle";
import TransferCurve from "./components/TransferCurve";
import { ratioOf, SOFT_KNEE_DB } from "./curve";
import type { Curve } from "./curve";
import { useMediaQuery } from "./hooks/useMediaQuery";
import { useStatus } from "./hooks/usePluginValues";
import type { Status } from "./hooks/usePluginValues";

// The latency the lookahead adds, in samples as the processor counts them
// (`ms_to_samples()` in src/lib.rs) once the sample rate is known.
function latencyText(lookaheadMs: number, lookaheadText: string, status: Status | null): string {
  if (status === null || status.sampleRate <= 0) {
    return lookaheadMs > 0 ? `Latency ${lookaheadText}` : "No latency";
  }
  const samples = Math.round(lookaheadMs * 0.001 * status.sampleRate);
  return samples > 0 ? `Latency ${lookaheadText}, ${samples} samples` : "No latency";
}

function App() {
  const ready = useReady();
  const status = useStatus();
  const threshold = useParam("threshold");
  const ratio = useParam("ratio");
  const softKnee = useParam("knee");
  const makeup = useParam("makeup");
  const lookahead = useParam("lookahead");
  const bypass = useParam("bypass");
  // The layout of a tall window, the same query as `portrait:` in the
  // classes and the scale in index.css.
  const portrait = useMediaQuery("(orientation: portrait)");

  // The parameters arrive with `ready`. Until then the panel stays empty.
  if (!ready) return null;

  // The curve follows the parameters at once, also while the host does not
  // run the processor.
  const curve: Curve = {
    thresholdDb: threshold.param.plain(),
    ratio: ratioOf(ratio.param.choices()[ratio.param.choice()]),
    kneeDb: softKnee.value >= 0.5 ? SOFT_KNEE_DB : 0,
    makeupDb: makeup.param.plain(),
  };
  // The editor has no bypass switch: the host's own sets the bypass
  // parameter (a VST3 host, an Audio Unit host that uses the bypass
  // property). The displays dim while it is on.
  const lit = bypass.value < 0.5;

  return (
    <>
      {/* The bar spans the window; its content lines up with the panel. */}
      <header className="h-[3.25rem] shrink-0 border-b border-neutral-800 bg-bar">
        <div className="mx-auto flex h-full w-[47.5rem] items-center justify-between px-5 portrait:w-[27.5rem]">
          <h1 className="text-[0.9375rem] font-semibold tracking-tight">Compressor</h1>
          <img src={beamerLogo} alt="Beamer" className="h-3.5" />
        </div>
      </header>

      {/* Two layouts: side by side in a wide window, the displays above the
          controls in a tall one. With the bar their sizes match the scale
          in index.css. Room left over under the bar is shared above and
          below the panel. */}
      <main className="mx-auto my-auto flex h-[23.5rem] w-[47.5rem] shrink-0 flex-col px-5 pt-4 pb-4 portrait:h-[39rem] portrait:w-[27.5rem]">
        <div className="flex gap-3 portrait:flex-col portrait:gap-6">
          {/* In a tall window the displays grow to the width of the controls. */}
          <div className="flex gap-3">
            <TransferCurve curve={curve} thresholdText={threshold.text} keyedOnSidechain={status?.keyedOnSidechain ?? false} lit={lit} />
            <ReductionMeter lit={lit} />
          </div>

          {portrait ? (
            // A tall window: the ratios, the knobs in one row and the
            // switches, each row the full width. Auto makeup sits on the
            // right, under the makeup knob.
            <div className="flex flex-col gap-5">
              <Choices paramId="ratio" />
              <div className="grid grid-cols-5 justify-items-center">
                <Knob paramId="threshold" />
                <Knob paramId="attack" />
                <Knob paramId="release" />
                <Knob paramId="lookahead" />
                <Knob paramId="makeup" label="Makeup" />
              </div>
              <div className="flex items-center justify-between">
                <Toggle paramId="knee" label="Soft knee" />
                <AutoMakeup gainFirst />
              </div>
            </div>
          ) : (
            <div className="ml-4 grid flex-1 grid-cols-3 content-between justify-items-center">
              <Knob paramId="threshold" />
              <div className="col-span-2 flex w-full flex-col justify-center gap-2.5">
                <Choices paramId="ratio" />
                <div>
                  <Toggle paramId="knee" label="Soft knee" />
                </div>
              </div>

              <Knob paramId="attack" />
              <Knob paramId="release" />
              <Knob paramId="lookahead" />

              <Knob paramId="makeup" label="Makeup" />
              <div className="col-span-2 flex w-full items-center">
                <AutoMakeup />
              </div>
            </div>
          )}
        </div>

        <footer className="mt-auto flex items-center justify-between text-xs">
          {/* What the compressor listens to: the track (internal) or the
              sidechain input (external). An on and off parameter, shown as
              the two choices. The curve's input axis tells which one it
              hears. */}
          <div className="flex items-center gap-2.5">
            <span className="text-[0.6875rem] text-neutral-400">Sidechain</span>
            <Choices paramId="sidechain" labels={["Internal", "External"]} />
          </div>
          <p className="text-neutral-500 tabular-nums">
            {latencyText(lookahead.param.plain(), lookahead.text, status)}
          </p>
        </footer>
      </main>
    </>
  );
}

export default App;
