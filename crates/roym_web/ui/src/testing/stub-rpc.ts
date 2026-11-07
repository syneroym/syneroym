import { vi } from "vitest";
import { RpcError } from "../rpc";

export type Handler = (params: Record<string, unknown>) => unknown;

export interface RecordedCall {
  method: string;
  params: Record<string, unknown>;
}

/// A fake `/rpc` that answers each method with the handler registered for
/// it, and returns every call it got. A handler that returns an `RpcError`
/// answers with that error; one that returns another `Error` answers with
/// an invalid-input error carrying its message. A method with no handler
/// throws, so a screen that calls something unexpected fails the test.
export function stubRpc(handlers: Record<string, Handler>): RecordedCall[] {
  const calls: RecordedCall[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_url: string, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as RecordedCall;
      calls.push(body);
      const handler = handlers[body.method];
      if (!handler) throw new Error(`unexpected method ${body.method}`);
      const out = handler(body.params);
      if (out instanceof RpcError) {
        const { code, message, data } = out;
        return { ok: true, json: async () => ({ error: { code, message, data } }) };
      }
      if (out instanceof Error) {
        return { ok: true, json: async () => ({ error: { code: -32602, message: out.message } }) };
      }
      return { ok: true, json: async () => ({ result: out }) };
    }),
  );
  return calls;
}
