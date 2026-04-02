import { useState } from "react";
import Slider from "./components/Slider";

interface PluginInfo {
  name: string;
  version: string;
  framework: string;
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

  return (
    <div className="relative flex flex-col items-center justify-center h-screen bg-slate-950 text-white font-sans select-none">
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

      <h1 className="text-3xl font-bold mb-2 text-[#7b68ee]">
        Beamer WebView Demo AUv2
      </h1>
      <p className="text-sm text-gray-400 mb-8">
        React 19 + Vite + Tailwind v4
      </p>

      <div className="flex gap-6">
        <Slider type="circular" paramId="gain" size={80} className="text-[#7b68ee]" />
        <Slider type="circular" paramId="pan" size={80} className="text-[#7b68ee]" />
      </div>
    </div>
  );
}

export default App;
