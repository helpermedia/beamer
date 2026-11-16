<!--
  The pad of a drum, with its name and note: it flashes on each hit, from
  the host or from a press. It plays the drum on a press (from the press,
  not the release, so it sounds at once), on Space or Enter while it has
  focus and on a click without a pointer (assistive technology). The note
  goes to the processor as notes from the editor do, at a fixed velocity.
-->
<script lang="ts">
  import { noteName, playNote } from "@helpermedia/beamer-api";
  import { DRUM_CHANNEL } from "../drums";
  import { flash } from "../flash";

  let { drum, note, name }: { drum: number; note: number; name: string } = $props();

  // The velocity of a press on the pad.
  const VELOCITY = 0.8;

  const play = () => playNote(note, { velocity: VELOCITY, channel: DRUM_CHANNEL });

  // Whether a key or a click comes with Command, Control or Alt: a
  // shortcut, which belongs to the page and plays nothing. A click with
  // Control is the secondary click on macOS.
  const shortcut = (event: KeyboardEvent | MouseEvent) => event.metaKey || event.ctrlKey || event.altKey;

  function onpointerdown(event: PointerEvent) {
    if (event.button === 0 && !shortcut(event)) play();
  }

  // A held key plays once. The key's own click is prevented, so it does
  // not play a second time.
  function onkeydown(event: KeyboardEvent) {
    if (shortcut(event) || (event.key !== " " && event.key !== "Enter")) return;
    event.preventDefault();
    if (!event.repeat) play();
  }

  // Whether the click to come is Space's own. The browser clicks the
  // button when Space comes up, right after this, with the keys held then:
  // a shortcut's modifier released first (or Option-Space, a no-break
  // space) would not show in that click. A plain Space played at the
  // press and prevented its click, so a click that comes with Space's
  // release never plays. The flag lasts until the click went by.
  let spaceClick = false;

  function onkeyup(event: KeyboardEvent) {
    if (event.code !== "Space") return;
    spaceClick = true;
    setTimeout(() => (spaceClick = false));
  }

  // A click without a pointer or a key behind it, as assistive technology
  // sends. A click from the pointer played at the press already. The click
  // of a shortcut on the pad (Control-Enter) carries its keys and plays
  // nothing, as does the click that comes with Space's release.
  function onclick(event: MouseEvent) {
    if (event.detail === 0 && !shortcut(event) && !spaceClick) play();
  }
</script>

<!-- It fills the cell it is given. An explicit tab stop: WebKit leaves
     buttons out of Tab unless the system's keyboard navigation is on. -->
<button
  type="button"
  tabindex="0"
  aria-label="Play {name}"
  class="relative flex size-full flex-col items-center justify-center overflow-hidden rounded-lg bg-neutral-700 ring-1 ring-neutral-600 ring-inset outline-none hover:bg-neutral-600 focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-offset-2 focus-visible:outline-brand-blue active:bg-neutral-500"
  {onpointerdown}
  {onkeydown}
  {onkeyup}
  {onclick}
>
  <span aria-hidden="true" class="absolute inset-0 opacity-0 bg-brand-blue shadow-[inset_0_0_3rem_rgb(255_255_255/0.25)]" {@attach flash(drum)}
  ></span>
  <span class="relative text-base font-semibold text-paper">{name}</span>
  <!-- Paper, a little transparent: readable on the pad and on the flash. -->
  <span class="relative mt-1 text-[0.6875rem] tabular-nums text-paper/70">{noteName(note)} · {note}</span>
</button>
