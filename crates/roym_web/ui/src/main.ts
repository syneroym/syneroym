import { renderCard } from "./cards/render";

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

  // Check current session
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
      await fetch("/_syneroym/session/logout", { method: "POST" });
      window.location.reload();
    };
    sessionBar.appendChild(logoutBtn);

    renderHome(contentArea, did);
  } else {
    const anonSpan = document.createElement("span");
    anonSpan.textContent = "Not logged in";
    sessionBar.appendChild(anonSpan);
    renderLogin(contentArea);
  }
}

async function renderLogin(container: HTMLElement) {
  container.replaceChildren();

  const box = document.createElement("div");
  box.className = "login-picker";
  const title = document.createElement("h2");
  title.textContent = "Select Person Identity";
  box.appendChild(title);

  try {
    const res = await fetch("/_syneroym/session/identities");
    if (res.status === 200) {
      const data: { identities: string[] } = await res.json();
      if (!data.identities || data.identities.length === 0) {
        const p = document.createElement("p");
        p.textContent = "no person identities found in this node's person_identities_dir; create one with roymctl identity create";
        box.appendChild(p);
      } else {
        const select = document.createElement("select");
        select.style.width = "100%";
        select.style.marginBottom = "12px";
        select.style.padding = "6px";
        for (const id of data.identities) {
          const opt = document.createElement("option");
          opt.value = id;
          opt.textContent = id;
          select.appendChild(opt);
        }
        box.appendChild(select);

        const btn = document.createElement("button");
        btn.className = "button";
        btn.textContent = "Log In";
        btn.onclick = async () => {
          const chosen = select.value;
          const loginRes = await fetch("/_syneroym/session/login-local", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ identity: chosen }),
          });
          if (loginRes.ok) {
            window.location.reload();
          } else {
            alert(`Login failed: ${loginRes.statusText}`);
          }
        };
        box.appendChild(btn);
      }
    } else {
      const p = document.createElement("p");
      p.textContent = "Local login is not available on this node.";
      box.appendChild(p);
    }
  } catch (e) {
    const p = document.createElement("p");
    p.textContent = `Error fetching identities: ${e}`;
    box.appendChild(p);
  }

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
