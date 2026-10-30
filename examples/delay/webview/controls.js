// Controls on `__BEAMER__.params`: a knob for a continuous parameter and a
// row of buttons for the choices of an enum parameter. Each change is a
// gesture of its own (beginEdit, set, endEdit), so the host records it as
// one undo step. A drag is one gesture from press to release. So is a
// scroll, until the wheel rests.

const params = __BEAMER__.params;

// Normalized change per pixel of a drag, normal and with shift held.
const DRAG_STEP = 1 / 200;
const FINE_DRAG_STEP = 1 / 1000;

// Normalized change per arrow key, normal and with shift.
const STEP = 0.01;
const FINE_STEP = 0.002;

// A scroll ends when no wheel event came for this long.
const WHEEL_REST_MS = 300;

// The arc of a knob: 270 degrees, open at the bottom.
const ARC_START = 135;
const ARC_SWEEP = 270;
const RADIUS = 20;
const CENTER = 26;

function clamp(normalized) {
    return Math.min(1, Math.max(0, normalized));
}

// Set a parameter as one gesture.
function edit(id, normalized) {
    params.beginEdit(id);
    params.set(id, clamp(normalized));
    params.endEdit(id);
}

// The value with its unit, as the host shows it.
function valueText(id) {
    const units = params.info(id).units;
    const text = params.getDisplayText(id);
    if (!units) return text;
    return units === "%" ? text + units : `${text} ${units}`;
}

function pointOnArc(degrees) {
    const radians = (degrees * Math.PI) / 180;
    return [CENTER + RADIUS * Math.cos(radians), CENTER + RADIUS * Math.sin(radians)];
}

// An SVG path along the arc, from its start to the given part of it.
function arcPath(part) {
    const [x0, y0] = pointOnArc(ARC_START);
    const sweep = Math.max(0.001, part) * ARC_SWEEP;
    const [x1, y1] = pointOnArc(ARC_START + sweep);
    const large = sweep > 180 ? 1 : 0;
    return `M ${x0} ${y0} A ${RADIUS} ${RADIUS} 0 ${large} 1 ${x1} ${y1}`;
}

// A knob for the parameter `id` in `container`. Drag up and down (shift for
// fine steps), scroll, use the arrow keys or double click to reset. A click
// on the value lets the user type one, which the parameter itself parses.
export function knob(container, id) {
    const info = params.info(id);
    container.className = "knob";
    container.innerHTML = `
        <svg viewBox="0 0 52 52" tabindex="0" role="slider"
             aria-label="${info.name}" aria-valuemin="0" aria-valuemax="1">
            <path class="knob-track" d="${arcPath(1)}"></path>
            <path class="knob-value"></path>
            <circle class="knob-dot" r="3"></circle>
        </svg>
        <span class="knob-name">${info.name}</span>
        <button class="knob-text" type="button" title="Type a value"></button>`;

    const svg = container.querySelector("svg");
    const valueArc = container.querySelector(".knob-value");
    const dot = container.querySelector(".knob-dot");
    let text = container.querySelector(".knob-text");

    function render() {
        const value = params.get(id);
        valueArc.setAttribute("d", arcPath(value));
        const [x, y] = pointOnArc(ARC_START + value * ARC_SWEEP);
        dot.setAttribute("cx", CENTER + (x - CENTER) * 0.55);
        dot.setAttribute("cy", CENTER + (y - CENTER) * 0.55);
        svg.setAttribute("aria-valuenow", value.toFixed(3));
        svg.setAttribute("aria-valuetext", valueText(id));
        text.textContent = valueText(id);
    }

    // A scroll is one gesture until the wheel rests. Any other edit ends it
    // first, so that gestures never overlap.
    let wheelRest = null;
    function settleWheel() {
        if (wheelRest === null) return;
        clearTimeout(wheelRest);
        wheelRest = null;
        params.endEdit(id);
    }

    // Dragging: one gesture from press to release. Each move adds to the
    // value, so pressing or releasing shift halfway does not make it jump.
    let lastY = null;
    svg.addEventListener("pointerdown", (event) => {
        // The primary button only: a right click opens the context menu,
        // which takes the release.
        if (event.button !== 0 || lastY !== null) return;
        settleWheel();
        svg.setPointerCapture(event.pointerId);
        lastY = event.clientY;
        params.beginEdit(id);
    });
    svg.addEventListener("pointermove", (event) => {
        if (lastY === null) return;
        const step = event.shiftKey ? FINE_DRAG_STEP : DRAG_STEP;
        params.set(id, clamp(params.get(id) + (lastY - event.clientY) * step));
        lastY = event.clientY;
    });
    const release = () => {
        if (lastY === null) return;
        lastY = null;
        params.endEdit(id);
    };
    svg.addEventListener("pointerup", release);
    svg.addEventListener("pointercancel", release);
    // The capture also ends when the window loses the pointer halfway.
    svg.addEventListener("lostpointercapture", release);

    svg.addEventListener("dblclick", () => {
        settleWheel();
        edit(id, info.defaultValue);
    });

    // Scrolling moves the value by the distance, as far as a drag would. A
    // trackpad sends many small events, so one gesture lasts until the
    // wheel rests. With shift held, macOS reports a mouse wheel as
    // horizontal.
    svg.addEventListener("wheel", (event) => {
        event.preventDefault();
        // A drag has the value already.
        if (lastY !== null) return;
        const delta = event.deltaY !== 0 ? event.deltaY : event.shiftKey ? event.deltaX : 0;
        if (delta === 0) return;
        if (wheelRest === null) {
            params.beginEdit(id);
        } else {
            clearTimeout(wheelRest);
        }
        const step = event.shiftKey ? FINE_DRAG_STEP : DRAG_STEP;
        params.set(id, clamp(params.get(id) - delta * step));
        wheelRest = setTimeout(() => {
            wheelRest = null;
            params.endEdit(id);
        }, WHEEL_REST_MS);
    }, { passive: false });

    svg.addEventListener("keydown", (event) => {
        const step = event.shiftKey ? FINE_STEP : STEP;
        const value = params.get(id);
        const next = {
            ArrowUp: value + step,
            ArrowRight: value + step,
            ArrowDown: value - step,
            ArrowLeft: value - step,
            PageUp: value + 0.1,
            PageDown: value - 0.1,
            Home: 0,
            End: 1,
        }[event.key];
        if (next === undefined) return;
        event.preventDefault();
        settleWheel();
        edit(id, next);
    });

    // Typing a value: the text field takes the place of the value until
    // Enter sets it or Escape or a click elsewhere leaves it.
    function startTyping() {
        settleWheel();
        const input = document.createElement("input");
        input.className = "knob-text";
        input.value = params.getDisplayText(id);
        input.setAttribute("aria-label", `${info.name} value`);
        const button = text;
        let done = false;
        const finish = async (commit, byKey) => {
            if (done) return;
            done = true;
            if (commit) {
                // The parameter's own parser: null when the text is no value.
                const normalized = await __BEAMER__.invoke(
                    "_beamer/paramTextToNormalized", info.id, input.value,
                );
                if (normalized !== null) edit(id, normalized);
            }
            input.replaceWith(button);
            text = button;
            render();
            // A keyboard user goes on from where they typed. A click
            // elsewhere keeps the focus it moved.
            if (byKey) button.focus();
        };
        input.addEventListener("keydown", (event) => {
            if (event.key === "Enter") finish(true, true);
            if (event.key === "Escape") finish(false, true);
        });
        input.addEventListener("blur", () => finish(false, false));
        button.replaceWith(input);
        input.select();
    }
    text.addEventListener("click", startTyping);

    params.on(id, render);
    render();

    return {
        // Dim the knob while the parameter has no effect. It stays usable.
        setUnused(unused, reason) {
            container.classList.toggle("unused", unused);
            container.title = unused ? reason : "";
        },
    };
}

// A button for each choice of the enum parameter `id` in `container`. The
// names come from the parameter (`valueTexts`), so they match the host.
export function choices(container, id) {
    const info = params.info(id);
    const buttons = info.valueTexts.map((name, index) => {
        const button = document.createElement("button");
        button.type = "button";
        button.setAttribute("role", "radio");
        button.textContent = name;
        button.addEventListener("click", () => edit(id, index / info.steps));
        container.append(button);
        return button;
    });

    function render() {
        const selected = Math.round(params.get(id) * info.steps);
        buttons.forEach((button, index) => {
            button.setAttribute("aria-checked", String(index === selected));
        });
    }

    params.on(id, render);
    render();
}
