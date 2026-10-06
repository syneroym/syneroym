# Ideas

Notes about things we might build one day. Nobody has committed to them.

An idea is **not** a requirement, a design or a plan. Do not implement it, cite
it from code or living docs, or treat it as a decision. It only keeps the
thinking, so we do not lose it.

## How to add one

Create `docs/ideas/<kebab-case-name>.md`. Start with this front matter:

```text
---
status: seed
---
```

`status` is one of:

| Status | Meaning |
| --- | --- |
| `seed` | A first thought. Little detail. |
| `exploring` | We are thinking about it, but have not decided to build it. |
| `parked` | Good idea, wrong time. Say what would make it the right time. |
| `rejected` | We decided not to do it. Keep the reason, so we do not repeat the debate. |
| `promoted` | It became a real change doc. Add `promoted-to: docs/planning/changes/<name>/change.md`. |

Write whatever helps: the problem, why it matters, rough approach, doubts,
links. No fixed sections.

## Moving up

- **To a change doc:** when we decide to work on it, copy the useful parts into
  a new change doc and set the idea to `promoted`.
- **To the backlog:** if we already built part of something and postponed the
  rest, that is a [deferred-backlog](../planning/deferred-backlog.md) row, not an idea.
