import type { Attachment } from "svelte/attachments";
import { runtime } from "@helpermedia/beamer-api";

// The hits from the stream sender "hits" (`Hit` in src/lib.rs): each drum
// that was hit, with its velocity, in arrays of what arrived since the
// last tick, also the hits of the pads. A hit is an event, not a state: the
// light flashes and fades on its own, so nothing waits for the host when
// it stops calling the plugin.

// How long a flash takes to fade, in ms.
const FADE_MS = 250;

// The brightness of the softest hit, so that it still shows.
const SOFTEST = 0.35;

// The loudest hit of `drum` in `hits`, 0 for none. The shape of what
// arrives is the sender's claim, so each field is checked.
function loudestHit(hits: unknown, drum: number): number {
  if (!Array.isArray(hits)) return 0;
  let loudest = 0;
  for (const hit of hits) {
    if (typeof hit !== "object" || hit === null) continue;
    const { drum: hitDrum, velocity } = hit as Record<string, unknown>;
    if (hitDrum === drum && typeof velocity === "number" && Number.isFinite(velocity)) {
      loudest = Math.max(loudest, Math.min(1, velocity));
    }
  }
  return loudest;
}

/**
 * Flash the element on each hit of `drum`, as bright as the hit is loud,
 * then fade it. Several hits in one tick flash once, at the loudest. With
 * reduced motion the light shows for the same time without the fade.
 */
export function flash(drum: number): Attachment<HTMLElement> {
  return (element) =>
    runtime().on("hits", (hits) => {
      const velocity = loudestHit(hits, drum);
      if (velocity <= 0) return;
      const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
      const brightness = SOFTEST + (1 - SOFTEST) * velocity;
      element.animate([{ opacity: brightness }, { opacity: 0 }], {
        duration: FADE_MS,
        easing: reduced ? "steps(1, end)" : "ease-out",
      });
    });
}
