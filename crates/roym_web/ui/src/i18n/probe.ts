import EN from "./en.ftl?raw";
const L = import.meta.glob<string>(["./*.ftl", "!./en.ftl"], { query: "?raw", import: "default" });
export async function probe(): Promise<string> {
  const k = Object.keys(L)[0];
  return EN + (k ? await L[k]() : "");
}
