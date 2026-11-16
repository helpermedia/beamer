// Drawing on whole device pixels. The editor scales as a whole, so its
// sizes are fractions of a pixel: a thumb of 12 design pixels can be 19.2
// device pixels wide, its edges between pixels and smoothed more on one
// side than on the other, so a line in it looks off its middle. A slider
// that measures its track in device pixels can draw every edge on a whole
// pixel instead, its line and its thumb of the same parity, so the line is
// in the middle exactly.

/**
 * An element's size in whole device pixels, the device pixels per CSS
 * pixel and the device pixels per design pixel (a sixteenth of the rem,
 * which follows the window). The browser places an SVG on a whole device
 * pixel, as it does an image, then rounds its size. An SVG of this size
 * (CSS width `width / ratio`) keeps it, so its units are the device pixels
 * exactly.
 */
export interface DevicePixels {
  width: number;
  height: number;
  ratio: number;
  unit: number;
}

/**
 * Measure `element` in device pixels now and again whenever its size, the
 * window or the pixel density changes (the editor scales with its window).
 * An element that is not shown (a hidden tab) measures nothing, so the
 * last size stays. Returns the function that stops measuring.
 */
export function measureDevicePixels(element: Element, measured: (box: DevicePixels) => void): () => void {
  let density: MediaQueryList | null = null;
  let last: DevicePixels | null = null;
  const measure = () => {
    const ratio = devicePixelRatio;
    density?.removeEventListener("change", measure);
    // Older WebKit (macOS 11 and 12) knows only the prefixed feature.
    density = matchMedia(`(resolution: ${ratio}dppx), (-webkit-device-pixel-ratio: ${ratio})`);
    density.addEventListener("change", measure);
    const rect = element.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;
    const rem = parseFloat(getComputedStyle(document.documentElement).fontSize);
    const box = { width: Math.round(rect.width * ratio), height: Math.round(rect.height * ratio), ratio, unit: (ratio * rem) / 16 };
    if (last && last.width === box.width && last.height === box.height && last.ratio === box.ratio && last.unit === box.unit) return;
    last = box;
    measured(box);
  };
  measure();
  const observer = new ResizeObserver(measure);
  observer.observe(element);
  addEventListener("resize", measure);
  return () => {
    observer.disconnect();
    removeEventListener("resize", measure);
    density?.removeEventListener("change", measure);
  };
}

/**
 * `length` rounded to a whole number of pixels (at least 1), one more when
 * that has not the parity of `outer`, so that it sits in the middle of
 * `outer` exactly.
 */
export const sameParity = (length: number, outer: number) => {
  const rounded = Math.max(1, Math.round(length));
  return (outer - rounded) % 2 === 0 ? rounded : rounded + 1;
};

/**
 * The largest whole number of pixels up to `length` with the parity of
 * `inner` (at least 1 or 2), so that `inner` sits in its middle exactly.
 * For a thumb: drawn no longer than the length the slider takes it to be,
 * so a press on it grabs it. The outermost pixel can still count as the
 * track, as the thumb's place and the pointer are rounded.
 */
export const atMostSameParity = (length: number, inner: number) => {
  const parity = inner % 2;
  return Math.max(parity === 0 ? 2 : 1, 2 * Math.floor((length - parity) / 2) + parity);
};
