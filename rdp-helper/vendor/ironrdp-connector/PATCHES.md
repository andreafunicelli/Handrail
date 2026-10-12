# Handrail patches to ironrdp-connector 0.10.0

Source: IronRDP commit `11a0810c` (`chore(release): prepare for publishing (#1364)`),
`crates/ironrdp-connector`, with the workspace-inherited fields of `Cargo.toml` written out.

- `set_console_session` (lib.rs) and the client cluster data in `create_gcc_blocks`
  (connection.rs): ask for the console session, as `mstsc /admin` does. Upstream 0.10 always
  sends no cluster data (TODO #139); later versions make it configurable. Servers whose RDS
  licensing breaks normal logons still let administrators in this way.
- Licensing ignores other channels (connection.rs, `LicensingExchange`): once licensing has
  started, upstream decodes every PDU as a licensing PDU, including the message channel's
  auto-detect and heartbeat PDUs that Windows Server 2019 sends meanwhile. That failed with
  "invalid `securityHeaderFlags`". PDUs on other channels are now answered (auto-detect RTT)
  or skipped, and an I/O-channel PDU without `SEC_LICENSE_PKT` is reported with its flags and
  first bytes.
- Server redirection is recognised (handrail.rs, used by both licensing entry points in
  connection.rs): a Remote Desktop Connection Broker answers the Client Info PDU with a Server
  Redirection PDU where the first licensing PDU would be. IronRDP 0.10 can't follow it and
  decoded it as a licensing PDU. It is now parsed (Enhanced and Standard Security forms) and
  reported as an error naming the target host. Any other non-licensing PDU on the I/O channel
  is reported with its first word and bytes. `handrail::parse_redirection` is public so
  `rdp-helper/tests/redirection.rs` can test it.
- `#[expect(single_use_lifetimes)]` → `#[allow(...)]` (connection.rs): the lint is only enabled
  by IronRDP's workspace lint settings, so the expectation would go unfulfilled here.

Drop this directory and the `[patch.crates-io]` entry once an IronRDP release on crates.io
exposes the cluster data in its config.
