import {
  findListings,
  runEnvelope,
  type MergedHit,
  type RefusedHit,
  type SourceOutcome,
} from "../directory/search";
import { errText, refiller, text } from "../dom";
import { lookupText, messageId, t } from "../i18n/i18n";
import {
  checkedWords,
  issuerChanged,
  membershipWords,
  type CheckStandingReply,
} from "../directory/membership";
import { formatAge } from "../i18n/format";
import { call } from "../rpc";

/// The word for a revocation status. A status this table does not know is
/// shown as it came.
const REVOCATION_LABELS = new Map<string, string>([
  ["good", messageId("directory-revocation-good")],
  ["unknown", messageId("directory-revocation-unknown")],
]);

interface SourceRow {
  did: string;
  label: string;
  last_ok_secs?: number;
  last_error?: { kind: string; message?: string } | null;
}

/// Each directory's label as the person chose it, so a membership line
/// names the directory the way the person knows it.
const sourceLabels = new Map<string, string>();

/// One line per source: what that directory's own signed evidence shows
/// about the listing owner, judged on this node, and a button that asks again.
/// The line is judged against this listing (scope, listing-scoped
/// suspensions). The re-check is not: `directory.check-standing` judges the
/// listing owner's membership alone, so its answer goes on a separate line and
/// never replaces the listing's own verdict.
export function membershipLine(source: MergedHit["sources"][number], hit: MergedHit): HTMLElement {
  const label = sourceLabels.get(source.directory) || source.directory;
  const line = document.createElement("div");
  line.className = "evidence-membership-row";
  const words = text("div", membershipWords(source.membership, label), "evidence-membership");
  const fresh = text("div", "", "evidence-membership-fresh");
  const status = text("div", "", "membership-check-status");
  const again = text("button", t("directory-check-again"), "button check-membership") as HTMLButtonElement;
  again.onclick = async () => {
    again.disabled = true;
    status.textContent = "";
    try {
      const res = await call<CheckStandingReply>("directory.check-standing", {
        source: source.directory,
        member_did: hit.issuer,
      });
      fresh.textContent = issuerChanged(res)
        ? ""
        : t("directory-membership-alone", { words: membershipWords(res.verdict, label) });
      status.textContent = checkOutcomeWords(res);
    } catch (err) {
      status.textContent = t("directory-check-failed", { detail: errText(err) });
    }
    again.disabled = false;
  };
  line.append(words, again, fresh, status);
  return line;
}

function checkOutcomeWords(res: CheckStandingReply): string {
  if (res.refreshed) return t("directory-checked-now");
  if (issuerChanged(res)) return t("membership-issuer-changed-check");
  return t("directory-unreachable", { checked: checkedWords(res.as_of_secs ?? 0) });
}

function sourceErrorWords(kind: string): string {
  switch (kind) {
    case "not-started":
      return t("directory-source-error-not-started");
    case "timed-out":
      return t("directory-source-error-timed-out");
    case "not-found":
      return t("directory-source-error-not-found");
    case "refused":
      return t("directory-source-error-refused");
    default:
      return t("directory-source-error-unreadable");
  }
}

export async function renderDirectory(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "directory-screen";
  box.appendChild(text("h2", t("directory-title")));
  box.appendChild(text("p", t("directory-intro"), "directory-intro"));

  const sourcesHost = document.createElement("div");
  sourcesHost.className = "directory-sources";
  box.appendChild(sourcesHost);
  box.appendChild(buildAddSource(() => reloadSources()));

  box.appendChild(buildSearch());

  const reloadSources = refiller(sourcesHost, async (into) => {
    into.appendChild(text("h3", t("directory-yours-heading")));
    let rows: SourceRow[] = [];
    try {
      const res = await call<{ sources: SourceRow[] }>("directory.sources");
      rows = res.sources ?? [];
    } catch (err) {
      into.appendChild(text("p", t("directory-load-failed", { detail: errText(err) })));
      return;
    }
    if (rows.length === 0) {
      into.appendChild(text("p", t("directory-empty"), "directory-empty"));
      return;
    }
    for (const row of rows) {
      sourceLabels.set(row.did, row.label);
      const line = document.createElement("div");
      line.className = "directory-source-row";
      line.appendChild(text("span", row.label || row.did, "source-label"));
      line.appendChild(text("span", row.did, "source-did"));
      if (row.last_error) {
        line.appendChild(
          text(
            "span",
            t("directory-last-error", { reason: sourceErrorWords(row.last_error.kind) }),
            "source-last-error",
          ),
        );
      } else if (row.last_ok_secs) {
        line.appendChild(text("span", t("directory-answered-recently"), "source-last-ok"));
      }
      const rm = text("button", t("common-remove"), "button remove-source") as HTMLButtonElement;
      rm.onclick = async () => {
        rm.disabled = true;
        try {
          await call("directory.remove-source", { did: row.did });
          await reloadSources();
        } catch (err) {
          rm.disabled = false;
          line.appendChild(
            text("p", t("directory-remove-failed", { detail: errText(err) }), "row-error"),
          );
        }
      };
      line.appendChild(rm);
      into.appendChild(line);
    }
  });

  await reloadSources();
  container.appendChild(box);
}

function buildAddSource(onAdded: () => Promise<void>): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "add-source";
  wrap.appendChild(text("h3", t("directory-add-heading")));
  wrap.appendChild(text("p", t("directory-add-help")));
  const didInput = document.createElement("input");
  didInput.className = "add-source-did";
  didInput.placeholder = t("directory-add-did-placeholder");
  const labelInput = document.createElement("input");
  labelInput.className = "add-source-label";
  labelInput.placeholder = t("directory-add-label-placeholder");
  const addBtn = text("button", t("directory-add-button"), "button add-source-btn") as HTMLButtonElement;
  const status = text("p", "", "add-source-status");

  addBtn.onclick = async () => {
    status.textContent = "";
    const did = didInput.value.trim();
    if (!did) return;
    addBtn.disabled = true;
    try {
      const res = await call<{ source: SourceRow; probe: string | null }>("directory.add-source", {
        did,
        label: labelInput.value.trim() || undefined,
      });
      if (res.probe) {
        // A probe that answered but returned no SynOrg -- stored anyway
        // (the SynOrg may be created later) but the person is told, so
        // they do not sit waiting for results that will never come.
        status.textContent = t("directory-added-note", { probe: res.probe });
      } else if (res.source.last_error) {
        status.textContent = t("directory-added-silent");
      } else {
        status.textContent = t("directory-added");
      }
      didInput.value = "";
      labelInput.value = "";
      await onAdded();
    } catch (err) {
      status.textContent = t("directory-add-failed", { detail: errText(err) });
    }
    addBtn.disabled = false;
  };

  wrap.append(didInput, labelInput, addBtn, status);
  return wrap;
}

function buildSearch(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "directory-search";
  wrap.appendChild(text("h3", t("directory-search-heading")));

  const textInput = document.createElement("input");
  textInput.className = "search-text";
  textInput.placeholder = t("directory-search-text-placeholder");
  const catInput = document.createElement("input");
  catInput.className = "search-categories";
  catInput.placeholder = t("directory-search-categories-placeholder");
  const help = text("p", t("directory-search-help"), "search-help");
  const goBtn = text("button", t("directory-search-button"), "button run-search") as HTMLButtonElement;
  const progress = text("p", "", "search-progress");
  const results = document.createElement("div");
  results.className = "search-results";
  const refusedHost = document.createElement("div");
  refusedHost.className = "search-refused";
  const errorsHost = document.createElement("div");
  errorsHost.className = "search-source-errors";

  goBtn.onclick = async () => {
    results.replaceChildren();
    refusedHost.replaceChildren();
    errorsHost.replaceChildren();
    progress.textContent = t("directory-searching");
    goBtn.disabled = true;

    const query: Record<string, unknown> = {};
    if (textInput.value.trim()) query.text = textInput.value.trim();
    const cats = catInput.value
      .split(",")
      .map((c) => c.trim())
      .filter(Boolean);
    if (cats.length) query.categories = cats;

    try {
      const res = await findListings(query);
      if (res.outcomes.length > 0 && res.outcomes.every((o) => o.kind === "not-started")) {
        progress.textContent = t("directory-search-not-started", {
          button: t("directory-search-button"),
        });
        return;
      }
      progress.textContent =
        res.outcomes.length === 0
          ? t("directory-no-sources")
          : t("directory-searched", { count: res.outcomes.length });
      results.replaceChildren();
      refusedHost.replaceChildren();
      errorsHost.replaceChildren();
      for (const outcome of res.outcomes) {
        renderSourceOutcome(errorsHost, outcome);
      }
      renderHits(results, res.hits, res.hitsTruncated, res.runId);
      renderRefused(refusedHost, res.refused, res.refusedTruncated);
    } catch (err) {
      progress.textContent =
        err instanceof Error
          ? errText(err)
          : t("directory-search-failed", {
              detail: String(err),
              button: t("directory-search-button"),
            });
    } finally {
      goBtn.disabled = false;
    }
  };

  wrap.append(textInput, catInput, help, goBtn, progress, results, refusedHost, errorsHost);
  return wrap;
}

function renderSourceOutcome(
  host: HTMLElement,
  outcome: SourceOutcome,
) {
  const clean = outcome.kind === "ok" && !outcome.truncated;
  if (clean) return;
  const line = document.createElement("div");
  line.className = "source-error-line";
  line.dataset.kind = outcome.kind === "ok" ? "truncated" : outcome.kind;
  line.appendChild(text("span", outcome.source, "source-error-did"));
  const reason =
    outcome.kind === "ok"
      ? t("directory-more-than-returned")
      : sourceErrorWords(outcome.kind);
  line.appendChild(text("span", reason, "source-error-reason"));
  host.appendChild(line);
}

function renderHits(host: HTMLElement, hits: MergedHit[], truncated: boolean, runId: string) {
  host.appendChild(text("h3", t("directory-results-heading")));
  if (hits.length === 0) {
    host.appendChild(text("p", t("directory-no-results"), "no-results"));
    return;
  }
  for (const hit of hits) {
    const card = document.createElement("div");
    card.className = "search-hit";
    card.dataset.verified = String(hit.verified);

    card.appendChild(text("div", hit.title || t("directory-untitled"), "hit-title"));
    card.appendChild(text("div", hit.summary, "hit-summary"));
    if (hit.categories.length) {
      card.appendChild(
        text("div", t("directory-hit-categories", { categories: hit.categories.join(", ") }), "hit-categories"),
      );
    }
    card.appendChild(text("div", t("directory-hit-offered-by", { issuer: hit.issuer }), "hit-issuer"));
    card.appendChild(text("div", t("directory-signed-age", { age: formatAge(hit.age_secs) }), "hit-age"));

    const evidence = document.createElement("div");
    evidence.className = "hit-evidence";
    // Never the word "verified" without a qualifier, and every missing
    // piece of evidence is spelled "unknown" / "not checked", never left
    // absent for the reader to assume the best.
    evidence.appendChild(
      text(
        "div",
        hit.verified
          ? t("directory-signature-ok")
          : t("directory-signature-bad"),
        "evidence-signature",
      ),
    );
    evidence.appendChild(
      text(
        "div",
        t("directory-revocation", {
          status: lookupText(REVOCATION_LABELS, hit.revocation_status) ?? hit.revocation_status,
        }),
        "evidence-revocation",
      ),
    );
    for (const source of hit.sources) evidence.appendChild(membershipLine(source, hit));
    card.appendChild(evidence);

    const from = hit.sources.map((s) => s.directory).join(", ");
    card.appendChild(text("div", t("directory-seen-at", { from }), "hit-sources"));
    if (hit.versions_differ) {
      card.appendChild(
        text("div", t("directory-versions-differ"), "hit-versions-differ"),
      );
    }

    if (hit.verified) {
      const engage = text("button", t("directory-engage-button"), "button hit-engage") as HTMLButtonElement;
      const engageOut = text("div", "", "hit-engage-out");
      engage.onclick = async () => {
        engageOut.textContent = "";
        try {
          const envelope = await runEnvelope(runId, hit.record_id);
          if (!envelope) {
            engageOut.textContent = t("directory-engage-gone");
            return;
          }
          engageOut.textContent = t("directory-engage-address", {
            address: hit.conversation_address,
          });
        } catch (err) {
          engageOut.textContent = t("directory-engage-failed", { detail: errText(err) });
        }
      };
      card.append(engage, engageOut);
    }

    host.appendChild(card);
  }
  if (truncated) {
    host.appendChild(text("p", t("directory-truncated"), "hits-truncated"));
  }
}

function renderRefused(host: HTMLElement, refused: RefusedHit[], truncated: boolean) {
  if (refused.length === 0) return;
  host.appendChild(text("h3", t("directory-refused-heading")));
  host.appendChild(text("p", t("directory-refused-intro"), "refused-intro"));
  for (const r of refused) {
    const line = document.createElement("div");
    line.className = "refused-hit";
    line.appendChild(text("div", r.reason || t("directory-refused-fallback"), "refused-reason"));
    line.appendChild(
      text("div", t("directory-served-by", { sources: r.sources.join(", ") }), "refused-sources"),
    );
    host.appendChild(line);
  }
  if (truncated) {
    host.appendChild(text("p", t("directory-refused-truncated"), "refused-truncated"));
  }
}
