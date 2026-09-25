# Governance

How decisions get made, who makes them, and where they end up written down.
This is a policy document; the mechanics of actually landing a change are in
[CONTRIBUTING.md](CONTRIBUTING.md).

## Today: one maintainer

Panora has one maintainer, **ygkali** (see [AUTHORS.md](AUTHORS.md) and
[.github/CODEOWNERS](.github/CODEOWNERS)), who reviews and merges every
change and has final say on scope, design and releases. This is a plain
fact about the project's current size, not a permanent design: as
contributors show sustained, trusted involvement, they are added to
CODEOWNERS and given the same authority over the areas they own. There is
no separate "core team" application process beyond that — it follows from
contributing.

## Where decisions live

Three kinds of decision, three places, so nobody has to guess which one
records a given choice:

| Kind of decision | Recorded in | Example |
|---|---|---|
| Product scope and sequencing: what ships, in what order, why something was deliberately left out | [docs/ROADMAP.md](docs/ROADMAP.md), sections 6, 7 and 11 | Which release a feature belongs to; why EGO submission waited |
| Architecture: a choice that is expensive to reverse once code depends on it | [docs/adr/](docs/adr/) (Architecture Decision Records, Turkish) | Storage format, the IPC protocol, the crypto primitives (ADR 0001–0003) |
| One-off calls needing a yes/no the maintainer is the only one positioned to make | `docs/ROADMAP.md` section 7 ("Açık kararlar") | Application id, repository layout, default language |

A new ADR is warranted when a change would be costly to undo later:
swapping a storage engine, bumping the on-disk or wire protocol version,
adding a cryptographic primitive, or taking on a dependency that other
components come to rely on. Day-to-day implementation choices do not need
one; `CHANGELOG.md` and commit messages cover those.

## Code of Conduct and security reports

Conduct is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md); the
maintainer (or, later, any CODEOWNERS member) enforces it and is the
contact point for reports. Security reports follow
[SECURITY.md](SECURITY.md) instead of the normal issue tracker.

## Pull requests

One maintainer (or relevant CODEOWNERS reviewer) approval merges a change;
see [CONTRIBUTING.md](CONTRIBUTING.md) for what a mergeable pull request
looks like. Privacy and security implications are reviewed before code
style.

## If the maintainer goes quiet

Documented here because a single-maintainer bus factor is a real risk
([docs/ROADMAP.md](docs/ROADMAP.md) section 8 tracks it as one), not
because it is expected: if the maintainer is unreachable for an extended
period (no commits, reviews or replies for several months), a contributor
with a sustained history on the project should open a visible, pinned
issue proposing themselves or another regular contributor as the new
maintainer, wait a reasonable public comment period, and, once GitHub's own
inactive-owner support process (initiated through GitHub Support, since
there is no in-repository mechanism for it) confirms the transfer, take
over CODEOWNERS and repository access. There is no committee to convene
first — the point of writing this down now is that there does not need to
be one later.
