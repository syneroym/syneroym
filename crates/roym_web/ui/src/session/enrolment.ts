import { call } from "../rpc";

/// Every Roym service that signs a record, and therefore needs the
/// person's record-signing certificate installed. Kept beside the Hub's
/// own gate so a service added later fails the gate rather than failing
/// a verb.
export const SIGNING_SERVICES = ["profile", "catalog", "conversation", "transaction"] as const;

/// The services still missing a certificate. Empty means the Hub is ready.
export async function pendingEnrolment(): Promise<string[]> {
  const missing: string[] = [];
  for (const name of SIGNING_SERVICES) {
    try {
      const res = await call<{ certificate?: { state?: string } }>(`${name}.signing-status`);
      if (res?.certificate?.state !== "installed") {
        missing.push(name);
      }
    } catch {
      missing.push(name);
    }
  }
  return missing;
}
