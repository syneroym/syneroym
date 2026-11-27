import { t } from "../i18n/i18n";
import { formatMinor } from "../money";

export type Role = "quote-author" | "counterparty";

export interface Commitment {
  commitment_id?: string;
  giver: string;
  resource_kind?: string;
  kind?: string;
  description?: { lang?: string; text?: string } | string;
  declared_value_minor?: number | bigint | string;
  window?: { earliest_secs: number; latest_secs: number };
  location?: { where: string; address?: string };
  payee?: string;
  tax_minor?: number | bigint;
  fees_minor?: number | bigint;
  [key: string]: unknown;
}

export interface DealTerms {
  commitments: Commitment[];
  [key: string]: unknown;
}

export interface SideChange {
  added: Commitment[];
  removed: Commitment[];
  changed: { before: Commitment; after: Commitment }[];
  totalBefore: bigint;
  totalAfter: bigint;
}

export interface VersionSummary {
  mine: SideChange;
  theirs: SideChange;
}

function descriptionText(c: Commitment): string {
  if (typeof c.description === "object" && c.description !== null) {
    return c.description.text ?? "";
  }
  return typeof c.description === "string" ? c.description : "";
}

function commitmentsDiffer(a: Commitment, b: Commitment): boolean {
  if (BigInt(a.declared_value_minor ?? 0) !== BigInt(b.declared_value_minor ?? 0)) return true;
  if ((a.payee ?? "") !== (b.payee ?? "")) return true;
  if (BigInt(a.tax_minor ?? 0) !== BigInt(b.tax_minor ?? 0)) return true;
  if (BigInt(a.fees_minor ?? 0) !== BigInt(b.fees_minor ?? 0)) return true;

  const wA = a.window;
  const wB = b.window;
  if (Boolean(wA) !== Boolean(wB)) return true;
  if (wA && wB && (wA.earliest_secs !== wB.earliest_secs || wA.latest_secs !== wB.latest_secs)) return true;

  const lA = a.location;
  const lB = b.location;
  if (Boolean(lA) !== Boolean(lB)) return true;
  if (lA && lB && (lA.where !== lB.where || (lA.address ?? "") !== (lB.address ?? ""))) return true;

  return false;
}

function computeSideChange(beforeList: Commitment[], afterList: Commitment[]): SideChange {
  const added: Commitment[] = [];
  const removed: Commitment[] = [];
  const changed: { before: Commitment; after: Commitment }[] = [];

  let totalBefore = 0n;
  for (const c of beforeList) {
    totalBefore += BigInt(c.declared_value_minor ?? 0);
  }

  let totalAfter = 0n;
  for (const c of afterList) {
    totalAfter += BigInt(c.declared_value_minor ?? 0);
  }

  const matchedAfterIndices = new Set<number>();

  for (const b of beforeList) {
    const bText = descriptionText(b);
    const isMoneyB = b.resource_kind === "currency" || b.kind === "currency";
    let matchIdx = -1;
    for (let i = 0; i < afterList.length; i++) {
      if (matchedAfterIndices.has(i)) continue;
      const a = afterList[i];
      const isMoneyA = a.resource_kind === "currency" || a.kind === "currency";
      if (isMoneyA && isMoneyB) {
        if (a.giver === b.giver && (a.payee ?? "") === (b.payee ?? "")) {
          matchIdx = i;
          break;
        }
      } else if (!isMoneyA && !isMoneyB) {
        if (a.giver === b.giver && a.resource_kind === b.resource_kind && descriptionText(a) === bText) {
          matchIdx = i;
          break;
        }
      }
    }

    if (matchIdx >= 0) {
      matchedAfterIndices.add(matchIdx);
      const a = afterList[matchIdx];
      if (commitmentsDiffer(b, a)) {
        changed.push({ before: b, after: a });
      }
    } else {
      removed.push(b);
    }
  }

  for (let i = 0; i < afterList.length; i++) {
    if (!matchedAfterIndices.has(i)) {
      added.push(afterList[i]);
    }
  }

  return { added, removed, changed, totalBefore, totalAfter };
}

export function versionSummary(
  first: DealTerms,
  latest: DealTerms,
  me: Role,
): VersionSummary {
  const firstMine = (first.commitments || []).filter((c) => c.giver === me);
  const latestMine = (latest.commitments || []).filter((c) => c.giver === me);

  const firstTheirs = (first.commitments || []).filter((c) => c.giver !== me);
  const latestTheirs = (latest.commitments || []).filter((c) => c.giver !== me);

  return {
    mine: computeSideChange(firstMine, latestMine),
    theirs: computeSideChange(firstTheirs, latestTheirs),
  };
}

export function renderVersionSummary(summary: VersionSummary, currency = "INR"): HTMLElement {
  const container = document.createElement("div");
  container.className = "quote-changes-summary";

  const heading = document.createElement("h4");
  heading.className = "quote-changes-heading";
  heading.textContent = t("deal-changes-heading");
  container.appendChild(heading);

  const renderSide = (side: SideChange, isMine: boolean, sideClass: string) => {
    const sideDiv = document.createElement("div");
    sideDiv.className = `quote-changes-side ${sideClass}`;

    const sideHeading = document.createElement("h5");
    sideHeading.className = "quote-changes-side-heading";
    const headingText = isMine
      ? t("deal-changes-side-give", {
          before: formatMinor(side.totalBefore, currency),
          after: formatMinor(side.totalAfter, currency),
        })
      : t("deal-changes-side-receive", {
          before: formatMinor(side.totalBefore, currency),
          after: formatMinor(side.totalAfter, currency),
        });
    sideHeading.textContent = headingText;
    sideDiv.appendChild(sideHeading);

    for (const c of side.added) {
      const p = document.createElement("p");
      p.className = "quote-change-item quote-change-added";
      const desc = descriptionText(c);
      p.textContent = t("deal-changes-added", { description: desc });
      sideDiv.appendChild(p);
    }

    for (const c of side.removed) {
      const p = document.createElement("p");
      p.className = "quote-change-item quote-change-removed";
      const desc = descriptionText(c);
      p.textContent = t("deal-changes-removed", { description: desc });
      sideDiv.appendChild(p);
    }

    for (const ch of side.changed) {
      const p = document.createElement("p");
      p.className = "quote-change-item quote-change-changed";
      const desc = descriptionText(ch.after);
      const valBefore = BigInt(ch.before.declared_value_minor ?? 0);
      const valAfter = BigInt(ch.after.declared_value_minor ?? 0);
      if (valBefore !== valAfter) {
        p.textContent = t("deal-changes-changed-detail", {
          description: desc,
          before: formatMinor(valBefore, currency),
          after: formatMinor(valAfter, currency),
        });
      } else {
        p.textContent = t("deal-changes-changed", { description: desc });
      }
      sideDiv.appendChild(p);
    }

    if (side.added.length > 0 || side.removed.length > 0 || side.changed.length > 0) {
      container.appendChild(sideDiv);
    }
  };

  renderSide(summary.mine, true, "quote-changes-mine");
  renderSide(summary.theirs, false, "quote-changes-theirs");

  return container;
}

