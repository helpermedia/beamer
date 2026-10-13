import { useEffect, useState } from "react";
import Slider from "./components/Slider";

interface PluginInfo {
  name: string;
  version: string;
  framework: string;
}

// State beyond parameters: the plugin keeps the pan law in a State field.
type PanLaw = "constant_power" | "linear";

function isPanLaw(value: unknown): value is PanLaw {
  return value === "constant_power" || value === "linear";
}

function App() {
  const [showAbout, setShowAbout] = useState(false);
  const [pluginInfo, setPluginInfo] = useState<PluginInfo | null>(null);

  const openAbout = () => {
    __BEAMER__
      .invoke("getInfo")
      .then((result) => {
        setPluginInfo(result as PluginInfo);
        setShowAbout(true);
      })
      .catch(() => {});
  };

  const closeAbout = () => setShowAbout(false);

  const [panLaw, setPanLaw] = useState<PanLaw | null>(null);

  // Read the pan law when the editor opens and keep following it: a host
  // can change it behind the UI by loading a preset or a project.
  useEffect(() => {
    let mounted = true;
    const read = () => {
      __BEAMER__
        .invoke("getPanLaw")
        .then((law) => {
          if (mounted && isPanLaw(law)) setPanLaw(law);
        })
        .catch((error) => console.error("getPanLaw failed:", error));
    };
    __BEAMER__.ready.then(read);
    const timer = setInterval(read, 500);
    return () => {
      mounted = false;
      clearInterval(timer);
    };
  }, []);

  const [sampleRate, setSampleRate] = useState<number | null>(null);

  // Ask for the sample rate when the editor opens, then follow the event
  // the plugin sends whenever the host prepares it again.
  useEffect(() => {
    let mounted = true;
    const show = (rate: unknown) => {
      if (mounted && typeof rate === "number" && rate > 0) setSampleRate(rate);
    };
    const unsubscribe = __BEAMER__.on("sampleRate", show);
    __BEAMER__.ready
      .then(() => __BEAMER__.invoke("getSampleRate"))
      .then(show)
      .catch((error) => console.error("getSampleRate failed:", error));
    return () => {
      mounted = false;
      unsubscribe();
    };
  }, []);

  const togglePanLaw = () => {
    const next: PanLaw = panLaw === "linear" ? "constant_power" : "linear";
    __BEAMER__
      .invoke("setPanLaw", next)
      .then(() => setPanLaw(next))
      .catch((error) => console.error("setPanLaw failed:", error));
  };

  return (
    <div className="relative flex flex-col items-center justify-center h-screen bg-slate-900 text-white font-sans select-none">
      <button
        onClick={openAbout}
        className="absolute top-3 right-3 text-xs text-gray-500 hover:text-gray-300 transition-colors cursor-pointer"
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
      <p className="text-sm text-gray-400 mb-8">
        React 19 + Vite + Tailwind v4
      </p>

      <div className="flex gap-6">
        <Slider type="circular" paramId="gain" size={80} />
        <Slider type="circular" paramId="pan" size={80} />
      </div>

      <button
        onClick={togglePanLaw}
        disabled={panLaw === null}
        className="mt-6 text-xs text-gray-400 hover:text-gray-200 transition-colors cursor-pointer disabled:cursor-default"
      >
        Pan law: {panLaw === "linear" ? "Linear" : "Constant power"}
      </button>

      <p className="absolute bottom-3 text-xs text-gray-500 font-mono">
        {sampleRate === null ? "Not prepared" : `${sampleRate} Hz`}
      </p>
    </div>
  );
}

export default App;
