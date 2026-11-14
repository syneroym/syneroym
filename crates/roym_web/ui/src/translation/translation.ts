import { errText } from "../dom";
import { t } from "../i18n/i18n";
import { languageName, readerLanguages, sameLanguage } from "../i18n/text_lang";
import { call, RpcError } from "../rpc";

/// The error code of every refusal by the node's translation host.
const TRANSLATION_ERROR = -32020;

/// A page keeps at most this many machine translations. The oldest goes
/// first.
const KEPT_LIMIT = 200;

/// How long the Hub waits for the list of target languages before it
/// goes on with none.
const TARGETS_TIMEOUT_MS = 3000;

/// The engine that made a machine translation, as the node names it.
export interface Engine {
  name: string;
  remote: boolean;
}

export interface MachineText {
  texts: string[];
  sourceLang: string;
  engine: Engine;
}

interface TargetsReply {
  targets: string[];
}

interface TranslateReply {
  items: { text: string; source_lang: string; engine: Engine }[];
}

/// The languages the node can translate into. Empty until `loadTargets`
/// answers, and for a node with no translation engine.
let targets: string[] = [];

/// The machine translations of this page, by the text that was sent. A
/// thread is drawn again on every new event, and a translation drawn again
/// from here costs no second call to the engine. Nothing is stored on the
/// device: a reload starts empty.
const kept = new Map<string, MachineText>();

/// The cards on which the person chose to read a kept translation, each
/// as the card's key joined to the key of the translation. The choice is
/// about one card: another card with the same text does not inherit it.
const shownOn = new Set<string>();

function keptKey(texts: readonly string[], sourceLang: string | null, targetLang: string): string {
  return JSON.stringify([sourceLang, targetLang, texts]);
}

/// The translation of exactly these texts into `targetLang`, if this page
/// made one.
export function keptTranslation(
  texts: readonly string[],
  sourceLang: string | null,
  targetLang: string,
): MachineText | undefined {
  return kept.get(keptKey(texts, sourceLang, targetLang));
}

function shownKey(
  cardKey: string,
  texts: readonly string[],
  sourceLang: string | null,
  targetLang: string,
): string {
  return JSON.stringify([cardKey, keptKey(texts, sourceLang, targetLang)]);
}

/// Records whether the person has the translation of these texts on screen
/// on the card `cardKey`, so a draw of the same card shows it again.
export function keepShown(
  cardKey: string,
  texts: readonly string[],
  sourceLang: string | null,
  targetLang: string,
  shown: boolean,
): void {
  const key = shownKey(cardKey, texts, sourceLang, targetLang);
  shownOn.delete(key);
  if (!shown) return;
  shownOn.add(key);
  if (shownOn.size > KEPT_LIMIT) {
    const oldest = shownOn.values().next().value;
    if (oldest !== undefined) shownOn.delete(oldest);
  }
}

/// Whether the person had this translation on screen on the card
/// `cardKey` when it was last drawn.
export function wasShown(
  cardKey: string,
  texts: readonly string[],
  sourceLang: string | null,
  targetLang: string,
): boolean {
  return shownOn.has(shownKey(cardKey, texts, sourceLang, targetLang));
}

function keep(key: string, out: MachineText): void {
  kept.delete(key);
  kept.set(key, out);
  if (kept.size > KEPT_LIMIT) {
    const oldest = kept.keys().next().value;
    if (oldest !== undefined) kept.delete(oldest);
  }
}

/// Asks the node once which languages it can translate into. A node that
/// cannot answer, is older, or has no engine leaves the list empty; the
/// person is told nothing, because no translate action is drawn then.
export async function loadTargets(): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<TargetsReply>((resolve) => {
    timer = setTimeout(() => resolve({ targets: [] }), TARGETS_TIMEOUT_MS);
  });
  try {
    const reply = await Promise.race([call<TargetsReply>("translation.targets"), timeout]);
    targets = Array.isArray(reply.targets) ? reply.targets.filter((x) => typeof x === "string") : [];
  } catch {
    targets = [];
  } finally {
    clearTimeout(timer);
  }
}

/// Called once, when the session is known. A page with no signed-in
/// person makes no call: the methods need the owner.
export async function startTranslation(signedIn: boolean): Promise<void> {
  if (signedIn) await loadTargets();
}

/// The language to translate into for a text that is already written in
/// `skipLangs`: the first language of the node's list that the reader
/// reads and that the text is not already in. `null` when there is none,
/// and then no translate action is drawn. The result is an entry of the
/// node's list, not the reader's own tag, so it is a canonical tag the node
/// accepts.
export function targetFor(skipLangs: readonly string[]): string | null {
  const reader = readerLanguages();
  const found = targets.find(
    (target) =>
      reader.some((tag) => sameLanguage(tag, target)) &&
      !skipLangs.some((tag) => sameLanguage(tag, target)),
  );
  return found ?? null;
}

/// The languages the search box can translate into.
export function searchTargets(): string[] {
  return [...targets];
}

/// Translates each text. Throws an `RpcError` when the node refuses, as
/// `call` does.
export async function translateTexts(
  texts: string[],
  sourceLang: string | null,
  targetLang: string,
): Promise<MachineText> {
  const params: Record<string, unknown> = { texts, target_lang: targetLang };
  if (sourceLang) params.source_lang = sourceLang;
  const reply = await call<TranslateReply>("translation.translate", params);
  const out = {
    texts: reply.items.map((item) => item.text),
    sourceLang: reply.items[0]?.source_lang ?? "",
    engine: reply.items[0]?.engine ?? { name: "", remote: false },
  };
  keep(keptKey(texts, sourceLang, targetLang), out);
  return out;
}

/// The node starts its `engine-failed` message with this, then adds the
/// engine's own text. The Hub says the first part in the reader's language.
const ENGINE_FAILED_PREFIX = /^translation engine failed: /;

function engineText(message: string): string {
  return message.replace(ENGINE_FAILED_PREFIX, "");
}

/// The text to show when a translation did not happen. A refusal by the
/// translation host is told by its kind, so the person learns what they can
/// change (a text that is too long, for example). Any other failure is
/// shown inside "Could not translate".
export function translationFailure(err: unknown): string {
  if (err instanceof RpcError && err.code === TRANSLATION_ERROR) {
    const data = (err.data ?? {}) as Record<string, unknown>;
    switch (data.kind) {
      case "unavailable":
        return t("translation-unavailable");
      case "unsupported-pair":
        return t("translation-unsupported-pair", {
          source: languageName(String(data.source ?? "")),
          target: languageName(String(data.target ?? "")),
        });
      case "too-long":
        return t("translation-too-long");
      case "engine-failed":
        return t("translation-engine-failed", { detail: engineText(err.message) });
      default:
        return errText(err);
    }
  }
  return t("translation-failed", { detail: errText(err) });
}

/// The label under a machine translation: its language and its engine, and
/// when the text left this device.
export function machineNote(targetLang: string, engine: Engine): string {
  const args = { language: languageName(targetLang), engine: engine.name };
  return engine.remote
    ? t("translation-machine-note-remote", args)
    : t("translation-machine-note", args);
}

/// For unit tests: sets the target list and forgets every kept translation.
export function setTargetsForTest(list: string[]): void {
  targets = [...list];
  kept.clear();
  shownOn.clear();
}
