import { t } from "../i18n/i18n";
import { pendingEnrolment } from "../session/enrolment";

export async function renderSetup(
  container: HTMLElement,
  onDone: () => void,
  initialPending?: string[],
) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "setup-screen";

  const h2 = document.createElement("h2");
  h2.textContent = t("setup-title");
  box.appendChild(h2);

  const desc = document.createElement("p");
  desc.textContent = t("setup-intro");
  box.appendChild(desc);

  const pendingList = document.createElement("div");
  pendingList.className = "pending-enrolment-list";
  const pending = initialPending ?? (await pendingEnrolment());
  if (pending.length > 0) {
    const p = document.createElement("p");
    p.className = "pending-services-text";
    p.textContent = t("setup-missing", { services: pending.join(", ") });
    pendingList.appendChild(p);
  }
  box.appendChild(pendingList);

  const checkBtn = document.createElement("button");
  checkBtn.className = "button check-again-button";
  checkBtn.textContent = t("setup-check-again");
  checkBtn.onclick = async () => {
    checkBtn.disabled = true;
    try {
      const remaining = await pendingEnrolment();
      if (remaining.length === 0) {
        onDone();
        return;
      }
      pendingList.replaceChildren();
      const p = document.createElement("p");
      p.className = "pending-services-text";
      p.textContent = t("setup-missing", { services: remaining.join(", ") });
      pendingList.appendChild(p);
    } finally {
      checkBtn.disabled = false;
    }
  };
  box.appendChild(checkBtn);

  container.appendChild(box);
}
