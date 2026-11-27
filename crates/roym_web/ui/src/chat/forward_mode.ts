import { text } from "../dom";
import { t } from "../i18n/i18n";
import { call, refusalCode, RpcError } from "../rpc";

/** The sender's choice for one message. */
export type ForwardMode = "signed" | "not-forwardable";

export interface ForwardSettings {
  default_mode: ForwardMode;
  asked: boolean;
}

/** The shipped default, used only while the node is unreachable. */
const DEFAULT: ForwardSettings = { default_mode: "not-forwardable", asked: false };

// One read per page. Every thread draw and the first-message question ask
// for the same row, and a redraw must not add a round trip that lets a feed
// event overtake the thread it is drawing.
let cached: ForwardSettings | null = null;
let pending: Promise<ForwardSettings> | null = null;

/** Forget the cached row. A test calls this between its cases. */
export function resetForwardSettingsCache() {
  cached = null;
  pending = null;
}

export async function loadForwardSettings(): Promise<ForwardSettings> {
  if (cached) return cached;
  if (pending) return pending;
  pending = call<ForwardSettings>("conversation.settings", {})
    .then((row) => {
      cached = row;
      pending = null;
      return row;
    })
    .catch(() => {
      pending = null;
      return { ...DEFAULT };
    });
  return pending;
}

export async function saveForwardSettings(patch: {
  default_mode?: ForwardMode;
  asked?: boolean;
}): Promise<ForwardSettings> {
  const row = await call<ForwardSettings>("conversation.settings", patch);
  cached = row;
  pending = null;
  return row;
}

export interface ModeControl {
  root: HTMLElement;
  value(): ForwardMode;
  /** Back to the person's saved default. */
  reset(): void;
  /** A new default after the first-message question. */
  setDefault(mode: ForwardMode): void;
}

/** The composer control, with the group notice when `signed` is in use. */
export function modeControl(defaultMode: ForwardMode, inGroup: boolean): ModeControl {
  const root = document.createElement("div");
  root.className = "forward-mode";
  root.appendChild(text("span", t("forward-mode-label"), "forward-mode-label"));

  const select = document.createElement("select");
  select.className = "forward-mode-select";
  for (const [value, label] of [
    ["signed", t("forward-mode-signed")],
    ["not-forwardable", t("forward-mode-not-forwardable")],
  ] as Array<[ForwardMode, string]>) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    select.appendChild(option);
  }

  const notice = text("p", t("forward-mode-group-notice"), "forward-mode-group-notice");
  const update = () => {
    notice.style.display = inGroup && select.value === "signed" ? "" : "none";
  };

  let savedDefault = defaultMode;
  const reset = () => {
    select.value = savedDefault;
    update();
  };
  select.onchange = update;
  reset();

  root.append(select, notice);
  return {
    root,
    value: () => (select.value === "signed" ? "signed" : "not-forwardable"),
    reset,
    setDefault: (mode) => {
      savedDefault = mode;
      reset();
    },
  };
}

const SIGNING_REFUSALS = new Set([
  "signing-not-enrolled",
  "signing-certificate-expired",
  "signing-certificate-stale",
  "signing-no-owner",
]);

/**
 * The two ways out of a refused signed send: send this one as not
 * forwardable, or renew the signing certificate. The Hub never changes the
 * mode on its own.
 */
export function signingRefusalActions(
  err: unknown,
  sendAsNotForwardable: () => void,
  renewSigning: () => void,
): HTMLElement | null {
  if (!(err instanceof RpcError)) return null;
  const code = refusalCode(err);
  if (!code || !SIGNING_REFUSALS.has(code)) return null;

  const box = document.createElement("div");
  box.className = "forward-signing-refusal";
  const send = text(
    "button",
    t("forward-send-not-forwardable"),
    "button forward-send-not-forwardable",
  ) as HTMLButtonElement;
  send.onclick = sendAsNotForwardable;
  const renew = text("button", t("forward-renew-signing"), "button forward-renew-signing") as HTMLButtonElement;
  renew.onclick = renewSigning;
  box.append(send, renew);
  return box;
}
