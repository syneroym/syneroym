import { authHeaders } from "./session/login";

export type RpcErrorType = "NotSignedIn" | "NotOwner" | "NoOwner" | "NotLocal" | "Other";

/// The message the Hub shows for a `-32013` refusal: the request reached a
/// service that only answers calls made from inside this installation.
export const NOT_LOCAL_MESSAGE =
  "this installation refused a request that did not come from you";

export class RpcError extends Error {
  constructor(public code: number, message: string, public type: RpcErrorType) {
    super(message);
    this.name = "RpcError";
  }
}

export async function call<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
  const res = await fetch("/rpc", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaders() },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method,
      params,
    }),
  });

  if (!res.ok) {
    throw new RpcError(res.status, `HTTP error ${res.status}`, "Other");
  }

  const json = await res.json();
  if (json.error) {
    const code = json.error.code;
    const msg = json.error.message || "RPC error";
    let type: RpcErrorType = "Other";
    if (code === -32010) type = "NotSignedIn";
    else if (code === -32011) type = "NotOwner";
    else if (code === -32012) type = "NoOwner";
    else if (code === -32013) type = "NotLocal";
    throw new RpcError(code, type === "NotLocal" ? NOT_LOCAL_MESSAGE : msg, type);
  }

  return json.result as T;
}
