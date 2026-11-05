# slskd contract

lull seeks ahead in a Soulseek song by sizing slskd's incomplete file to X before it enqueues the download, and streams a song by reading that file while slskd writes it. This test proves the pinned slskd (`deploy/compose.yaml`, `app/build.rs`) still behaves the way both need. ADR-0014 and ADR-0062 have the reasoning.

## What it proves

It runs the real slskd against a stand-in Soulseek server and a stand-in peer. Both are Node, speaking the protocol as Soulseek.NET 10.0.2 does. It checks three cases:

- **resume** (slskd's default settings): the incomplete file is pre-sized to 1 MiB of `0xAB` at the path slskd uses. The peer must be asked for offset 1 MiB. The finished file must be that pre-fill followed by the peer's bytes from 1 MiB on.
- **overwrite** (`transfers.download.retry.partial: overwrite`): with the same setup, the peer must be asked for offset 0, and the file must be the peer's whole file.
- **stream** (the default settings, nothing pre-sized): the peer sends slowly. While slskd downloads, the incomplete file must open for reading the way lull opens it (Node's open, like Rust's, shares read, write and delete), and its first bytes must be the peer's. The file stays open while slskd finishes and moves it to its downloads folder, then must read to the end as the peer's file. Stock slskd on Windows fails this with a sharing violation (lull's "os error 32"), which is why the Windows app bundles lull's build of it (`app/slskd/`).

Any difference exits non-zero with one sentence saying what differed. slskd's log tail is printed with it.

## Files

| File | What it is |
|---|---|
| `run.mjs` | The runner: starts the stand-ins and slskd, enqueues the file through slskd's API, then checks the result |
| `server.mjs` | The stand-in server: login, the peer's address, and the two stats questions slskd asks after login |
| `peer.mjs` | The stand-in peer: queues the request, then opens the transfer and records the offset it is asked for |
| `protocol.mjs` | Message framing and the codes used |

## Run it locally

You need Node 22 or later. There are no packages to install.

```sh
node tests/slskd-contract/run.mjs
```

That runs the image `deploy/compose.yaml` pins, in Docker.

- **Linux, native Docker engine:** slskd runs with `--network host`, and everything talks over 127.0.0.1.
- **Docker Desktop (Windows, macOS):** slskd reaches this machine as `host.docker.internal`. Its API and listen port are published on 127.0.0.1.
- **Wrong guess:** if the mode is guessed wrong, set `CONTRACT_NETWORK=host` or `CONTRACT_NETWORK=desktop`.
- **Protocol trace:** `CONTRACT_TRACE=1` prints every message code the stand-ins receive.

```sh
node tests/slskd-contract/run.mjs --native
```

That runs the binary `app/build.rs` pins for this machine, the way the desktop app runs it. It works on Windows, where it matters, and is downloaded and checked by its sha256 first.

CI runs this in `.github/workflows/slskd-contract.yml` whenever the pins, lull's Windows build (`app/slskd/`), `crates/soulseek/` or this folder changes: in Docker on Linux, and `--native` on Windows. The same workflow rebuilds lull's Windows slskd and checks it is the pinned bytes. Upgrading slskd means changing the tag and digest in `deploy/compose.yaml`, the desktop sidecar's URLs and hashes in `app/build.rs`, and lull's Windows build (ADR-0062) together; this test must then pass.
