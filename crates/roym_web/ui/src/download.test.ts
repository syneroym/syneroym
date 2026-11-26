import { describe, expect, it, vi } from "vitest";
import { download } from "./download";

describe("download helper", () => {
  it("creates object url, triggers click on an anchor, and revokes object url", () => {
    const createUrlSpy = vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:mock-url");
    const revokeUrlSpy = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});

    let clicked = false;
    let downloadAttr = "";
    let hrefAttr = "";

    const origCreateElement = document.createElement.bind(document);
    vi.spyOn(document, "createElement").mockImplementation((tag: string) => {
      const el = origCreateElement(tag);
      if (tag === "a") {
        el.click = () => {
          clicked = true;
          downloadAttr = (el as HTMLAnchorElement).download;
          hrefAttr = (el as HTMLAnchorElement).href;
        };
      }
      return el;
    });

    const blob = new Blob(["test"], { type: "text/plain" });
    download("myfile.txt", blob);

    expect(createUrlSpy).toHaveBeenCalledWith(blob);
    expect(clicked).toBe(true);
    expect(downloadAttr).toBe("myfile.txt");
    expect(hrefAttr).toBe("blob:mock-url");
    expect(revokeUrlSpy).toHaveBeenCalledWith("blob:mock-url");

    vi.restoreAllMocks();
  });
});
