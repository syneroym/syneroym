import { authHeaders } from "./session/login";

export type RpcErrorType = "NotSignedIn" | "NotOwner" | "NoOwner" | "NotLocal" | "Other";

export class RpcError extends Error {
  constructor(
    public code: number,
    message: string,
    public type: RpcErrorType,
    /// The refusal's own structured detail, when the service gave one.
    public data?: unknown,
  ) {
    super(message);
    this.name = "RpcError";
  }
}

/// A reply the node sent, as far as `call` reads it.
interface Reply {
  result?: unknown;
  error?: { code: number; message?: string; data?: unknown };
}

/// The node could not be reached, or its reply was not JSON. The browser
/// words the cause in its own English, so it is only a detail; the Hub's
/// own message is chosen by code `0`.
function unreachable(cause: unknown): RpcError {
  return new RpcError(0, String(cause), "Other");
}

function errorType(code: number): RpcErrorType {
  if (code === -32010) return "NotSignedIn";
  if (code === -32011) return "NotOwner";
  if (code === -32012) return "NoOwner";
  if (code === -32013) return "NotLocal";
  return "Other";
}

export async function call<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
  let res: Response;
  try {
    res = await fetch("/rpc", {
      method: "POST",
      headers: { "Content-Type": "application/json", ...authHeaders() },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method,
        params,
      }),
    });
  } catch (cause) {
    throw unreachable(cause);
  }

  // The code is the HTTP status; the Hub words it (see `errText`).
  if (!res.ok) {
    throw new RpcError(res.status, "", "Other");
  }

  let json: Reply;
  try {
    json = (await res.json()) as Reply;
  } catch (cause) {
    throw unreachable(cause);
  }
  if (json.error) {
    const { code } = json.error;
    throw new RpcError(code, json.error.message || "RPC error", errorType(code), json.error.data);
  }

  return json.result as T;
}
