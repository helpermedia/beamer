import { useState } from 'react'

function App() {
  const [gain, setGain] = useState(0)

  return (
    <div className="h-screen bg-[#1a1a2e] text-gray-200 flex flex-col items-center justify-center select-none">
      <h1 className="text-2xl font-bold text-[#7b68ee] mb-1">
        Beamer WebView Demo AUv2
      </h1>
      <p className="text-sm text-gray-500 mb-8">
        React 19 + Vite + Tailwind v4
      </p>

      <div className="bg-[#16213e] rounded-xl p-6 w-72 shadow-lg">
        <label className="block text-xs text-gray-400 uppercase tracking-wider mb-3">
          Gain
        </label>
        <input
          type="range"
          min={-60}
          max={12}
          step={0.1}
          value={gain}
          onChange={(e) => setGain(parseFloat(e.target.value))}
          className="w-full accent-[#7b68ee]"
        />
        <div className="text-center mt-3 text-lg font-mono text-[#7b68ee]">
          {gain.toFixed(1)} dB
        </div>
      </div>
    </div>
  )
}

export default App
