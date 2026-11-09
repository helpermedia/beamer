import { useEffect, useState } from "react";
import { runtime } from "@helpermedia/beamer-api";
import { useEvent, useReady, useStateField } from "@helpermedia/beamer-react";
import LevelHistory from "./components/LevelHistory";
import LevelMeter from "./components/LevelMeter";
import Dial from "./components/Dial";

interface PluginInfo {
  name: string;
  version: string;
  framework: string;
}

// State beyond parameters: the plugin keeps the pan law in a State field,
// which serde writes as one of these strings.
type PanLaw = "constant_power" | "linear";

function App() {
  const [showAbout, setShowAbout] = useState(false);
  const [pluginInfo, setPluginInfo] = useState<PluginInfo | null>(null);

  // Ask for the name and version when the editor opens. A call needs no
  // `ready`: the plugin answers it while the parameters are on their way.
  useEffect(() => {
    let mounted = true;
    runtime()
      .invoke("getInfo")
      .then((result) => {
        if (mounted) setPluginInfo(result as PluginInfo);
      })
      .catch((error) => console.error("getInfo failed:", error));
    return () => {
      mounted = false;
    };
  }, []);

  const openAbout = () => setShowAbout(true);
  const closeAbout = () => setShowAbout(false);

  // The pan law follows the plugin, also when a host changes it behind
  // the UI by loading a project.
  const [panLaw, setPanLaw] = useStateField<PanLaw>("pan_law");

  const [sampleRate, setSampleRate] = useState<number | null>(null);

  // The format the editor runs in, known once the runtime is ready.
  const ready = useReady();
  const formatNames = { vst3: "VST3", auv2: "AUv2", auv3: "AUv3" };
  const format = ready ? runtime().pluginFormat : null;
  const pluginFormat = format ? formatNames[format] : null;

  // Ask for the sample rate when the editor opens, then follow the event
  // the plugin sends whenever the host prepares it again.
  const showSampleRate = (rate: unknown) => {
    if (typeof rate === "number" && rate > 0) setSampleRate(rate);
  };
  useEvent("sampleRate", showSampleRate);
  useEffect(() => {
    let mounted = true;
    runtime()
      .invoke("getSampleRate")
      .then((rate) => {
        if (mounted) showSampleRate(rate);
      })
      .catch((error) => console.error("getSampleRate failed:", error));
    return () => {
      mounted = false;
    };
  }, []);

  const togglePanLaw = () => setPanLaw(panLaw === "linear" ? "constant_power" : "linear");

  return (
    <div className="relative flex flex-col items-center justify-center h-screen bg-slate-900 text-white font-sans select-none">
      <button
        onClick={openAbout}
        disabled={pluginInfo === null}
        className="absolute top-3 right-3 text-xs text-gray-500 hover:text-gray-300 transition-colors cursor-pointer disabled:cursor-default"
      >
        About
      </button>

      {showAbout && pluginInfo && (
        <div className="absolute inset-0 flex items-center justify-center z-10 bg-black/75" onClick={closeAbout}>
          <div className="text-sm text-gray-300 bg-slate-800 rounded px-8 py-6 font-mono text-center">
            <div className="text-sm text-white mb-1">{pluginInfo.name}</div>
            <div>v{pluginInfo.version}</div>
            <div className="text-gray-500 mt-1">{pluginInfo.framework}</div>
          </div>
        </div>
      )}

      <h1 className="text-3xl font-bold mb-2 text-cyan-400">
        Beamer WebView Demo
      </h1>
      <p className="text-sm text-gray-400 mb-6">
        React 19 + Vite + Tailwind v4
      </p>

      <div className="flex gap-6">
        <Dial paramId="gain" size={80} />
        <Dial paramId="pan" size={80} />
      </div>

      <button
        onClick={togglePanLaw}
        disabled={panLaw === undefined}
        className="mt-6 text-xs text-gray-400 hover:text-gray-200 transition-colors cursor-pointer disabled:cursor-default"
      >
        Pan law: {panLaw === "linear" ? "Linear" : "Constant power"}
      </button>

      <div className="mt-4 flex items-end gap-3">
        <LevelMeter />
        <LevelHistory />
      </div>

      <p className="absolute bottom-3 text-xs text-gray-500 font-mono">
        {[
          pluginFormat,
          sampleRate === null ? "Not prepared" : `${sampleRate} Hz`,
          pluginInfo && `v${pluginInfo.version}`,
        ]
          .filter(Boolean)
          .join(" · ")}
      </p>
    </div>
  );
}

export default App;
