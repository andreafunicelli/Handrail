//! Handrail additions to the connector (see PATCHES.md).
//!
//! A Remote Desktop Connection Broker answers the Client Info PDU with a Server Redirection PDU
//! where the first licensing PDU would be, telling the client to reconnect to the host that
//! holds (or will hold) the session. IronRDP 0.10 has no redirection support and tried to decode
//! it as a licensing PDU ("invalid security header flags"). Recognise it and say where the
//! server wants the client to go.

use ironrdp_pdu::rdp::headers::{ShareControlHeader, ShareControlPdu, ShareDataPdu};
use ironrdp_pdu::rdp::server_error_info::{ErrorInfo, ServerSetErrorInfoPdu};

use crate::{ConnectorResult, reason_err};

const SEC_LICENSE_PKT: u16 = 0x0080;
const SEC_REDIRECTION_PKT: u16 = 0x0400;
const PDUTYPE_SERVER_REDIR_PKT: u16 = 0x000A;

const LB_TARGET_NET_ADDRESS: u32 = 0x0001;
const LB_LOAD_BALANCE_INFO: u32 = 0x0002;
const LB_USERNAME: u32 = 0x0004;
const LB_DOMAIN: u32 = 0x0008;
const LB_PASSWORD: u32 = 0x0010;
/// Informational only: the client stays where it is ([MS-RDPBCGR] 2.2.13.1).
pub const LB_NOREDIRECT: u32 = 0x0080;
const LB_TARGET_FQDN: u32 = 0x0100;
const LB_TARGET_NETBIOS_NAME: u32 = 0x0200;

/// The parts of an `RDP_SERVER_REDIRECTION_PACKET` ([MS-RDPBCGR] 2.2.13.1) worth reporting.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Redirection {
    pub session_id: u32,
    pub redir_flags: u32,
    pub target_net_address: Option<String>,
    pub load_balance_info: Option<Vec<u8>>,
    pub username: Option<String>,
    pub domain: Option<String>,
    pub target_fqdn: Option<String>,
    pub target_netbios_name: Option<String>,
}

impl Redirection {
    /// Where the server sends the client: FQDN, else NetBIOS name, else address.
    pub fn target(&self) -> Option<&str> {
        [&self.target_fqdn, &self.target_netbios_name, &self.target_net_address]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .find(|s| !s.is_empty())
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let out = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(out)
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    /// A `u32` byte length followed by that many bytes.
    fn blob(&mut self) -> Option<&'a [u8]> {
        let len = usize::try_from(self.u32()?).ok()?;
        self.take(len)
    }
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned()
}

/// Parse a Server Redirection PDU from the user data of an MCS Send Data Indication: either the
/// Enhanced Security form (TLS/NLA: share control header of type `PDUTYPE_SERVER_REDIR_PKT`,
/// two pad bytes, the packet) or the Standard Security one (basic security header with
/// `SEC_REDIRECTION_PKT`, the packet).
pub fn parse_redirection(user_data: &[u8]) -> Option<Redirection> {
    let mut r = Reader { buf: user_data, pos: 0 };
    let first = r.u16()?;
    let second = r.u16()?;
    if second & 0x000F == PDUTYPE_SERVER_REDIR_PKT {
        // Enhanced: totalLength, pduType, then pduSource and pad2Octets.
        r.take(4)?;
    } else if first & SEC_REDIRECTION_PKT == 0 {
        return None;
    }
    // Standard: flags, flagsHi were the two u16s read.
    if r.u16()? != SEC_REDIRECTION_PKT {
        return None;
    }
    let _length = r.u16()?;
    let mut out = Redirection {
        session_id: r.u32()?,
        redir_flags: r.u32()?,
        ..Redirection::default()
    };
    let f = out.redir_flags;
    if f & LB_TARGET_NET_ADDRESS != 0 {
        out.target_net_address = Some(utf16(r.blob()?));
    }
    if f & LB_LOAD_BALANCE_INFO != 0 {
        out.load_balance_info = Some(r.blob()?.to_vec());
    }
    if f & LB_USERNAME != 0 {
        out.username = Some(utf16(r.blob()?));
    }
    if f & LB_DOMAIN != 0 {
        out.domain = Some(utf16(r.blob()?));
    }
    if f & LB_PASSWORD != 0 {
        // An opaque cookie for the target; skipped, never reported.
        r.blob()?;
    }
    // Truncated optional fields after this point still leave a useful result.
    if f & LB_TARGET_FQDN != 0 {
        out.target_fqdn = r.blob().map(utf16);
    }
    if f & LB_TARGET_NETBIOS_NAME != 0 && (f & LB_TARGET_FQDN == 0 || out.target_fqdn.is_some()) {
        out.target_netbios_name = r.blob().map(utf16);
    }
    Some(out)
}

/// What to do with a PDU that arrives where licensing expects one.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LicensingInput {
    /// Hand it to the licensing sequence.
    Licensing,
    /// An informational redirection (`LB_NOREDIRECT`): consume it and keep waiting.
    Skip,
}

/// Before a PDU is handed to the licensing sequence: when it is on the I/O channel but is not a
/// licensing PDU, fail with what it is (a redirection and where to) instead of a decode error.
pub(crate) fn check_licensing_pdu(input: &[u8], io_channel_id: u16, state: &str) -> ConnectorResult<LicensingInput> {
    let Ok(data) = ironrdp_pdu::mcs::decode_send_data_indication(input) else {
        return Ok(LicensingInput::Licensing);
    };
    let user_data = data.user_data;
    if data.channel_id != io_channel_id || user_data.len() < 2 {
        return Ok(LicensingInput::Licensing);
    }
    // Redirection first: in the enhanced form the first word is a length, which may happen to
    // have the licensing bit set.
    if let Some(redirection) = parse_redirection(user_data) {
        if redirection.redir_flags & LB_NOREDIRECT != 0 {
            tracing::debug!(
                ?redirection,
                "Informational server redirection (LB_NOREDIRECT), staying"
            );
            return Ok(LicensingInput::Skip);
        }
        let target = redirection.target().unwrap_or("another host (no name given)");
        let mut details = format!("session {}", redirection.session_id);
        if let Some(lb) = redirection.load_balance_info.as_deref().filter(|lb| !lb.is_empty()) {
            let lb = String::from_utf8_lossy(lb);
            details.push_str(&format!(
                ", load-balance info \"{}\"",
                lb.trim_end_matches(['\r', '\n', '\0'])
            ));
        }
        return Err(reason_err!(
            "LicensingExchange",
            "the server redirected the connection to {target} ({details}). This is a Remote Desktop \
             Connection Broker or load-balanced farm, and Handrail cannot follow redirections yet: \
             connect to {target} directly"
        ));
    }
    // A server that refuses the session says why in a Set Error Info PDU (slow-path data PDU,
    // no security header) and disconnects. Licensing failures arrive here, at the first
    // licensing step: no license server, no CAL, ...
    if let Some(info) = set_error_info(user_data) {
        let code = user_data[user_data.len() - 4..]
            .try_into()
            .map(u32::from_le_bytes)
            .unwrap_or_default();
        let hint = if matches!(info, ErrorInfo::ProtocolIndependentLicensingCode(_)) {
            ". The server's Remote Desktop licensing is not working, so it refuses normal sessions \
             (other clients such as mstsc fail the same way). An administrator can fix the RD \
             licensing configuration on the server, or connect with \"Administrative session \
             (/admin)\", which needs no license"
        } else {
            ""
        };
        return Err(reason_err!(
            "LicensingExchange",
            "the server refused the session: {} (error info {code:#010x}){hint}",
            info.description(),
        ));
    }
    let flags = u16::from_le_bytes([user_data[0], user_data[1]]);
    if flags & SEC_LICENSE_PKT != 0 {
        return Ok(LicensingInput::Licensing);
    }
    let head = &user_data[..user_data.len().min(32)];
    Err(reason_err!(
        "LicensingExchange",
        "the server sent a non-licensing PDU on the I/O channel during licensing (state {state}, \
         first word {flags:#06x}, {} bytes, starting {head:02x?})",
        user_data.len(),
    ))
}

/// The error of a Set Error Info PDU ([MS-RDPBCGR] 2.2.5.1.1), if that is what `user_data` is.
pub fn set_error_info(user_data: &[u8]) -> Option<ErrorInfo> {
    let header = ironrdp_core::decode::<ShareControlHeader>(user_data).ok()?;
    match header.share_control_pdu {
        ShareControlPdu::Data(data) => match data.share_data_pdu {
            ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(info)) => Some(info),
            _ => None,
        },
        _ => None,
    }
}
