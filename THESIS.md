# Syneroym — Thesis

## One sentence

Syneroym is a truly peer-to-peer foundation for group communication and trust, on which independent mini-apps — chat, marketplace, social, AI — plug in and work together as one experience, with no central server in the middle.

## The hard problem we are solving

Today's software mostly follows three models:

- **Platform model** (e.g. WhatsApp, Amazon): one organization runs the servers and the rules. This gives a polished, integrated experience, but on that organization's terms — where it operates, what its policies allow, and how it handles your data.
- **Federated model** (e.g. Matrix, Mastodon): many independent servers cooperate. No single owner, but a server still sits between you and the people you interact with.
- **P2P model**: no server in the middle at all. Your device — or a machine you choose and control — talks directly to other people's devices.

Syneroym explores this third model, and without relying on blockchains or cryptocurrency. Community members run small bootstrap nodes that help peers find each other, and relay — end-to-end encrypted — the small fraction of connections that cannot be made directly; they store nothing and own nothing.

Direct device-to-device connection by itself is well understood. The genuinely open problem is:

> **Rich group activity — messaging, buying and selling, sharing, coordinating — among many people, groups and apps, at real scale, where no participant's device needs to fully trust any other.**

This is the gap Syneroym goes after. Everything else in the project exists to prove it.

## What sits on top

Once the foundation works, apps plug into it like apps on a phone — except they can choose to share one identity, one contact list, one set of groups, one trust model, all provided as platform primitives. So:

- A chat with your plumber can carry a quote you approve with one tap.
- A neighborhood group can double as a local marketplace.
- An AI assistant can help across all of it, with only the access you grant it.

This combined everyday experience is what people actually touch. The foundation underneath is what makes it possible without any single operator owning it all.

## What this is not

- **Not a replacement for existing platforms.** It is an alternative for people who want one — chosen, not imposed. If it attracts nobody, it fails; it does not win by claiming to.
- **Not decentralization for its own sake.** Managed hosting and easy onboarding are fine. The one rule: you can always leave, taking your data, history, and reputation with you.
- **Not built in one go.** It ships in phases. A small, real, working slice comes first; nothing is promised before it is proven.

## How it relates to prior work

Several projects have advanced parts of this vision, and Syneroym builds on their lessons:

| Project | Contribution | What Syneroym adds |
|---|---|---|
| Solid | Personal data ownership | A trust, discovery, and app-composition layer around the data |
| Holochain | Agent-centric P2P computing | An integrated, everyday consumer experience on top of a P2P core |
| Matrix | Open, large-scale group messaging | A fully P2P transport rather than server federation |
| WeChat-style super-apps | Proof that one integrated experience is valuable | The same integration without a single central operator |

The combination — an integrated everyday experience on a truly P2P, trust-aware foundation — is the specific ground none of these occupy yet.

## Why builders would care

The foundation is generic, like Kubernetes is generic. Anyone can build their own mini-app (SynApp) on it. Our flagship experience is the proof it works — not the only thing it is for.
