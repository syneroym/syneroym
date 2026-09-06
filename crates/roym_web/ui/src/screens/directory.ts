import {
  runEnvelope,
  runSearch,
  type MergedHit,
  type MergeResult,
  type RefusedHit,
  type SourceOutcome,
} from "../directory/search";
import { call, RpcError } from "../rpc";

interface SourceRow {
  did: string;
  label: string;
  last_ok_secs?: number;
  last_error?: { kind: string; message?: string } | null;
}

function errText(err: unknown): string {
  if (err instanceof RpcError) return err.message;
  return err instanceof Error ? err.message : String(err);
}

/// A stranger's directory label, listing text or error string is only ever
/// a text node -- never markup, never a URL turned into a link.
function text(tag: string, value: string, className?: string): HTMLElement {
  const el = document.createElement(tag);
  el.textContent = value;
  if (className) el.className = className;
  return el;
}

/// Freshness shown to the person, always spelled in words and always
/// computed from `age_secs`, which the node already computed on this
/// person's own clock.
function ageWords(secs: number): string {
  if (secs < 90) return "moments ago";
  const mins = Math.round(secs / 60);
  if (mins < 90) return `${mins} minutes ago`;
  const hours = Math.round(secs / 3600);
  if (hours < 48) return `${hours} hours ago`;
  return `${Math.round(secs / 86400)} days ago`;
}

function sourceErrorWords(kind: string): string {
  switch (kind) {
    case "not-started":
      return "this installation was busy and did not start the call";
    case "timed-out":
      return "the directory did not answer in time";
    case "not-found":
      return "no directory answers at that address";
    case "refused":
      return "the directory refused the request";
    default:
      return "the directory's answer could not be read";
  }
}

export async function renderDirectory(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "directory-screen";
  box.appendChild(text("h2", "Directories"));
  box.appendChild(
    text(
      "p",
      "A directory is a place you choose to search. It is optional: you can " +
        "always reach a provider directly from a link they send you. Your node " +
        "checks every result itself -- a directory is never trusted.",
      "directory-intro",
    ),
  );

  const sourcesHost = document.createElement("div");
  sourcesHost.className = "directory-sources";
  box.appendChild(sourcesHost);
  box.appendChild(buildAddSource(() => reloadSources()));

  box.appendChild(buildSearch());

  async function reloadSources() {
    sourcesHost.replaceChildren();
    sourcesHost.appendChild(text("h3", "Your directories"));
    let rows: SourceRow[] = [];
    try {
      const res = await call<{ sources: SourceRow[] }>("directory.sources");
      rows = res.sources ?? [];
    } catch (err) {
      sourcesHost.appendChild(text("p", `Could not load your directories: ${errText(err)}`));
      return;
    }
    if (rows.length === 0) {
      sourcesHost.appendChild(text("p", "No directories added yet.", "directory-empty"));
      return;
    }
    for (const row of rows) {
      const line = document.createElement("div");
      line.className = "directory-source-row";
      line.appendChild(text("span", row.label || row.did, "source-label"));
      line.appendChild(text("span", row.did, "source-did"));
      if (row.last_error) {
        line.appendChild(
          text("span", `last error: ${sourceErrorWords(row.last_error.kind)}`, "source-last-error"),
        );
      } else if (row.last_ok_secs) {
        line.appendChild(text("span", "answered recently", "source-last-ok"));
      }
      const rm = text("button", "Remove", "button remove-source") as HTMLButtonElement;
      rm.onclick = async () => {
        rm.disabled = true;
        try {
          await call("directory.remove-source", { did: row.did });
          await reloadSources();
        } catch (err) {
          rm.disabled = false;
          line.appendChild(text("p", `Remove failed: ${errText(err)}`, "row-error"));
        }
      };
      line.appendChild(rm);
      sourcesHost.appendChild(line);
    }
  }

  await reloadSources();
  container.appendChild(box);
}

function buildAddSource(onAdded: () => Promise<void>): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "add-source";
  wrap.appendChild(text("h3", "Add a directory"));
  wrap.appendChild(
    text(
      "p",
      "Paste the Roym Directory service DID a friend, a referral or a " +
        "well-known list gave you. There is no directory of directories.",
    ),
  );
  const didInput = document.createElement("input");
  didInput.className = "add-source-did";
  didInput.placeholder = "did:key:... (Roym Directory service DID)";
  const labelInput = document.createElement("input");
  labelInput.className = "add-source-label";
  labelInput.placeholder = "your label for it (optional)";
  const addBtn = text("button", "Add directory", "button add-source-btn") as HTMLButtonElement;
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
        status.textContent = `Added. Note: ${res.probe}.`;
      } else if (res.source.last_error) {
        status.textContent = `Added, but the directory did not answer a check just now.`;
      } else {
        status.textContent = "Added.";
      }
      didInput.value = "";
      labelInput.value = "";
      await onAdded();
    } catch (err) {
      status.textContent = `Could not add: ${errText(err)}`;
    }
    addBtn.disabled = false;
  };

  wrap.append(didInput, labelInput, addBtn, status);
  return wrap;
}

function buildSearch(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "directory-search";
  wrap.appendChild(text("h3", "Search your directories"));

  const textInput = document.createElement("input");
  textInput.className = "search-text";
  textInput.placeholder = "words to look for (optional)";
  const catInput = document.createElement("input");
  catInput.className = "search-categories";
  catInput.placeholder = "categories, comma separated (optional)";
  const help = text(
    "p",
    "Matching is on whole words in the title, summary and categories. " +
      "Punctuation is ignored, so searching for 50% finds 50. There is no " +
      "ranking and no paid placement -- results are shown newest first, " +
      "one directory at a time in turn.",
    "search-help",
  );
  const goBtn = text("button", "Search", "button run-search") as HTMLButtonElement;
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
    progress.textContent = "Starting...";
    goBtn.disabled = true;

    const query: Record<string, unknown> = {};
    if (textInput.value.trim()) query.text = textInput.value.trim();
    const cats = catInput.value
      .split(",")
      .map((c) => c.trim())
      .filter(Boolean);
    if (cats.length) query.categories = cats;

    try {
      let lastRunId = "";
      const { run, merged } = await runSearch(query, async (outcome, prog) => {
        lastRunId = prog.runId;
        progress.textContent = `${prog.answered} of ${prog.total} directories answered...`;
        renderSourceOutcome(errorsHost, outcome);
        // Render what has come in so far while slower sources are still
        // outstanding -- `merge` returns whatever `query-source` rows exist
        // at the moment it is called.
        try {
          const partial = await call<MergeResult>("directory.merge", { run_id: prog.runId });
          results.replaceChildren();
          refusedHost.replaceChildren();
          renderHits(results, partial.hits, partial.hits_truncated, prog.runId);
          renderRefused(refusedHost, partial.refused, partial.refused_truncated);
        } catch {
          /* the final merge below is authoritative */
        }
      });
      progress.textContent =
        run.sources.length === 0
          ? "You have not added any directories. Add one above, or reach a provider by a direct link."
          : `${run.sources.length} directories searched.`;
      results.replaceChildren();
      refusedHost.replaceChildren();
      renderHits(results, merged.hits, merged.hits_truncated, run.runId || lastRunId);
      renderRefused(refusedHost, merged.refused, merged.refused_truncated);
    } catch (err) {
      progress.textContent = `Search failed: ${errText(err)}`;
    }
    goBtn.disabled = false;
  };

  wrap.append(textInput, catInput, help, goBtn, progress, results, refusedHost, errorsHost);
  return wrap;
}

function renderSourceOutcome(host: HTMLElement, outcome: SourceOutcome) {
  if (outcome.kind === "ok") return;
  const line = document.createElement("div");
  line.className = "source-error-line";
  line.dataset.kind = outcome.kind;
  line.appendChild(text("span", outcome.source, "source-error-did"));
  line.appendChild(text("span", sourceErrorWords(outcome.kind), "source-error-reason"));
  host.appendChild(line);
}

function renderHits(host: HTMLElement, hits: MergedHit[], truncated: boolean, runId: string) {
  host.appendChild(text("h3", "Results"));
  if (hits.length === 0) {
    host.appendChild(text("p", "No results.", "no-results"));
    return;
  }
  for (const hit of hits) {
    const card = document.createElement("div");
    card.className = "search-hit";
    card.dataset.verified = String(hit.verified);

    card.appendChild(text("div", hit.title || "(untitled)", "hit-title"));
    card.appendChild(text("div", hit.summary, "hit-summary"));
    if (hit.categories.length) {
      card.appendChild(text("div", `Categories: ${hit.categories.join(", ")}`, "hit-categories"));
    }
    card.appendChild(text("div", `Offered by: ${hit.issuer}`, "hit-issuer"));
    card.appendChild(text("div", `Signed ${ageWords(hit.age_secs)}`, "hit-age"));

    const evidence = document.createElement("div");
    evidence.className = "hit-evidence";
    // Never the word "verified" without a qualifier, and every missing
    // piece of evidence is spelled "unknown" / "not checked", never left
    // absent for the reader to assume the best.
    evidence.appendChild(
      text(
        "div",
        hit.verified
          ? "signature: checked on your node"
          : "signature: did NOT check out -- treat as unknown",
        "evidence-signature",
      ),
    );
    evidence.appendChild(text("div", `revocation: ${hit.revocation_status}`, "evidence-revocation"));
    evidence.appendChild(
      text("div", "membership: not checked", "evidence-membership"),
    );
    card.appendChild(evidence);

    const from = hit.sources.map((s) => s.directory).join(", ");
    card.appendChild(text("div", `Seen at: ${from}`, "hit-sources"));
    if (hit.versions_differ) {
      card.appendChild(
        text(
          "div",
          "Two directories disagree about which version of this offer is current. " +
            "The newer signed version is shown.",
          "hit-versions-differ",
        ),
      );
    }

    if (hit.verified) {
      const engage = text("button", "Message this provider", "button hit-engage") as HTMLButtonElement;
      const engageOut = text("div", "", "hit-engage-out");
      engage.onclick = async () => {
        engageOut.textContent = "";
        try {
          const envelope = await runEnvelope(runId, hit.record_id);
          if (!envelope) {
            engageOut.textContent = "The full listing is no longer in this run.";
            return;
          }
          engageOut.textContent =
            `Conversation address: ${hit.conversation_address} -- open the Messages tab ` +
            `and start a conversation with this address.`;
        } catch (err) {
          engageOut.textContent = `Could not open: ${errText(err)}`;
        }
      };
      card.append(engage, engageOut);
    }

    host.appendChild(card);
  }
  if (truncated) {
    host.appendChild(text("p", "More results were found than are shown here.", "hits-truncated"));
  }
}

function renderRefused(host: HTMLElement, refused: RefusedHit[], truncated: boolean) {
  if (refused.length === 0) return;
  host.appendChild(text("h3", "Refused evidence"));
  host.appendChild(
    text(
      "p",
      "One or more directories served these, but their signatures did not " +
        "check out on your node. They are shown so you know a directory served " +
        "them. They are never treated as real offers.",
      "refused-intro",
    ),
  );
  for (const r of refused) {
    const line = document.createElement("div");
    line.className = "refused-hit";
    line.appendChild(text("div", r.reason || "signature did not verify", "refused-reason"));
    line.appendChild(text("div", `Served by: ${r.sources.join(", ")}`, "refused-sources"));
    host.appendChild(line);
  }
  if (truncated) {
    host.appendChild(text("p", "More refused evidence was served than is shown here.", "refused-truncated"));
  }
}
