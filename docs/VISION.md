# Background

The [thesis](../THESIS.md) states the core bet. We build a truly peer-to-peer foundation for group communication and trust. Independent mini-apps — chat, marketplace, social, AI — plug into this foundation. They work together as one experience. No central server sits in the middle. We use no blockchains and no cryptocurrency. This document builds on the thesis. It describes what we will actually pursue and build.

The rest of this section looks at what today's platforms get right and wrong. A credible alternative must keep their benefits and drop their drawbacks.

- Popular online consumer platforms like Swiggy, Urban Company, AirBnb, Uber, Upwork, Amazon, Upgrad, Practo:
    - have benefited consumers as well as providers big time
    - For example:
        - Gig workers and professionals (drivers, delivery partners, doctors, plumbers)
        - Small sellers (restaurants, grocery shops, real estate owners, movie theatres)
        - And their consumers

- Are run by large organizations that take charge of all aspects of the business

- Are evolving into ecosystems of multiple related parties driving a lot of value generation

- Above large Organizations and centrally-controlled systems bring benefits to service providers such as:
    - Overall technology enablement of businesses and efficiency
        - Without the need to build and manage sophisticated technology themselves
    - Massive discovery & distribution
    - Streamlining, standardization of interaction patterns
    - Institutional trust
    - Security at scale
    - Fault tolerance
    - Legal shielding
    - Reputation aggregation
    - Economies of scale
    - Network effects

- And have the following drawbacks:
    - Non-availability (geographies, power/network/technology constrained scenarios)
    - Vendor lock-in
    - Governance asymmetry less freedom (but less decision making hassle) to participants
    - Flexibility to customize for localized scenarios
    - Data ownership loss
    - Sudden policy risk leading to unhappy participants
    - No transparency of how the internal systems/algorithms work
    - Strategic dependency
    - Not friendly to buildup of deep provider-client relationships, mostly transactional

# Our objective
Realize the thesis in a way that keeps the benefits of centrally controlled systems, without their drawbacks. Our approach is `Autonomous Cooperating Mini-Apps over a common technology substrate`. We will build:

- A technology substrate: the truly peer-to-peer foundation. It gives useful technology primitives to mini-apps (SynApps) running on top of it. These primitives are identity, contacts, groups, trust, and discovery. They help ecosystem participants build value generation ecosystems.

- **Roym** — our flagship combined experience. Mini-apps share one identity, one contact list, one set of groups, and one trust model. We will start with two business verticals inside it.
    1. Professional Services Guild (Home services first)
    - E.g. Local equivalent of Urban Company or TaskRabbit
    - This will be built as a deep, real vertical
    2. Local Producer-Distributor Mesh (Food + small retail)
    - E.g. Local equivalent of Swiggy or Amazon
    - This will be built as a thinner, vertical for demonstration purposes

# Common Mini-app characteristics
Mini-apps have the following characteristics
- Independent providers or provider groups can build, buy, host, and manage mini-apps themselves. The cost is low enough. They need only limited technical expertise.
- They usually operate at smaller scales. They manage a smaller group of providers, sometimes a single provider.
- They can run on low-end hardware (PCs, RPI, mobiles). Such hardware often has power and network limits.
- They can scale out by federating lower-end hardware units. Other ecosystem participants can provide these units from spare infrastructure that has the needed capabilities.
- They provide benefits of large platforms as described above, but also avoid drawbacks discussed
- They work autonomously within the group owning it
- They also help providers using different mini-apps to coordinate and cooperate via rich primitives in the underlying technology substrate

# Rationale behind selecting our initial mini-apps
We selected the 2 mini-apps mentioned above, namely, Services Guild, and Producer-Distributor mesh, due to their following inherent characteristics:
- They have a strong chance of being viable alternatives to platforms like those listed above that providers use currently for various reasons such as:
    - Fragmented supply side (many small providers)
    - Local density effects (in local settings, trust and word of mouth can outperform algorithmic ranking)
    - Strong dissatisfaction with existing platforms
    - Manageable trust surface (not life-critical at first)
    - Low regulatory friction
    - Relatively easy to pilot, no heavy logistics

- They have characteristics useful to demonstrate the power, flexibility and long-term potential of such autonomous-cooperative alternatives, get people excited about them. E.g.
    - High transaction volumes
    - Clear cross-mini-app federation potential
    - Can run on low hardware, individual or federated

- Can naturally share a lot of common substrate primitives such as:
    - Identity
    - Discovery
    - Capability advertisement
    - Pricing, Negotiation protocols
    - Payment abstraction
    - Reputation portability
    - Governance voting

