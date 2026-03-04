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

  const handleAbout = () => {
    if (showAbout) {
      setShowAbout(false);
      return;
    }
    __BEAMER__
      .invoke("getInfo")
      .then((result) => {
        setPluginInfo(result as PluginInfo);
        setShowAbout(true);
      })
      .catch(() => {});
  };

  return (
    <div className="relative flex flex-col items-center justify-center h-screen bg-slate-950 text-white font-sans select-none">
      <div className="absolute top-3 right-3">
        <button
          onClick={handleAbout}
          className="text-xs text-gray-500 hover:text-gray-300 transition-colors cursor-pointer"
        >
          About
        </button>

        {showAbout && pluginInfo && (
          <div className="text-xs text-gray-300 bg-slate-800 rounded px-4 py-3 mt-1 w-48 font-mono text-right">
            <div>{pluginInfo.name}</div>
            <div>v{pluginInfo.version}</div>
            <div>{pluginInfo.framework}</div>
          </div>
        )}
      </div>

      <h1 className="text-3xl font-bold mb-2 text-[#7b68ee]">
        Beamer WebView Demo AUv2
      </h1>
      <p className="text-sm text-gray-400 mb-8">
        React 19 + Vite + Tailwind v4
      </p>

      <Slider type="circular" paramId="gain" size={80} className="text-[#7b68ee]" />
    </div>
  );
}

export default App;
