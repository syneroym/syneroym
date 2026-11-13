import { t } from "../i18n/i18n";
import { languageName, readerLanguages, sameLanguage } from "../i18n/text_lang";
import { call } from "../rpc";

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
  return {
    texts: reply.items.map((item) => item.text),
    sourceLang: reply.items[0]?.source_lang ?? "",
    engine: reply.items[0]?.engine ?? { name: "", remote: false },
  };
}

/// The label under a machine translation: its language and its engine, and
/// when the text left this device.
export function machineNote(targetLang: string, engine: Engine): string {
  const args = { language: languageName(targetLang), engine: engine.name };
  return engine.remote
    ? t("translation-machine-note-remote", args)
    : t("translation-machine-note", args);
}

/// For unit tests: the one piece of state this module keeps.
export function setTargetsForTest(list: string[]): void {
  targets = [...list];
}
