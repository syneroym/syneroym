import { renderCard } from "./cards/render";
import { renderRefusedCard } from "./cards/refused";
import { runSearch } from "./directory/search";
import { renderBackup } from "./screens/backup";
import { renderContacts } from "./screens/contacts";
import { renderDirectory } from "./screens/directory";
import { renderGroups } from "./screens/groups";
import { renderListings } from "./screens/listings";
import { renderMemberships } from "./screens/memberships";
import {
  ADDRESS_DISCLOSURE_NOTICE,
  DECLINE_NOTE,
  DEFAULT_DATA_USE_NOTICE,
  openDeclineDialog,
  openQuoteForm,
  renderCardActions,
  renderMessages,
} from "./screens/messages";
import { renderProfile } from "./screens/profile";
import { renderSafety } from "./screens/safety";
import { renderSetup } from "./screens/setup";
import { renderSynOrg } from "./screens/synorg";
import { pendingEnrolment } from "./session/enrolment";
import {
  authHeaders,
  fetchMethods,
  hasStoredKey,
  loginWithBundle,
  loginWithStoredKey,
  logout,
  storedToken,
  whoami,
  type SessionKeyBundle,
} from "./session/login";

declare global {
  interface Window {
    RoymRegistry?: {
      renderCard: typeof renderCard;
      renderRefusedCard: typeof renderRefusedCard;
    };
    RoymSession?: {
      storedToken: typeof storedToken;
      authHeaders: typeof authHeaders;
      whoami: typeof whoami;
    };
    RoymDirectory?: {
      runSearch: typeof runSearch;
    };
    RoymMessages?: {
      openQuoteForm: typeof openQuoteForm;
      openDeclineDialog: typeof openDeclineDialog;
      renderCardActions: typeof renderCardActions;
      DEFAULT_DATA_USE_NOTICE: string;
      ADDRESS_DISCLOSURE_NOTICE: string;
      DECLINE_NOTE: string;
    };
  }
}

// Test hook: lets the e2e suite call the real card renderer directly with a
// crafted fixture, instead of only ever seeing the sample gallery below.
window.RoymRegistry = { renderCard, renderRefusedCard };
window.RoymSession = { storedToken, authHeaders, whoami };
// Test hook: the `roym-hub.spec.ts` `NotStarted` case drives the real
// client loop with `ignoreConcurrency` so it can oversubscribe this node's
// admission limit on purpose.
window.RoymDirectory = { runSearch };
window.RoymMessages = {
  openQuoteForm,
  openDeclineDialog,
  renderCardActions,
  DEFAULT_DATA_USE_NOTICE,
  ADDRESS_DISCLOSURE_NOTICE,
  DECLINE_NOTE,
};

async function main() {
  const app = document.getElementById("app");
  if (!app) return;

  const title = document.createElement("h1");
  title.textContent = "Roym Hub";
  app.appendChild(title);

  const sessionBar = document.createElement("div");
  sessionBar.className = "session-bar";
  app.appendChild(sessionBar);

  const contentArea = document.createElement("div");
  contentArea.className = "content-area";
  app.appendChild(contentArea);

  // Check current session. No token, a 401, or a restarted substrate is an
  // ordinary "log in again" state, never an error banner.
  const whoamiData = await whoami();

  const did = whoamiData?.person_did;
  if (did) {
    const didSpan = document.createElement("span");
    didSpan.textContent = `Logged in: ${did} (${whoamiData?.auth || "delegated"})`;
    sessionBar.appendChild(didSpan);

    const logoutBtn = document.createElement("button");
    logoutBtn.className = "button";
    logoutBtn.textContent = "Log out";
    logoutBtn.onclick = async () => {
      await logout();
      window.location.reload();
    };
    sessionBar.appendChild(logoutBtn);

    const pending = await pendingEnrolment();
    if (pending.length === 0) {
      renderTabs(contentArea, did);
    } else {
      await renderSetup(contentArea, () => renderTabs(contentArea, did), pending);
    }
  } else {
    const anonSpan = document.createElement("span");
    anonSpan.textContent = "Not logged in";
    sessionBar.appendChild(anonSpan);
    await renderLogin(contentArea);
  }
}

async function renderTabs(container: HTMLElement, did: string) {
  container.replaceChildren();

  const nav = document.createElement("div");
  nav.className = "tab-nav";
  nav.style.display = "flex";
  nav.style.gap = "12px";
  nav.style.marginBottom = "16px";

  // Each tab render gets its own fresh container that replaces the old one.
  // A screen that is still loading (it awaits the node before it appends)
  // then writes into a container that is already detached, so a quick tab
  // click can never leave two screens in the page.
  let tabContainer = document.createElement("div");
  tabContainer.className = "tab-content";
  container.appendChild(nav);
  container.appendChild(tabContainer);

  const show = (render: (host: HTMLElement) => Promise<void>) => {
    const host = document.createElement("div");
    host.className = "tab-content";
    tabContainer.replaceWith(host);
    tabContainer = host;
    return render(host);
  };

  const tabs = [
    { name: "Profile", render: () => show((host) => renderProfile(host)) },
    { name: "Contacts", render: () => show((host) => renderContacts(host)) },
    { name: "Messages", render: () => show((host) => renderMessages(host)) },
    { name: "Groups", render: () => show((host) => renderGroups(host)) },
    { name: "Listings", render: () => show((host) => renderListings(host)) },
    { name: "Directory", render: () => show((host) => renderDirectory(host)) },
    { name: "Memberships", render: () => show((host) => renderMemberships(host)) },
    { name: "SynOrg", render: () => show((host) => renderSynOrg(host)) },
    { name: "Safety", render: () => show((host) => renderSafety(host)) },
    { name: "Backup", render: () => show((host) => renderBackup(host)) },
    { name: "Components", render: () => show((host) => renderHome(host, did)) },
  ];

  for (let i = 0; i < tabs.length; i++) {
    const btn = document.createElement("button");
    btn.className = "button tab-button";
    btn.textContent = tabs[i].name;
    btn.onclick = () => {
      tabs[i].render();
    };
    nav.appendChild(btn);
  }

  // Default to Profile tab
  await tabs[0].render();
}

async function renderLogin(container: HTMLElement) {
  container.replaceChildren();

  const box = document.createElement("div");
  box.className = "login-picker";
  const heading = document.createElement("h2");
  heading.textContent = "Sign in";
  box.appendChild(heading);

  const status = document.createElement("p");
  status.className = "login-status";
  const setStatus = (text: string) => {
    status.textContent = text;
  };

  let methods: string[] = [];
  try {
    methods = (await fetchMethods()).methods;
  } catch {
    setStatus("Could not reach the auth service on this node.");
    box.appendChild(status);
    container.appendChild(box);
    return;
  }

  if (!methods.includes("delegated-key")) {
    setStatus("This node has no browser login method enabled.");
    box.appendChild(status);
    container.appendChild(box);
    return;
  }

  const intro = document.createElement("p");
  intro.textContent =
    "Load a session key file produced by " +
    "`roymctl session delegate --as <name> --registry-url <registry>`. " +
    "The key stays in this browser and cannot be read back by page code.";
  box.appendChild(intro);

  // Re-use a delegated key already held in this browser, if any.
  if (await hasStoredKey()) {
    const reuseBtn = document.createElement("button");
    reuseBtn.className = "button";
    reuseBtn.textContent = "Sign in with this browser's key";
    reuseBtn.onclick = async () => {
      reuseBtn.disabled = true;
      setStatus("Signing in...");
      try {
        if (await loginWithStoredKey()) {
          window.location.reload();
          return;
        }
        setStatus("The stored key is gone; load a session key file.");
      } catch (e) {
        setStatus(`Sign-in failed: ${e instanceof Error ? e.message : String(e)}`);
      }
      reuseBtn.disabled = false;
    };
    box.appendChild(reuseBtn);
  }

  const fileLabel = document.createElement("label");
  fileLabel.className = "button file-button";
  fileLabel.textContent = "Load session key file";
  const fileInput = document.createElement("input");
  fileInput.type = "file";
  fileInput.accept = "application/json,.json";
  fileInput.style.display = "none";
  fileInput.onchange = async () => {
    const file = fileInput.files?.[0];
    if (!file) return;
    setStatus("Signing in...");
    try {
      const bundle = JSON.parse(await file.text()) as SessionKeyBundle;
      await loginWithBundle(bundle);
      window.location.reload();
    } catch (e) {
      setStatus(`Sign-in failed: ${e instanceof Error ? e.message : String(e)}`);
      fileInput.value = "";
    }
  };
  fileLabel.appendChild(fileInput);
  box.appendChild(fileLabel);

  box.appendChild(status);
  container.appendChild(box);
}

async function renderHome(container: HTMLElement, did: string) {
  container.replaceChildren();

  const homeSection = document.createElement("div");
  const h2 = document.createElement("h2");
  h2.textContent = "Roym Hub Home";
  homeSection.appendChild(h2);

  const didP = document.createElement("p");
  didP.textContent = `Authenticated DID: ${did}`;
  homeSection.appendChild(didP);

  // Ping profile service via POST /rpc
  const rpcP = document.createElement("p");
  rpcP.textContent = "Contacting profile service...";
  homeSection.appendChild(rpcP);

  try {
    const rpcRes = await fetch("/rpc", {
      method: "POST",
      headers: { "Content-Type": "application/json", ...authHeaders() },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "profile.ping",
        params: {},
      }),
    });
    if (rpcRes.ok) {
      const rpcJson = await rpcRes.json();
      rpcP.textContent = `Profile ping response: ${JSON.stringify(rpcJson)}`;
    } else {
      rpcP.textContent = `Profile ping failed: HTTP ${rpcRes.status}`;
    }
  } catch (e) {
    rpcP.textContent = `Profile ping error: ${e}`;
  }

  // Card gallery section
  const gallerySection = document.createElement("div");
  gallerySection.style.marginTop = "24px";
  const galleryH3 = document.createElement("h3");
  galleryH3.textContent = "Card Gallery (Sample Cards)";
  gallerySection.appendChild(galleryH3);

  const samples = [
    {
      type: "request",
      version: 1,
      data: {
        description: "Front brake cable replacement and tuning",
        categories: ["cycling", "repair"],
      },
    },
    {
      type: "quote",
      version: 1,
      data: {
        terms: {
          scope: "Replace brake cable and adjust pads",
          currency: "EUR",
          amount_minor: 4500,
          tax_minor: 500,
          fees_minor: 0,
          payee: "Y Repairs",
          payment_timing: "after-work",
          schedule: {
            earliest_secs: 1800000000,
            latest_secs: 1800003600,
          },
          location: {
            where: "at-receiver",
            address: "123 High Street",
          },
          cancellation_terms: "24 hours notice required for full refund",
          refund_terms: "Full refund if work not completed",
          dispute_path: "Small claims court or informal mediation",
          quote_expires_at_secs: 1900000000,
        },
      },
    },
    {
      type: "agreement-receipt",
      version: 1,
      data: {
        quote_record_id: "rec_quote12345",
        counterparty_did: "did:key:z6MkhaXgBZDvotDkL5257faiz4898Xnp213as98z",
        quote_author_did: "did:key:z6MkmFaqYQ36Xk559C351gJ1jF8d7s86s511as88",
        role: "counterparty",
        pair: { state: "complete" },
        terms: {
          scope: "Replace brake cable and adjust pads",
          currency: "EUR",
          amount_minor: 4500,
          tax_minor: 500,
          fees_minor: 0,
          payee: "Y Repairs",
          payment_timing: "after-work",
          schedule: {
            earliest_secs: 1800000000,
            latest_secs: 1800003600,
          },
          location: {
            where: "at-receiver",
            address: "123 High Street",
          },
          cancellation_terms: "24 hours notice required for full refund",
          refund_terms: "Full refund if work not completed",
          dispute_path: "Small claims court or informal mediation",
          quote_expires_at_secs: 1900000000,
        },
      },
    },
    { type: "booking-progress", version: 1, data: { progress: "In Progress" } },
    { type: "payment-request", version: 1, data: { amount: "$120", currency: "USD" } },
    { type: "payment-acknowledgement", version: 1, data: { receipt_id: "rcpt-456" } },
    { type: "fulfilment-receipt", version: 1, data: { tracking: "TRK-789" } },
    { type: "not-a-real-type", version: 1, data: {} },
    { type: "quote", version: 99, data: {} },
  ];

  for (const sample of samples) {
    gallerySection.appendChild(renderCard(sample));
  }

  homeSection.appendChild(gallerySection);
  container.appendChild(homeSection);
}

window.addEventListener("DOMContentLoaded", main);
