const logElement = document.querySelector("#event-log");
const raceStatus = document.querySelector("#race-status");
const delay = document.querySelector("#race-delay");
const delayValue = document.querySelector("#delay-value");
const liveEvents = [];
let lastGhostState = "missing";

function liveRecord(type, detail = {}) {
  liveEvents.push({ type, at_ms: performance.now(), ...detail });
}

function ghostSnapshot() {
  const host = document.querySelector("[data-badi-owned]");
  const field = document.querySelector("#draft");
  const hostRect = host instanceof HTMLElement ? host.getBoundingClientRect() : null;
  const fieldRect = field instanceof HTMLElement ? field.getBoundingClientRect() : null;
  return {
    exists: host instanceof HTMLElement,
    visible: host instanceof HTMLElement && !host.hidden,
    host: hostRect === null
      ? null
      : { left: hostRect.left, top: hostRect.top, width: hostRect.width, height: hostRect.height },
    field: fieldRect === null
      ? null
      : { left: fieldRect.left, top: fieldRect.top, width: fieldRect.width, height: fieldRect.height },
  };
}

function captureGhostTransition() {
  const snapshot = ghostSnapshot();
  const state = snapshot.visible ? "visible" : snapshot.exists ? "hidden" : "missing";
  if (state === lastGhostState) return;
  lastGhostState = state;
  liveRecord(`ghost.${state}`, { snapshot });
}

const ghostObserver = new MutationObserver(captureGhostTransition);
ghostObserver.observe(document.documentElement, {
  subtree: true,
  childList: true,
  attributes: true,
  attributeFilter: ["hidden", "style"],
});

function draft() {
  const field = document.querySelector("#draft");
  if (!(field instanceof HTMLTextAreaElement)) throw new Error("Draft field missing");
  return field;
}

function log(message) {
  const stamp = performance.now().toFixed(1).padStart(8, " ");
  logElement.textContent = `${stamp}  ${message}\n${logElement.textContent}`.slice(0, 6_000);
}

function input(field, value) {
  field.value = value;
  field.setSelectionRange(value.length, value.length);
  field.dispatchEvent(
    new InputEvent("input", {
      bubbles: true,
      inputType: "insertText",
      data: value.at(-1) ?? null,
    }),
  );
}

delay.addEventListener("input", () => {
  delayValue.value = delay.value;
});

document.addEventListener("focusin", (event) => {
  if (event.target instanceof HTMLElement) log(`focus → #${event.target.id || event.target.tagName}`);
});

document.addEventListener("keydown", (event) => {
  liveRecord("key.down", {
    key: event.key,
    alt: event.altKey,
    ctrl: event.ctrlKey,
    meta: event.metaKey,
    shift: event.shiftKey,
    is_trusted: event.isTrusted,
  });
}, true);

document.addEventListener("input", (event) => {
  if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) {
    log(`input #${event.target.id} length=${event.target.value.length}`);
    liveRecord("field.input", {
      id: event.target.id,
      input_type: event instanceof InputEvent ? event.inputType : "",
      is_trusted: event.isTrusted,
      value_length: event.target.value.length,
      selection_start: event.target.selectionStart,
      selection_end: event.target.selectionEnd,
    });
  }
});

document.addEventListener("click", async (event) => {
  const button = event.target.closest("button[data-action]");
  if (!(button instanceof HTMLButtonElement)) return;
  switch (button.dataset.action) {
    case "focus-draft":
      draft().focus();
      break;
    case "replace-draft": {
      const previous = draft();
      const replacement = previous.cloneNode();
      replacement.value = previous.value;
      previous.replaceWith(replacement);
      replacement.focus();
      log("draft node replaced with a new field identity");
      break;
    }
    case "type-through": {
      const field = draft();
      field.focus();
      input(field, `${field.value}x`);
      break;
    }
    case "focus-password":
      document.querySelector("#password").focus();
      break;
    case "focus-otp":
      document.querySelector("#otp").focus();
      break;
    case "focus-away":
      button.focus();
      break;
    case "clear-log":
      logElement.textContent = "";
      break;
    case "run-race": {
      button.disabled = true;
      const field = draft();
      field.focus();
      raceStatus.value = "running";
      const gap = Number(delay.value);
      for (let revision = 0; revision <= 100; revision += 1) {
        input(field, `race revision ${revision}`);
        if (gap > 0) await new Promise((resolve) => setTimeout(resolve, gap));
      }
      raceStatus.value = "101 sent";
      button.disabled = false;
      log("race complete: 100 superseded + 1 latest revision");
      break;
    }
  }
});

window.__badiLive = Object.freeze({
  resetEvents() {
    liveEvents.length = 0;
    liveRecord("fixture.reset");
  },
  mark(label) {
    liveRecord("fixture.mark", { label });
  },
  events() {
    return structuredClone(liveEvents);
  },
  ghostSnapshot,
  setDraft(value, start = value.length, end = start, dispatch = true) {
    const field = draft();
    field.value = value;
    field.setSelectionRange(start, end);
    if (dispatch) {
      field.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          inputType: "insertText",
          data: null,
        }),
      );
    }
    return { value: field.value, start: field.selectionStart, end: field.selectionEnd };
  },
  replaceDraft() {
    const previous = draft();
    const replacement = previous.cloneNode();
    replacement.value = previous.value;
    previous.replaceWith(replacement);
    replacement.focus();
    return replacement.value;
  },
  dispatchComposition(type, data = "") {
    const field = draft();
    field.dispatchEvent(new CompositionEvent(type, { bubbles: true, data }));
  },
});

log("fixture ready");
