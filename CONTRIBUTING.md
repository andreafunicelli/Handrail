# Contributing

Thanks for helping with Handrail. Security problems go through private reporting, not issues:
see [SECURITY.md](SECURITY.md).

## Prerequisites

- Node.js 22 and npm
- Rust through [rustup](https://rustup.rs): `rust-toolchain.toml` pins the version, and rustup
  installs it (with rustfmt and clippy) the first time you run `cargo` in the repository
- The [Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform
  (WebView2 and the C++ build tools on Windows)

```bash
npm ci
node scripts/build-rdp-helper.mjs
npm run tauri -- dev
```

## Checks

CI runs these on every pull request; run them before opening one:

```bash
npm run check
npm run test
npm run build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo fmt --manifest-path rdp-helper/Cargo.toml -- --check
cargo clippy --manifest-path rdp-helper/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path rdp-helper/Cargo.toml --locked
```

`npm run build` creates `ui/dist`, which the workspace tests need; the RDP helper is its own Cargo
workspace. CI also runs `cargo deny check` for both workspaces.

### Integration tests

Some tests talk to real servers and **skip themselves** when their variable is unset (CI's Linux
job starts the servers and fails if any variable is missing):

| Test | Variables |
| --- | --- |
| `crates/aht-ssh/tests/real_sftp.rs` | `HANDRAIL_SSH_TEST=user@host:port`, `HANDRAIL_SSH_TEST_KEY=/path/to/private/key` |
| `crates/aht-s3/tests/real_s3.rs` | `HANDRAIL_S3_TEST=http://127.0.0.1:9000`, `HANDRAIL_S3_BUCKET` (must exist), `HANDRAIL_S3_KEY`, `HANDRAIL_S3_SECRET` |
| `crates/aht-db/tests/real_db.rs` | `HANDRAIL_MYSQL_TEST` and/or `HANDRAIL_PG_TEST`, each `user:password@host:port/database` |

The "Rust" job in `.github/workflows/ci.yml` shows how CI starts each server.

## Branches and releases

- Open pull requests against `main`. Every merge to `main` publishes a stable release
  (`v<major>.<minor>.<n>`), after CI passes on that commit.
- Pushes to the `nightly` branch publish a *Handrail Nightly* prerelease (`nightly-<version>`),
  also only after CI passes.
- When a pull request adds or changes an IPC command, update `docs/ipc-contract.md`, and keep the
  English and Italian strings (`ui/src/lib/i18n/en.ts`, `it.ts`) in step.

## Bumping the Rust toolchain

The toolchain is pinned so that a new Rust release (and its new clippy lints) cannot break `main`.
To move to a newer one:

1. Change `channel` in `rust-toolchain.toml` (it must stay at or above the workspace
   `rust-version` in `Cargo.toml`).
2. Change every `toolchain:` passed to `dtolnay/rust-toolchain` in `.github/workflows/ci.yml` and
   `release.yml` to the same version (`grep -rn "toolchain:" .github/workflows`).
3. Run the checks above, fix any new clippy warnings in the same pull request.
