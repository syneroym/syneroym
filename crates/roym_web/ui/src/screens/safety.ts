import { call } from "../rpc";

export async function renderSafety(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "safety-screen";

  const h2 = document.createElement("h2");
  h2.textContent = "Safety & Blocklist";
  box.appendChild(h2);

  const blockDesc = document.createElement("p");
  box.appendChild(blockDesc);

  try {
    const policy = await call<{ statement?: string }>("profile.policy");
    if (policy?.statement) {
      blockDesc.textContent = policy.statement;
    } else {
      blockDesc.textContent = "Could not load this node's safety policy.";
    }
  } catch {
    blockDesc.textContent = "Could not load this node's safety policy.";
  }

  const blockInput = document.createElement("input");
  blockInput.placeholder = "Person DID or Address to block";
  box.appendChild(blockInput);

  const blockBtn = document.createElement("button");
  blockBtn.className = "button";
  blockBtn.textContent = "Block";

  const blockError = document.createElement("p");
  blockError.className = "block-error";
  box.appendChild(blockBtn);
  box.appendChild(blockError);

  blockBtn.onclick = async () => {
    blockError.textContent = "";
    const val = blockInput.value.trim();
    if (!val) return;
    try {
      if (val.startsWith("did:")) {
        await call("block.add", { person_did: val });
      } else {
        await call("block.add", { address: val });
      }
      blockInput.value = "";
      await loadBlocks();
    } catch (err) {
      blockError.textContent = `Block failed: ${err instanceof Error ? err.message : String(err)}`;
    }
  };

  const blockList = document.createElement("div");
  box.appendChild(blockList);

  const loadBlocks = async () => {
    blockList.replaceChildren();
    try {
      const rows = await call<Array<{ person_did?: string; address?: string }>>("block.list");
      for (const r of rows) {
        const item = document.createElement("div");
        item.textContent = `Blocked: ${r.person_did || r.address || ""}`;
        blockList.appendChild(item);
      }
    } catch {
      blockList.textContent = "Could not load block list.";
    }
  };
  await loadBlocks();

  container.appendChild(box);
}
