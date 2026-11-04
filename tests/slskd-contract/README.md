# slskd resume contract

lull seeks ahead in a Soulseek song by sizing slskd's incomplete file to X before it enqueues the download. This test proves the pinned slskd (`deploy/compose.yaml`, `app/build.rs`) still behaves the way that needs. ADR-0014 has the reasoning.

## What it proves

It runs the real image against a stand-in Soulseek server and a stand-in peer. Both are Node, speaking the protocol as Soulseek.NET 10.0.2 does. It checks two cases:

- **resume** (slskd's default settings): the incomplete file is pre-sized to 1 MiB of `0xAB` at the path slskd uses. The peer must be asked for offset 1 MiB. The finished file must be that pre-fill followed by the peer's bytes from 1 MiB on.
- **overwrite** (`transfers.download.retry.partial: overwrite`): with the same setup, the peer must be asked for offset 0, and the file must be the peer's whole file.

Any difference exits non-zero with one sentence saying what differed. slskd's log tail is printed with it.

## Files

| File | What it is |
|---|---|
| `run.mjs` | The runner: starts the stand-ins and slskd, enqueues the file through slskd's API, then checks the result |
| `server.mjs` | The stand-in server: login, the peer's address, and the two stats questions slskd asks after login |
| `peer.mjs` | The stand-in peer: queues the request, then opens the transfer and records the offset it is asked for |
| `protocol.mjs` | Message framing and the codes used |

## Run it locally

You need Docker and Node 22 or later. There are no packages to install.

```sh
node tests/slskd-contract/run.mjs
```

- **Linux, native Docker engine:** slskd runs with `--network host`, and everything talks over 127.0.0.1.
- **Docker Desktop (Windows, macOS):** slskd reaches this machine as `host.docker.internal`. Its API and listen port are published on 127.0.0.1.
- **Wrong guess:** if the mode is guessed wrong, set `CONTRACT_NETWORK=host` or `CONTRACT_NETWORK=desktop`.
- **Protocol trace:** `CONTRACT_TRACE=1` prints every message code the stand-ins receive.

CI runs this in `.github/workflows/slskd-contract.yml` whenever the pin, `crates/soulseek/` or this folder changes. It tests the image `deploy/compose.yaml` pins. Upgrading slskd means changing the tag and digest there and the desktop sidecar's version and hashes in `app/build.rs` together; this test must then pass.
