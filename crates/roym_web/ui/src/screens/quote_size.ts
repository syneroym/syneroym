import { call } from "../rpc";
import { t } from "../i18n/i18n";

export interface MeasureResult {
  bytes: number;
  limit: number;
  details: { commitment_id: string; bytes: number; limit: number }[];
  fits: boolean;
  card_bytes: number;
  mailbox_fits: boolean;
}

export function attachSizeLine(
  form: HTMLElement,
  readTerms: () => Record<string, unknown> | undefined,
): () => void {
  const line = document.createElement("p");
  line.className = "quote-size-line";
  line.style.display = "none";

  const errP = form.querySelector(".quote-form-error");
  if (errP) {
    form.insertBefore(line, errP);
  } else {
    form.appendChild(line);
  }

  let timer: ReturnType<typeof setTimeout> | undefined;
  let active = true;

  const update = async () => {
    const terms = readTerms();
    if (!terms) {
      line.style.display = "none";
      line.textContent = "";
      line.className = "quote-size-line";
      return;
    }

    try {
      const res = await call<MeasureResult>("quote.measure", { terms });
      if (!active) return;
      line.style.display = "";
      const used = (res.bytes / 1024).toFixed(1);
      const limit = (res.limit / 1024).toFixed(0);

      let text = t("quote-size-line", { used, limit });
      if (!res.fits) {
        line.className = "quote-size-line quote-size-over";
        text += " " + t("quote-size-over");
      } else if (!res.mailbox_fits) {
        line.className = "quote-size-line";
        text += "\n" + t("quote-size-mailbox");
      } else {
        line.className = "quote-size-line";
      }
      line.textContent = text;
    } catch {
      if (!active) return;
      line.style.display = "none";
      line.textContent = "";
      line.className = "quote-size-line";
    }
  };

  const onInput = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(update, 300);
  };

  form.addEventListener("input", onInput);
  form.addEventListener("change", onInput);

  onInput();

  return () => {
    active = false;
    if (timer !== undefined) clearTimeout(timer);
    form.removeEventListener("input", onInput);
    form.removeEventListener("change", onInput);
    line.remove();
  };
}
