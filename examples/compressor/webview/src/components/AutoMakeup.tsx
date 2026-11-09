import { useParam } from "@helpermedia/beamer-react";
import { useMeters } from "../hooks/usePluginValues";
import Switch from "./Switch";
import { withMinus } from "../format";

interface AutoMakeupProps {
  // Whether the gain shows before the switch instead of after it, for a
  // switch on the right edge.
  gainFirst?: boolean;
}

// The auto makeup switch and, while it is on, the gain it adds now. The
// gain follows the average reduction of the last second, so the audio
// thread sends it.
function AutoMakeup({ gainFirst = false }: AutoMakeupProps) {
  const { value } = useParam("auto_makeup");
  const { autoMakeupDb } = useMeters();

  return (
    <div className={`flex items-center gap-3 ${gainFirst ? "flex-row-reverse" : ""}`}>
      <Switch paramId="auto_makeup" label="Auto makeup" />
      {value >= 0.5 && (
        <span className="text-xs font-medium tabular-nums text-brand-blue" aria-label="Auto makeup gain">
          {autoMakeupDb >= 0 ? "+" : ""}
          {withMinus(autoMakeupDb.toFixed(1))} dB
        </span>
      )}
    </div>
  );
}

export default AutoMakeup;
