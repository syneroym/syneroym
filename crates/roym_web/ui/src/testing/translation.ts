import { vi } from "vitest";
import { RpcError } from "../rpc";
import { type Handler } from "./stub-rpc";

/// A fake engine's name. It carries the marker the Hindi sweep removes.
export const ENGINE = "ZqEngine";

/// The languages a reader reads, after the UI language: `navigator.languages`
/// is replaced for this test. Undo with `vi.restoreAllMocks()`.
export function readerReads(tags: string[]): void {
  vi.spyOn(navigator, "languages", "get").mockReturnValue(tags);
}

/// `translation.translate` for a fake engine: each text comes back as
/// `T:<text>`, and the call is recorded in `seen`.
export function fakeEngine(seen: Record<string, unknown>[], remote = false): Handler {
  return (params) => {
    seen.push(params);
    return {
      items: (params.texts as string[]).map((text) => ({
        text: `T:${text}`,
        source_lang: (params.source_lang as string | undefined) ?? "hi",
        engine: { name: ENGINE, remote },
      })),
    };
  };
}

/// `translation.translate` for an engine that is not there.
export const unavailableEngine: Handler = () =>
  new RpcError(-32020, "Zq-translation-unavailable", "Other", { kind: "unavailable" });
