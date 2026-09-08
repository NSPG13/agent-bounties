# Local verification environment

This runbook provisions the facilities for the opt-in QICSWU continuous synthetic gate. The gate is local-only: it reads a reviewed Base snapshot, starts Anvil/Postgres/API/MCP/indexer/browser processes, runs a digest-pinned benchmark in a disposable Docker daemon, and never broadcasts to Base or uses live funds.

## Tested facility matrix

The locked candidate requires the following exact version matrix. Absolute paths may differ, but the direct report records and the composite validator checks every listed runtime version; a version divergence is a failed candidate rather than silently equivalent evidence.

| Facility | Tested version | Required contract |
|---|---|---|
| Python | 3.14.6 | `selenium==4.31.0` importable by the Python that launches the gate |
| Node.js | 24.19.0 | executable JavaScript test runner |
| Rust / Cargo | 1.88.0 | one toolchain directory containing `rustc`; Cargo must build with `--locked` |
| Foundry `forge` / `cast` / `anvil` | 1.8.1 | all three binaries from one release |
| Docker client/server | 29.7.2 | static client and daemon bundle, including containerd/shim/runc |
| containerd | 2.3.3 | supplied with the Docker static bundle |
| containerd shim | 2.3.3 | `containerd-shim-runc-v2` from the same static bundle |
| runc / docker-init | 1.4.3 / 0.19.0 | supplied with the same static bundle |
| Chromium / ChromeDriver | 150.0.7871.181 | exact major/minor pair; both executable |
| PostgreSQL server / client | 18.4 | `initdb`, `postgres`, and `psql` executable |
| util-linux `unshare` | 2.42.2 | unprivileged user and mount namespaces enabled |

Before the run, expose ordinary repository tools (`cargo`, `rustc`, `node`, `npm`, `forge`) on the invoking shell's command path for `scripts/preflight.sh` and `scripts/check.sh`. The QICSWU composite and direct runner additionally accept explicit paths through these environment variables:

| Variable | Facility |
|---|---|
| `QICSWU_CARGO_BINARY` | Cargo binary used by component and direct gates |
| `QICSWU_NODE_BINARY` | Node binary used by component gates |
| `QICSWU_RUST_BIN` | directory containing the matching `rustc` toolchain |
| `QICSWU_CARGO_HOME` | writable Cargo cache/home |
| `QICSWU_FORGE_BINARY`, `QICSWU_CAST_BINARY`, `QICSWU_ANVIL_BINARY` | Foundry binaries |
| `QICSWU_DOCKER_BINARY`, `QICSWU_DOCKERD_BINARY` | Docker client and daemon |
| `QICSWU_CONTAINERD_BINARY`, `QICSWU_CONTAINERD_SHIM_BINARY`, `QICSWU_RUNC_BINARY`, `QICSWU_DOCKER_INIT_BINARY` | remaining static Docker bundle |
| `QICSWU_UNSHARE_BINARY` | util-linux namespace launcher |
| `QICSWU_CHROMIUM_BINARY`, `QICSWU_CHROMEDRIVER_BINARY` | browser pair |
| `QICSWU_INITDB_BINARY`, `QICSWU_POSTGRES_BINARY`, `QICSWU_PSQL_BINARY` | PostgreSQL binaries |

The direct runner also exposes matching command-line flags in `python3 scripts/qicswu-continuous-golden-path.py --help`. Resolution order is explicit environment variable, ordinary executable lookup where supported, then the documented tested-host fallback. A missing facility returns `unavailable`; it never becomes a pass.

## Start the disposable Docker daemon

The pinned verifier image contains UID/GID ownership beyond the invoking account. A one-ID namespace can start Docker but cannot unpack that image faithfully. On Debian or Kali, an administrator must install `uidmap` (tested here with package `1:4.19.3-2`), and `/etc/subuid` plus `/etc/subgid` must each assign the invoking user a range of at least 65,536 IDs. Verify the resulting namespace before the release run:

```bash
sudo apt-get install uidmap
stat -c '%U:%G %a %n' /usr/bin/newuidmap /usr/bin/newgidmap
unshare --user --map-auto --map-root-user -- sh -c 'cat /proc/self/uid_map; cat /proc/self/gid_map'
```

The two maps must each cover inner IDs 0 through 65535. A typical valid result maps inner ID 0 to the invoking account and inner IDs 1–65535 to the first subordinate range. The checked-in launcher repeats this functional check and exits before creating its state directory if the helpers or ranges are unavailable. It resolves `newuidmap` and `newgidmap` from `PATH`; `QICSWU_NEWUIDMAP_BINARY` and `QICSWU_NEWGIDMAP_BINARY` can provide explicit paths.

Use a new, explicit `/tmp/qicswu-*` state directory. The launcher refuses to reuse existing state, disables bridge networking and iptables, uses the `vfs` driver, and runs the daemon in the verified multi-ID user/mount namespace. In terminal A:

```bash
export QICSWU_DOCKER_BIN_DIR=/absolute/path/to/docker-static-29.7.2
export QICSWU_DOCKER_STATE_DIR=/tmp/qicswu-release-gate-01
export QICSWU_NEWUIDMAP_BINARY=/usr/bin/newuidmap
export QICSWU_NEWGIDMAP_BINARY=/usr/bin/newgidmap
bash scripts/start-qicswu-isolated-docker.sh
```

Leave terminal A attached. A valid bundle directory contains `docker`, `dockerd`, `containerd`, `containerd-shim-runc-v2`, `runc`, and `docker-init`. The runner independently rejects a socket outside `/tmp`, a symlinked socket, a non-`vfs` daemon, or a Docker data root that is not beside that socket.

## Run the locked candidate

In terminal B, from the repository root, set absolute paths for the provisioned facilities. This example mirrors the tested host; replace paths only when the facility matrix review permits it:

```bash
export DOCKER_HOST=unix:///tmp/qicswu-release-gate-01/docker.sock
export QICSWU_DOCKER_BINARY=/absolute/path/to/docker-static-29.7.2/docker
export QICSWU_DOCKERD_BINARY=/absolute/path/to/docker-static-29.7.2/dockerd
export QICSWU_CONTAINERD_BINARY=/absolute/path/to/docker-static-29.7.2/containerd
export QICSWU_CONTAINERD_SHIM_BINARY=/absolute/path/to/docker-static-29.7.2/containerd-shim-runc-v2
export QICSWU_RUNC_BINARY=/absolute/path/to/docker-static-29.7.2/runc
export QICSWU_DOCKER_INIT_BINARY=/absolute/path/to/docker-static-29.7.2/docker-init
export QICSWU_UNSHARE_BINARY=/usr/bin/unshare
export QICSWU_RUST_BIN=/absolute/path/to/rust-1.88.0/bin
export QICSWU_CARGO_BINARY=/absolute/path/to/rust-1.88.0/bin/cargo
export QICSWU_CARGO_HOME=/tmp/qicswu-cargo-home
export QICSWU_NODE_BINARY=/absolute/path/to/node-24.19.0/bin/node
export QICSWU_FORGE_BINARY=/absolute/path/to/foundry-1.8.1/forge
export QICSWU_CAST_BINARY=/absolute/path/to/foundry-1.8.1/cast
export QICSWU_ANVIL_BINARY=/absolute/path/to/foundry-1.8.1/anvil
export QICSWU_CHROMIUM_BINARY=/usr/bin/chromium
export QICSWU_CHROMEDRIVER_BINARY=/usr/bin/chromedriver
export QICSWU_INITDB_BINARY=/usr/lib/postgresql/18/bin/initdb
export QICSWU_POSTGRES_BINARY=/usr/lib/postgresql/18/bin/postgres
export QICSWU_PSQL_BINARY=/usr/lib/postgresql/18/bin/psql
python3 scripts/qicswu-continuous-golden-path.py --check-source-integrity
DOCKER_HOST=unix:///tmp/qicswu-release-gate-01/docker.sock python3 scripts/check-qicswu-growth-golden-paths.py --run-continuous-local-fork --require-continuous --continuous-run-output target/qicswu-continuous-golden-path.json --output target/qicswu-growth-golden-paths.json
python3 scripts/qicswu-continuous-golden-path.py --check-source-integrity
```

The first integrity command must report `dirty: false`. The direct gate captures the exact commit, a canonical SHA-256 over tracked and untracked source files, every matrix facility's resolved path and version, active Docker server-component versions, artifact hashes, Docker isolation facts, pinned Base block hash, and canonical runtime code hashes. The validator requires the complete exact matrix. The gate rechecks the same source fingerprint after build and after the complete flow. The final command proves the later repository gates did not change source.

The run needs outbound read access to the configured Base RPC, the pinned GitHub source archive, and the digest-pinned Docker image. It permits no upstream RPC writes; all transactions, generated identities, balances, verifier code injection, and rows exist only in the disposable local environment. Exit `1` means a required assertion failed. Exit `2` means evidence could not be produced because a facility was unavailable.

Stop terminal A with Ctrl-C after the retained reports have been copied to ignored `target/` or external artifact storage. The explicitly named temporary state directory is disposable, but this runbook does not delete it automatically.
