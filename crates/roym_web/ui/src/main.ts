import { renderCard } from "./cards/render";
import { call } from "./rpc";
import { renderBackup } from "./screens/backup";
import { renderContacts } from "./screens/contacts";
import { renderProfile } from "./screens/profile";
import { renderSafety } from "./screens/safety";
import { renderSetup } from "./screens/setup";
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
    RoymRegistry?: { renderCard: typeof renderCard };
    RoymSession?: {
      storedToken: typeof storedToken;
      authHeaders: typeof authHeaders;
      whoami: typeof whoami;
    };
  }
}

// Test hook: lets the e2e suite call the real card renderer directly with a
// crafted fixture, instead of only ever seeing the sample gallery below.
window.RoymRegistry = { renderCard };
window.RoymSession = { storedToken, authHeaders, whoami };

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

    const statusVal = await call<{ certificate: { state: string } }>("profile.signing-status").catch(() => null);
    if (statusVal?.certificate?.state === "installed") {
      renderTabs(contentArea, did);
    } else {
      await renderSetup(contentArea, () => renderTabs(contentArea, did));
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

  const tabContainer = document.createElement("div");
  tabContainer.className = "tab-content";

  const tabs = [
    { name: "Profile", render: () => renderProfile(tabContainer) },
    { name: "Contacts", render: () => renderContacts(tabContainer) },
    { name: "Safety", render: () => renderSafety(tabContainer) },
    { name: "Backup", render: () => renderBackup(tabContainer) },
    { name: "Components", render: () => renderHome(tabContainer, did) },
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

  container.appendChild(nav);
  container.appendChild(tabContainer);

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
    { type: "request", version: 1, data: { summary: "Car repair quote request" } },
    { type: "quote", version: 1, data: { price: "$120" } },
    { type: "agreement-receipt", version: 1, data: { agreement_id: "agr-123" } },
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
