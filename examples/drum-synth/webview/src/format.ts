// A number's minus as the typographic minus sign (U+2212), which Inter
// draws to match its digits; a hyphen sits loose before tabular figures.
// For display only: typed values may keep the hyphen.
export function withMinus(text: string): string {
  return text.replace(/-(?=\d)/g, "−");
}
