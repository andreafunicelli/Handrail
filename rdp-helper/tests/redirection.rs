//! Recognising a Connection Broker's Server Redirection PDU where licensing should start
//! (vendor/ironrdp-connector/src/handrail.rs).

use ironrdp_connector::handrail::{parse_redirection, set_error_info, Redirection, LB_NOREDIRECT};
use ironrdp_pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentLicensingCode};

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn blob(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
    out.extend_from_slice(bytes);
}

/// `RDP_SERVER_REDIRECTION_PACKET` with a target address, load-balance info, a password cookie,
/// FQDN and NetBIOS name.
fn packet() -> Vec<u8> {
    let mut fields = Vec::new();
    blob(&mut fields, &utf16z("10.0.0.21"));
    blob(
        &mut fields,
        b"tsv://MS Terminal Services Plugin.1.Desktops\r\n",
    );
    blob(&mut fields, &[0xAA; 12]);
    blob(&mut fields, &utf16z("rdsh02.corp.example"));
    blob(&mut fields, &utf16z("RDSH02"));
    let redir_flags: u32 = 0x0001 | 0x0002 | 0x0010 | 0x0100 | 0x0200;
    let mut p = Vec::new();
    p.extend_from_slice(&0x0400u16.to_le_bytes());
    p.extend_from_slice(&u16::try_from(12 + fields.len()).unwrap().to_le_bytes());
    p.extend_from_slice(&7u32.to_le_bytes());
    p.extend_from_slice(&redir_flags.to_le_bytes());
    p.extend_from_slice(&fields);
    p
}

fn expected() -> Redirection {
    Redirection {
        session_id: 7,
        redir_flags: 0x0313,
        target_net_address: Some("10.0.0.21".into()),
        load_balance_info: Some(b"tsv://MS Terminal Services Plugin.1.Desktops\r\n".to_vec()),
        username: None,
        domain: None,
        target_fqdn: Some("rdsh02.corp.example".into()),
        target_netbios_name: Some("RDSH02".into()),
    }
}

#[test]
fn enhanced_security_redirection() {
    // Share control header (totalLength, pduType = PDUTYPE_SERVER_REDIR_PKT | version 1,
    // pduSource), pad2Octets, the packet. The length has the licensing bit (0x80) set on
    // purpose: it must not be mistaken for a licensing PDU.
    let p = packet();
    let mut pdu = Vec::new();
    pdu.extend_from_slice(&0x00C0u16.to_le_bytes());
    pdu.extend_from_slice(&0x001Au16.to_le_bytes());
    pdu.extend_from_slice(&0x03EAu16.to_le_bytes());
    pdu.extend_from_slice(&[0, 0]);
    pdu.extend_from_slice(&p);
    let r = parse_redirection(&pdu).expect("a redirection");
    assert_eq!(r, expected());
    assert_eq!(r.target(), Some("rdsh02.corp.example"));
}

#[test]
fn standard_security_redirection() {
    let mut pdu = Vec::new();
    pdu.extend_from_slice(&0x0400u16.to_le_bytes());
    pdu.extend_from_slice(&0u16.to_le_bytes());
    pdu.extend_from_slice(&packet());
    assert_eq!(parse_redirection(&pdu), Some(expected()));
}

#[test]
fn truncated_names_still_give_the_address() {
    let mut pdu = vec![0x00, 0x04, 0, 0];
    let p = packet();
    // Cut inside the FQDN.
    pdu.extend_from_slice(&p[..p.len() - 20]);
    let r = parse_redirection(&pdu).expect("a redirection");
    assert_eq!(r.target(), Some("10.0.0.21"));
}

#[test]
fn licensing_pdus_are_not_redirections() {
    // Basic security header with SEC_LICENSE_PKT, then a License Request preamble.
    let license = [0x80, 0x00, 0x00, 0x00, 0x01, 0x03, 0x10, 0x00, 0, 0, 0, 0];
    assert_eq!(parse_redirection(&license), None);
    assert_eq!(parse_redirection(&[0x80]), None);
    assert_eq!(parse_redirection(&[]), None);
}

#[test]
fn informational_redirection_keeps_its_flag() {
    // LB_NOREDIRECT with no other fields: the connector skips it instead of failing.
    let mut pdu = vec![0x00, 0x04, 0, 0, 0x00, 0x04, 12, 0];
    pdu.extend_from_slice(&3u32.to_le_bytes());
    pdu.extend_from_slice(&LB_NOREDIRECT.to_le_bytes());
    let r = parse_redirection(&pdu).expect("a redirection");
    assert_eq!(
        (r.session_id, r.redir_flags & LB_NOREDIRECT),
        (3, LB_NOREDIRECT)
    );
    assert_eq!(r.target(), None);
}

/// What a Windows Server 2019 with no reachable RD license server sent in place of the first
/// licensing PDU: Set Error Info, ERRINFO_LICENSE_NO_LICENSE_SERVER (0x101).
const NO_LICENSE_SERVER: [u8; 22] = [
    0x16, 0x00, 0x17, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x16, 0x00, 0x2f, 0x00,
    0x00, 0x00, 0x01, 0x01, 0x00, 0x00,
];

#[test]
fn reads_the_servers_licensing_refusal() {
    assert_eq!(
        set_error_info(&NO_LICENSE_SERVER),
        Some(ErrorInfo::ProtocolIndependentLicensingCode(
            ProtocolIndependentLicensingCode::NoLicenseServer
        ))
    );
    assert_eq!(parse_redirection(&NO_LICENSE_SERVER), None);
}

#[test]
fn licensing_pdus_are_not_error_info() {
    let license = [0x80, 0x00, 0x00, 0x00, 0x01, 0x03, 0x10, 0x00, 0, 0, 0, 0];
    assert_eq!(set_error_info(&license), None);
    assert_eq!(set_error_info(&[]), None);
}
