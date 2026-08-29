const logElement = document.querySelector("#event-log");
const raceStatus = document.querySelector("#race-status");
const delay = document.querySelector("#race-delay");
const delayValue = document.querySelector("#delay-value");

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

document.addEventListener("input", (event) => {
  if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) {
    log(`input #${event.target.id} length=${event.target.value.length}`);
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

log("fixture ready");
