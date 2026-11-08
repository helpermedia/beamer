// The rules of the processor (`MidiTransformProcessor` in src/lib.rs) in
// TypeScript, so that the displays follow a control at once, also while
// the host does not run the processor.

// The note modes in the order of `NoteTransformMode`, by index of choice.
export const NoteMode = {
  Through: 0,
  Transpose: 1,
  OctaveUp: 2,
  OctaveDown: 3,
  Remap: 4,
  Invert: 5,
} as const;

// The CC modes in the order of `CcTransformMode`, by index of choice.
export const CcMode = {
  Through: 0,
  Remap: 1,
  Scale: 2,
  Invert: 3,
  RemapAndScale: 4,
} as const;

// The note settings as plain values.
export interface NoteSettings {
  enabled: boolean;
  mode: number;
  transpose: number;
  inputNote: number;
  outputNote: number;
  velocityScale: number;
}

// The CC settings as plain values.
export interface CcSettings {
  enabled: boolean;
  mode: number;
  inputCc: number;
  outputCc: number;
  valueScale: number;
}

// Middle C, the axis of the inversion.
export const MIDDLE_C = 60;

// The pitch a note goes out with, or null when it falls outside the MIDI
// range and is filtered out (`transform_pitch()`).
export function transformPitch(pitch: number, settings: NoteSettings): number | null {
  if (!settings.enabled) return pitch;
  let out = pitch;
  switch (settings.mode) {
    case NoteMode.Transpose:
      out = pitch + settings.transpose;
      break;
    case NoteMode.OctaveUp:
      out = pitch + 12;
      break;
    case NoteMode.OctaveDown:
      out = pitch - 12;
      break;
    case NoteMode.Remap:
      out = pitch === settings.inputNote ? settings.outputNote : pitch;
      break;
    case NoteMode.Invert:
      out = MIDDLE_C + (MIDDLE_C - pitch);
      break;
  }
  return out >= 0 && out <= 127 ? out : null;
}

// Whether the CC value of `controller` changes (`transform_cc_value()`).
export function changesCcValue(controller: number, settings: CcSettings): boolean {
  if (!settings.enabled) return false;
  switch (settings.mode) {
    case CcMode.Scale:
    case CcMode.Invert:
      return true;
    case CcMode.RemapAndScale:
      return controller === settings.inputCc;
    default:
      return false;
  }
}

// Whether the CC number changes: the input CC goes out as the output CC
// (`transform_cc_number()`).
export function remapsCc(settings: CcSettings): boolean {
  return settings.enabled && (settings.mode === CcMode.Remap || settings.mode === CcMode.RemapAndScale);
}

// A CC value from 0 to 1 as it goes out, where the value changes.
export function transformCcValue(value: number, settings: CcSettings): number {
  if (settings.mode === CcMode.Invert) return 1 - value;
  return Math.min(1, Math.max(0, value * settings.valueScale));
}

// Which note settings the mode uses, for dimming the others.
export function usesTranspose(settings: NoteSettings): boolean {
  return settings.enabled && settings.mode === NoteMode.Transpose;
}

export function usesNoteRemap(settings: NoteSettings): boolean {
  return settings.enabled && settings.mode === NoteMode.Remap;
}

// Which CC settings the mode uses.
export function usesCcScale(settings: CcSettings): boolean {
  return settings.enabled && (settings.mode === CcMode.Scale || settings.mode === CcMode.RemapAndScale);
}

// Whether a MIDI note is a black key.
export function isBlack(pitch: number): boolean {
  return [1, 3, 6, 8, 10].includes(((pitch % 12) + 12) % 12);
}

// The names of the CCs that have one in the MIDI specification and that
// players meet most.
const CC_NAMES: Record<number, string> = {
  0: "Bank Select",
  1: "Mod Wheel",
  2: "Breath",
  4: "Foot Pedal",
  5: "Portamento Time",
  7: "Volume",
  8: "Balance",
  10: "Pan",
  11: "Expression",
  32: "Bank Select LSB",
  64: "Sustain",
  65: "Portamento",
  66: "Sostenuto",
  67: "Soft Pedal",
  68: "Legato",
  71: "Resonance",
  72: "Release",
  73: "Attack",
  74: "Cutoff",
  120: "All Sound Off",
  121: "Reset Controllers",
  123: "All Notes Off",
};

// The name of a CC, or "" for one without a common name.
export function ccName(controller: number): string {
  return CC_NAMES[controller] ?? "";
}
