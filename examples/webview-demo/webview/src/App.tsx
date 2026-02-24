import { useEffect, useState } from "react";

interface PluginInfo {
  name: string;
  version: string;
  framework: string;
}

function App() {
  const [gain, setGain] = useState(0);
  const [ready, setReady] = useState(false);
  const [pluginInfo, setPluginInfo] = useState<PluginInfo | null>(null);
  const [invokeError, setInvokeError] = useState<string | null>(null);

  useEffect(() => {
    __BEAMER__.ready.then(() => {
      setGain(__BEAMER__.params.get("gain"));
      setReady(true);
    });

    return __BEAMER__.params.on("gain", setGain);
  }, []);

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const v = parseFloat(e.target.value);
    setGain(v);
    __BEAMER__.params.set("gain", v);
  };

  const handleGetInfo = () => {
    setInvokeError(null);
    __BEAMER__
      .invoke("getInfo")
      .then((result) => setPluginInfo(result as PluginInfo))
      .catch((err) => setInvokeError(String(err)));
  };

  const info = __BEAMER__?.params?.info("gain");
  const min = info?.min ?? -60;
  const max = info?.max ?? 12;
  const displayValue = min + gain * (max - min);

  return (
    <div className="flex flex-col items-center justify-center h-screen bg-[#1a1a2e] text-white font-sans select-none">
      <h1 className="text-3xl font-bold mb-2 text-[#7b68ee]">
        Beamer WebView Demo
      </h1>
      <p className="text-sm text-gray-400 mb-8">
        React 19 + Vite + Tailwind v4
      </p>

      <div className="flex flex-col items-center gap-4 w-72">
        <label className="text-lg font-medium">Gain</label>

        <input
          type="range"
          min={0}
          max={1}
          step={0.001}
          value={gain}
          onMouseDown={() => __BEAMER__.params.beginEdit("gain")}
          onChange={handleChange}
          onMouseUp={() => __BEAMER__.params.endEdit("gain")}
          className="w-full h-2 bg-gray-700 rounded-lg appearance-none cursor-pointer accent-[#7b68ee]"
        />

        <span className="text-2xl font-mono text-[#7b68ee]">
          {displayValue.toFixed(1)} dB
        </span>

        {!ready && (
          <span className="text-xs text-gray-500">Connecting...</span>
        )}
      </div>

      {/* Invoke round-trip demo */}
      <div className="flex flex-col items-center gap-3 mt-8 w-72">
        <button
          onClick={handleGetInfo}
          className="px-4 py-2 bg-[#7b68ee] hover:bg-[#6a5acd] rounded text-sm font-medium transition-colors cursor-pointer"
        >
          invoke("getInfo")
        </button>

        {pluginInfo && (
          <div className="text-xs text-gray-300 bg-[#16213e] rounded px-4 py-3 w-full font-mono">
            <div>{pluginInfo.name}</div>
            <div>v{pluginInfo.version}</div>
            <div>{pluginInfo.framework}</div>
          </div>
        )}

        {invokeError && (
          <div className="text-xs text-red-400 bg-[#16213e] rounded px-4 py-3 w-full font-mono">
            Error: {invokeError}
          </div>
        )}
      </div>
    </div>
  );
}

export default App;
