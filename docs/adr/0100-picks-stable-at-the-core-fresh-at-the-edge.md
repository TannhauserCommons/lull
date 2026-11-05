# ADR-0100: Picks are stable at the core and fresh at the edge

- Status: accepted
- Date: 2026-11-05

## Context

Picks must not swing when a listener plays one new song or dips into an unusual genre, and they must not go stale either. Before this, every build took its seeds from the strongest 4 artists and tags of a profile in which each play counted. A day of bingeing one new artist could make it the top seed. The same 4 seeds came back every build, and "Refresh picks" only showed the stored feed again.

What the evidence says comes from randomised and controlled studies: Spotify's A/B tests, field and lab experiments, and reviews. The sources are listed below, with correlational findings marked. The evidence gives directions. It does not give thresholds for a single self-hosted listener, so the thresholds below are engineering choices, checked by simulations (`picking/sim.rs`).

## Decision

**Damping, in the taste model:**
- At most 3 heard plays and 2 skips per artist per day count, so a day counts, not each play.
- An artist is **established**, and can seed picks or count for a tag, only once it has been heard on 2 separate days, or the listener acted on it: a like, a save, a follow, "more like this", albums in the library, or its Last.fm history.
- An artist is held back for skips only after skips on 2 or more days, or after "less like this".

**Seeds:**
- The 3 strongest established artists and the 2 strongest tags (followed ones first) always seed picks. This is the core.
- One more artist and up to two more tags are drawn by weight from the next ones, up to the 12th. This is the fresh edge.
- The edge is new artists close to the listener's own taste. Novelty is measured as distance from that taste (lower ranks of the listener's own established artists and tags), never as popularity. Obscure artists are welcome; tag picks lean to the less popular half on purpose.

**Mix:**
- About a fifth of the shelf comes from the edge: 3 of 12 for a listener of few tags, 2 for a broad one.
- One song per artist, originals first, nothing the listener knows, hid or holds back, and no artist they already like.

**Turnover:**
- An automatic rebuild (6-hourly, or after a follow or feedback) keeps up to half the last shelf's picks not taken yet.
- A pick gives way once it has been on 3 shelves, and no pick from the last 3 shelves comes back as a new one.
- Refresh, the listener's own, builds a whole new shelf, at most once an hour.

**Learning from picks:**
- Each pick's seed is remembered.
- Per pick, after it was shown: heard +0.5, heard again on a later day +2, liked or saved +2, skipped −0.5. These fade with taste's 45-day half-life.
- A seed's total is shrunk by n/(n+3), for its n picks with an outcome.
- Its weight moves by a factor of 2^(score/3), between half and twice.
- Outcomes count for 180 days, and only since the last taste reset.

## Evidence

| Choice | Evidence |
|---|---|
| About a fifth of the shelf exploratory | Mehrotra 2021 (Spotify, four randomised tests, up to 28M users): discovery around 10% cost no satisfaction, and the winning balancer leaned toward about 20%. That share figure is an association within each test arm, not randomised. Ziegler et al. 2005 (2,125 users): swapping about 2–3 of 10 items captured most of the gain from diversity |
| Novelty near the listener's taste, not a foreign experience | Ekstrand et al. 2014 (582 users): perceived diversity raised satisfaction, perceived novelty lowered it. Willemsen et al. 2016: diversifying within taste raised attractiveness and eased choice |
| More edge for narrow listeners | Sun et al. 2023 (randomised field experiment, 1,859 users): a broad shelf helped narrow-taste users and slightly hurt already-broad ones |
| Damp single events, count returns | Maystre et al. 2023 (Spotify A/B): optimising for 60-day return listening beat optimising for immediate plays by 81% in listening time. McDonald et al. 2023 / Zhang et al. 2025 (Spotify A/B): early return behaviour predicted long-term taste. PinnerFormer 2022 (Pinterest A/B): modelling a multi-day window beat modelling the next action |
| Judge exploration over weeks | Su et al. 2024 (Google, long-run experiments); Hohnhold et al. 2015: short-term readouts mispredict long-term effects |
| Pure relevance narrows listeners | Holtz et al. 2020 (randomised, 852,937 Spotify users): personalisation raised streams 28.9% and cut each user's diversity 11.5% |

Silent in the evidence, so engineering choices:
- how many repeats before adapting (2 days);
- how much a rebuild changes (half);
- how often a pick not taken is shown (3 shelves).

The full notes are kept with the work that led here.

## Measured (simulated listeners, `cargo test -p lull-discovery sim -- --nocapture`)

| Listener | Event | Now | Before (main's rules) |
|---|---|---|---|
| Steady, one genre | Next automatic build | 50% of picks change; seed shift 0.000; 3 edge picks | Seeds the same every build |
| Wide, eclectic | 7 days of builds | 8 seed artists over the week; 50% turnover | 4 seed artists every build |
| Binges a new artist | One day of 40 plays | Seed shift 0.000; the artist doesn't seed | Seed shift 0.200 |
| Binges a new artist | 3 such days | It starts to seed (about 1% of draws) | n/a |
| One stray play, unusual genre | Next build | Seed shift 0.000 | 0.000 |
| New listener | Nothing / a library | Starter tags / library artists seed | Starter tags / starter tags |
| Back after 4 months | First day back | Seed shift 0.000; the library and follows still seed | n/a |
| Back after 4 months | A week | The new listening leads | n/a |
| Heavy skipper (60% skips) | A month | 12 picks; held back only after skips on 2+ days | n/a |

The seed shift is the total-variation distance between seed distributions: 0 means no change, 1 means entirely different seeds.

## Rejected

- **Popularity as novelty:** the owner's rule is that obscure is fine and an unfamiliar experience is not.
- **Fully random seeds every build:** they churn without a core.
- **A time-based cool-down on new artists:** it guesses at intent. Returning on separate days is the signal.
