// drivers/netstack/dhcp.rs — DHCP client (RFC 2131)
//
// Implements the four-message exchange:
//   Client  →  DHCPDISCOVER  (broadcast)
//   Server  →  DHCPOFFER
//   Client  →  DHCPREQUEST   (broadcast, echoes chosen offer)
//   Server  →  DHCPACK
//
// All packets are sent as UDP (src=0.0.0.0:68 → dst=255.255.255.255:67) via
// Ethernet broadcast so they work before the client has an IP address.

use alloc::vec::Vec;

pub const CLIENT_PORT: u16 = 68;
pub const SERVER_PORT: u16 = 67;

// DHCP message-type option values (option 53)
const MSG_DISCOVER: u8 = 1;
const MSG_OFFER:    u8 = 2;
const MSG_REQUEST:  u8 = 3;
const MSG_ACK:      u8 = 5;
const MSG_NAK:      u8 = 6;

const MAGIC: [u8; 4] = [0x63, 0x82, 0x53, 0x63];

// ── State machine ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DhcpState {
    Idle,
    Selecting,   // DISCOVER sent — waiting for OFFER
    Requesting,  // REQUEST sent  — waiting for ACK
    Bound,
}

pub struct DhcpClient {
    pub state:       DhcpState,
    pub xid:         u32,        // transaction ID
    pub offered_ip:  [u8; 4],
    pub server_ip:   [u8; 4],
    pub subnet_mask: [u8; 4],
    pub gateway:     [u8; 4],
    pub lease_secs:  u32,
}

/// Action the `NetworkStack` should take after `DhcpClient::handle_packet`.
pub enum DhcpAction {
    /// OFFER received — send a DHCPREQUEST.
    SendRequest,
    /// ACK received — the client is bound; field contains the assigned IP.
    Bound([u8; 4]),
    /// NAK received — restart discovery.
    Nak,
}

// ── Implementation ────────────────────────────────────────────────────────────

impl DhcpClient {
    pub fn new(mac: &[u8; 6]) -> Self {
        // Derive a transaction ID from the last four MAC bytes.
        let xid = ((mac[2] as u32) << 24)
                | ((mac[3] as u32) << 16)
                | ((mac[4] as u32) <<  8)
                |  (mac[5] as u32);
        DhcpClient {
            state:       DhcpState::Idle,
            xid,
            offered_ip:  [0; 4],
            server_ip:   [0; 4],
            subnet_mask: [0; 4],
            gateway:     [0; 4],
            lease_secs:  86400,
        }
    }

    /// Build a DHCPDISCOVER packet (call before transitioning to Selecting).
    pub fn build_discover(&self, mac: &[u8; 6]) -> Vec<u8> {
        self.build_bootp(mac, MSG_DISCOVER, [0; 4], [0; 4])
    }

    /// Build a DHCPREQUEST packet (call after receiving an OFFER).
    pub fn build_request(&self, mac: &[u8; 6]) -> Vec<u8> {
        self.build_bootp(mac, MSG_REQUEST, self.offered_ip, self.server_ip)
    }

    fn build_bootp(
        &self,
        mac:          &[u8; 6],
        msg_type:     u8,
        requested_ip: [u8; 4],
        server_id:    [u8; 4],
    ) -> Vec<u8> {
        let mut p = [0u8; 300];

        p[0] = 1;   // op: BOOTREQUEST
        p[1] = 1;   // htype: Ethernet
        p[2] = 6;   // hlen: 6
        // p[3] = 0 hops, already zero
        p[4..8].copy_from_slice(&self.xid.to_be_bytes());
        p[10..12].copy_from_slice(&0x8000u16.to_be_bytes()); // broadcast flag
        p[28..34].copy_from_slice(mac);                       // chaddr
        p[236..240].copy_from_slice(&MAGIC);

        let mut o = 240usize;

        // Option 53: DHCP message type
        p[o] = 53; p[o+1] = 1; p[o+2] = msg_type;
        o += 3;

        // Option 50: requested IP address  (DHCPREQUEST only)
        if requested_ip != [0u8; 4] {
            p[o] = 50; p[o+1] = 4;
            p[o+2..o+6].copy_from_slice(&requested_ip);
            o += 6;
        }

        // Option 54: server identifier  (DHCPREQUEST only)
        if server_id != [0u8; 4] {
            p[o] = 54; p[o+1] = 4;
            p[o+2..o+6].copy_from_slice(&server_id);
            o += 6;
        }

        // Option 55: parameter request list
        p[o] = 55; p[o+1] = 3;
        p[o+2] = 1;   // subnet mask
        p[o+3] = 3;   // router
        p[o+4] = 51;  // lease time
        o += 5;

        p[o] = 255; // End option
        o += 1;

        p[..o].to_vec()
    }

    /// Parse an incoming UDP payload from port 67.  Returns the action to
    /// take, or `None` if the packet is not relevant to the current state.
    pub fn handle_packet(&mut self, data: &[u8]) -> Option<DhcpAction> {
        if data.len() < 240 { return None; }

        // Must be a BOOTREPLY with our transaction ID and the correct magic cookie.
        if data[0] != 2 { return None; }
        if data[236..240] != MAGIC { return None; }
        let xid = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        if xid != self.xid { return None; }

        let yiaddr = [data[16], data[17], data[18], data[19]];

        // Walk the options TLV list.
        let mut msg_type    = 0u8;
        let mut server_id   = [0u8; 4];
        let mut subnet_mask = [0u8; 4];
        let mut gateway     = [0u8; 4];
        let mut lease_secs  = 86400u32;

        let mut i = 240;
        while i < data.len() {
            let tag = data[i];
            if tag == 255 { break; }      // End
            if tag == 0   { i += 1; continue; } // Pad
            if i + 1 >= data.len() { break; }
            let len = data[i + 1] as usize;
            if i + 2 + len > data.len() { break; }
            let val = &data[i + 2..i + 2 + len];

            match tag {
                53 if len >= 1 => msg_type = val[0],
                54 if len >= 4 => server_id.copy_from_slice(&val[..4]),
                1  if len >= 4 => subnet_mask.copy_from_slice(&val[..4]),
                3  if len >= 4 => gateway.copy_from_slice(&val[..4]),
                51 if len >= 4 => {
                    lease_secs = u32::from_be_bytes([val[0], val[1], val[2], val[3]]);
                }
                _ => {}
            }
            i += 2 + len;
        }

        match (self.state, msg_type) {
            (DhcpState::Selecting, MSG_OFFER) => {
                self.offered_ip  = yiaddr;
                self.server_ip   = server_id;
                self.subnet_mask = subnet_mask;
                self.gateway     = gateway;
                self.lease_secs  = lease_secs;
                self.state       = DhcpState::Requesting;
                Some(DhcpAction::SendRequest)
            }
            (DhcpState::Requesting, MSG_ACK) => {
                self.offered_ip = yiaddr;
                self.state      = DhcpState::Bound;
                Some(DhcpAction::Bound(yiaddr))
            }
            (DhcpState::Requesting, MSG_NAK) => {
                self.state = DhcpState::Idle;
                Some(DhcpAction::Nak)
            }
            _ => None,
        }
    }
}
