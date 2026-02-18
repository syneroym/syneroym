# Background
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
We feel there is room for alternate systems that bring the best of both worlds, which many providers and customers would appreciate. Systems which maintain the above-mentioned benefits of centrally controlled systems, minus their drawbacks.

We will take a shot at building one such an alternative with `Autonomous Cooperating Mini-Apps over a common technology substrate`. Specifically we will build:

- A technology substrate that catalyzes building similar value generation ecosystems on top by providing needed underlying technology ingradients via of Mini-apps running on top of it.

- Mini-apps for selected business verticals that act as fundamental blocks of the new ecosystem. We will start by building the following two.
    1. Federated Professional Services Guild (Home services first)
    - E.g. Local equivalent of Urban Company or TaskRabbit
    - This will be built as a deep, real vertical
    2. Local Producer-Distributor Mesh (Food + small retail)
    - E.g. Local equivalent of Swiggy or Amazon
    - This will be built as a thinner, vertical for demonstration purposes

# Common Mini-app characteristics
Mini-apps have the following characteristics
- Independent Providers / provider groups can build/buy, host, manage mini-apps themselves at sufficiently low cost, with limited technical expertise
- Typically they operate at smaller scales, managing a smaller group of providers, even a single provider
- They can work on low end hardware infrastructure (PCs, RPI, Mobiles), often with power and network connectivity constraints
- They can scale out by federating lower-end hardware units. These hardware units can be made available by other ecosystem participants having spare infrastructure with needed capabilities
- They provide benefits of large platforms as described above, but also avoid drawbacks discussed
- They work autonomously within the group owning it
- They also help providers using different mini-apps to coordinate and cooperate via rich collaboration and coordination primitives in the underlying technology substrate

# Rationale behind selecting our initial mini-apps
We selected the 2 mini-apps mentioned above, namely, Services Guild, and Producer-Distributor mesh, due to their following inherent characteristics:
- They have a strong chance of being viable alternatives to platforms like those listed above that providers use currently for various reasons such as:
    - Fragmented supply side (many small providers)
    - Local density effects (in local settings, trust and word of mouth can beat algorithmic ranking)
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

