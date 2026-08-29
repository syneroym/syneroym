import { renderCard } from "./cards/render";
import {
  fetchMethods,
  hasStoredKey,
  loginWithBundle,
  loginWithStoredKey,
  logout,
  type SessionKeyBundle,
} from "./session/login";

declare global {
  interface Window {
    RoymRegistry?: { renderCard: typeof renderCard };
  }
}

// Test hook: lets the e2e suite call the real card renderer directly with a
// crafted fixture, instead of only ever seeing the sample gallery below.
window.RoymRegistry = { renderCard };

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

  // Check current session. A 401 (or a restarted substrate) is an ordinary
  // "log in again" state, never an error banner.
  let whoamiData: { person_did?: string; auth?: string; did?: string } | null = null;
  try {
    const res = await fetch("/_syneroym/session/whoami");
    if (res.status === 200) {
      whoamiData = await res.json();
    }
  } catch {
    // Network or server unreachable
  }

  const did = whoamiData?.person_did || whoamiData?.did;
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

    renderHome(contentArea, did);
  } else {
    const anonSpan = document.createElement("span");
    anonSpan.textContent = "Not logged in";
    sessionBar.appendChild(anonSpan);
    await renderLogin(contentArea);
  }
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
      headers: { "Content-Type": "application/json" },
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
