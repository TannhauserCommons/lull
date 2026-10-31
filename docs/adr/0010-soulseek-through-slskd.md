# ADR-0010: Soulseek through an slskd sidecar, not a native Rust client

- Status: accepted
- Date: 2026-10-31

## Context

lull must find music on Soulseek, start playing within seconds while the transfer runs, fall back to another peer when one stalls, resume rather than refetch, stay polite to the network and share the listener's library. There are two ways to speak Soulseek from a Rust server:

- **slskd** (0.26.2, October 2026), a maintained daemon built on Soulseek.NET, with a REST API, run as a sidecar container.
- **A native client in Rust**, implementing the peer protocol ourselves. The existing Rust crates are small and unmaintained.

The brief's criteria, and how each option does (slskd checked against its 0.26.2 source; research in the vault, `projects/lull/research/slskd-progressive-streaming.md`):

| Criterion | slskd sidecar | Native Rust client |
|---|---|---|
| Reliability | Years of use; handles server reconnects, NAT, distributed search, peer quirks | Ours to find and fix, one peer bug at a time |
| Maintenance | Upstream follows protocol and server changes | A second product to keep alive |
| Progressive streaming | Incomplete files are written front to back, never pre-sized, into a predictable path; lull reads them as they grow | Full control: bytes straight from the socket |
| Resume and offsets | Since 0.26.0 a re-queued file resumes from its partial file | Ours to build |
| Rate limits | Two searches at a time, queued beyond that; retries with backoff; upload slots and speed limits | Ours to build and tune |
| Sharing | Mature share scanning, upload queues and limits | Ours to build |
| Search streaming | Answers are revealed only when a search ends; only counters are live | Answers as they arrive |

## Decision

lull talks to Soulseek only through slskd's REST API (`crates/soulseek/src/slskd.rs`), with slskd as a sidecar sharing one transfers volume with lull.

- **Streaming:** lull reads slskd's growing incomplete file directly. Each segment of the path is sanitised the way slskd does it; the finished path comes from slskd's `DownloadFileComplete` event.
- **Search:** each search is capped at 15 s, with 6 s of no answers ending it early. lull publishes the live counters and then the merged results, the moment slskd has them.
- **Sharing:** the deploy kit configures it at slskd's start. lull checks it at run time and rescans after saves.

## Consequences

- **Results arrive in one batch.** Search results land after up to about 15 s rather than trickling in; the counters move meanwhile. If slskd starts publishing responses as they arrive, lull can stream them without changing its API.
- **Deploys need both containers.** Two containers and one shared volume are now part of every deploy; the kit hides that behind one compose file (ADR-0011).
- **lull follows slskd's file layout.** It tracks the incomplete-file layout of slskd 0.26; an slskd upgrade must be checked against `layout.rs`. The image is pinned in `deploy/compose.yaml`.
- **Sharing can't change at run time.** slskd refuses share changes from API keys, so `LULL_SHARE` takes effect when slskd restarts.
- **The upstream protocol work is not ours.** Rewriting the protocol natively stays possible behind the same crate boundary if slskd ever blocks a feature.
