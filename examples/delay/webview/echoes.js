// The echo strip: what one short sound at the input becomes. Time runs to
// the right. The echoes of the left channel rise above the axis, those of
// the right channel hang below it. On a mono output there is one lane.
//
// The echoes follow the processor in `src/lib.rs`: echo n comes n delay
// times after the input, at `mix * feedback^(n - 1)`. In ping-pong mode the
// odd echoes are on the left and the even ones on the right.

// The strip shows as long as the longest delay.
export const WINDOW_MS = 2500;

// Heights are in dB: an echo this far below full level has no height.
const RANGE_DB = 48;

// Room for the time labels below the lanes and at the edges.
const LABEL_BAND = 16;
const PAD = 12;

const SVG = "http://www.w3.org/2000/svg";

// The part of a lane that an echo at `level` (linear) fills.
function share(level) {
    if (level <= 0) return 0;
    return Math.max(0, (20 * Math.log10(level) + RANGE_DB) / RANGE_DB);
}

// The echoes as {ms, level, side} with side "left", "right" or "both".
function echoes({ delayMs, feedback, mix, pingPong }) {
    const list = [];
    let level = mix;
    for (let n = 1; n * delayMs <= WINDOW_MS && share(level) > 0; n++) {
        const side = pingPong ? (n % 2 === 1 ? "left" : "right") : "both";
        list.push({ ms: n * delayMs, level, side });
        level *= feedback;
    }
    return list;
}

function line(x1, y1, x2, y2, className, width) {
    const element = document.createElementNS(SVG, "line");
    element.setAttribute("x1", x1);
    element.setAttribute("x2", x2);
    element.setAttribute("y1", y1);
    element.setAttribute("y2", y2);
    element.setAttribute("class", className);
    element.setAttribute("stroke-width", width);
    return element;
}

function label(x, y, text, className, anchor = "middle") {
    const element = document.createElementNS(SVG, "text");
    element.setAttribute("x", x);
    element.setAttribute("y", y);
    element.setAttribute("class", className);
    element.setAttribute("text-anchor", anchor);
    element.textContent = text;
    return element;
}

// Draw the echoes of `settings` ({delayMs, feedback, mix, pingPong,
// outputChannels}) into `svg`, at the size it has on the page.
export function drawEchoes(svg, settings) {
    const width = svg.clientWidth;
    const height = svg.clientHeight;
    if (width === 0 || height === 0) return;
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);

    const stereo = settings.outputChannels > 1;
    const top = PAD;
    const bottom = height - LABEL_BAND;
    const axis = stereo ? (top + bottom) / 2 : bottom;
    const lane = axis - top;
    const toX = (ms) => PAD + (ms / WINDOW_MS) * (width - 2 * PAD);

    const parts = [];

    // A tick every half second. The end of the strip has a label only, set
    // inside the edge.
    for (let ms = 500; ms <= WINDOW_MS; ms += 500) {
        const x = toX(ms);
        const end = ms === WINDOW_MS;
        if (!end) parts.push(line(x, top, x, bottom, "tick", 1));
        parts.push(label(x, height - 4, `${ms / 1000} s`, "tick-label", end ? "end" : "middle"));
    }
    parts.push(line(PAD, axis, width - PAD, axis, "axis", 1));

    if (stereo) {
        parts.push(label(width - PAD, top + 10, "Left", "lane-label left", "end"));
        parts.push(label(width - PAD, bottom - 4, "Right", "lane-label right", "end"));
    }

    // Echoes closer together than a pixel share one line, with the level
    // of the loudest. That bounds the work for a delay of a few ms.
    const columns = new Map();
    const add = (x, side, level) => {
        const key = `${Math.round(x)}:${side}`;
        const known = columns.get(key);
        if (!known || known.level < level) columns.set(key, { x: Math.round(x), side, level });
    };
    for (const echo of echoes(settings)) {
        const x = toX(echo.ms);
        if (!stereo) {
            add(x, "left", echo.level);
            continue;
        }
        if (echo.side !== "right") add(x, "left", echo.level);
        if (echo.side !== "left") add(x, "right", echo.level);
    }

    // Thinner lines where echoes crowd.
    const spacing = (settings.delayMs / WINDOW_MS) * (width - 2 * PAD);
    const stroke = spacing >= 5 ? 2 : 1;

    // The input itself, at the dry level.
    const dry = share(1 - settings.mix) * lane;
    if (dry > 0) {
        parts.push(line(PAD, axis, PAD, axis - dry, "echo-input", 2));
        if (stereo) parts.push(line(PAD, axis, PAD, axis + dry, "echo-input", 2));
    }

    for (const { x, side, level } of columns.values()) {
        const reach = share(level) * lane;
        if (side === "left") {
            parts.push(line(x, axis, x, axis - reach, "echo-left", stroke));
        } else {
            parts.push(line(x, axis, x, axis + reach, "echo-right", stroke));
        }
    }

    svg.replaceChildren(...parts);
}
