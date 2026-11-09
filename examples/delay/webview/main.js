// The delay's editor, on `__BEAMER__` alone: no framework, no build step.
//
// The parameters come from `__BEAMER__.params`. The host's tempo and the
// output channels come from the processor's value sender "status" (see
// `Status` in `src/lib.rs`). From both the editor works out the delay time
// in effect, so it follows a dial at once, also while the host is stopped
// and does not run the processor.

import { choices, dial } from "./controls.js";
import { drawEchoes, WINDOW_MS } from "./echoes.js";

const params = __BEAMER__.params;

// The length of each sync mode in beats, in the order of `SyncMode` in
// `src/lib.rs`. Free has none: the time parameter sets the delay.
const SYNC_BEATS = [null, 1, 0.5, 0.25, 0.125];

// The index of ping-pong in `StereoMode` in `src/lib.rs`.
const PING_PONG = 1;

// The tempo the processor assumes while the host reports none.
const FALLBACK_TEMPO = 120;

// The index of the choice an enum parameter is at.
function choice(id) {
    return Math.round(params.get(id) * params.info(id).steps);
}

function formatNumber(value) {
    return String(Number(value.toFixed(value < 100 ? 1 : 0)));
}

// The delay time in effect and where it comes from.
function delay(status) {
    const index = choice("sync_mode");
    const beats = SYNC_BEATS[index];
    if (beats === null) {
        return { ms: params.getPlain("time"), source: "Free time" };
    }

    const note = params.info("sync_mode").valueTexts[index];
    const tempo = status.tempo ?? FALLBACK_TEMPO;
    const ms = (60000 / tempo) * beats;
    let source = `${note} at ${formatNumber(tempo)} BPM`;
    if (status.tempo === null) source += ", as the host reports no tempo";
    if (ms > WINDOW_MS) {
        return { ms: WINDOW_MS, source: `${source}, held at the longest delay` };
    }
    return { ms, source };
}

async function start() {
    await __BEAMER__.ready;

    const timeText = document.getElementById("delay-time");
    const sourceText = document.getElementById("delay-source");
    const strip = document.getElementById("echoes");

    choices(document.getElementById("stereo-mode"), "stereo_mode");
    choices(document.getElementById("sync-mode"), "sync_mode");
    const time = dial(document.getElementById("time"), "time");
    dial(document.getElementById("feedback"), "feedback");
    dial(document.getElementById("mix"), "mix");

    // Draw at most once per frame, however many changes arrive.
    let pending = false;
    function update() {
        if (pending) return;
        pending = true;
        requestAnimationFrame(() => {
            pending = false;
            // Undefined until the host prepared the plugin.
            const status = __BEAMER__.values.get("status") ?? { tempo: null, outputChannels: 2 };
            const { ms, source } = delay(status);
            timeText.textContent = formatNumber(ms);
            sourceText.textContent = source;

            const synced = SYNC_BEATS[choice("sync_mode")] !== null;
            time.setUnused(synced, "A note value sets the time");

            drawEchoes(strip, {
                delayMs: ms,
                feedback: params.getPlain("feedback"),
                mix: params.getPlain("mix"),
                pingPong: choice("stereo_mode") === PING_PONG,
                outputChannels: status.outputChannels,
            });
        });
    }

    for (const id of ["sync_mode", "stereo_mode", "time", "feedback", "mix"]) {
        params.on(id, update);
    }
    __BEAMER__.values.on("status", update);
    new ResizeObserver(update).observe(strip);

    document.querySelector(".editor").hidden = false;
    update();
}

start();
