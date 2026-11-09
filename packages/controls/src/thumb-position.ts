// Where a slider's thumb goes, shared by the drawing and the drag, so that
// the pointer stays on the thumb.

/** The thumb's length as a slider uses it: clamped to 0 to 1, 0 for NaN. */
export function thumbPart(thumbLength: number): number {
  return Number.isNaN(thumbLength) ? 0 : Math.min(1, Math.max(0, thumbLength));
}

/**
 * Where the center of a slider's thumb goes for the normalized `value`, as
 * a part of the slider's length from its low end (the bottom of an upright
 * slider, the left of a lying one): half of `thumbLength` in at 0, half of
 * it in from the other end at 1. Draw the thumb there with the
 * `thumbLength` given to `attachSlider` and the pointer stays on it during
 * a drag. For an upright slider of height `h` in SVG, the thumb's center is
 * at `y = h * (1 - thumbPosition(value, thumbLength))`.
 */
export function thumbPosition(value: number, thumbLength = 0): number {
  const thumb = thumbPart(thumbLength);
  return thumb / 2 + Math.min(1, Math.max(0, value)) * (1 - thumb);
}

/**
 * The value at `part` of the slider's length from its low end, the inverse
 * of `thumbPosition` without the clamp: below 0 or above 1 past the ends.
 * Undefined when the thumb leaves nothing to travel.
 */
export function valueAtThumbPosition(part: number, thumbLength: number): number | undefined {
  const thumb = thumbPart(thumbLength);
  if (thumb >= 1) return undefined;
  return (part - thumb / 2) / (1 - thumb);
}
