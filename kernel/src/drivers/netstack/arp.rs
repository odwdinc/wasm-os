// drivers/network/arp.rs — ARP handling

use super::ethernet::ETH_MAC_LEN;


pub const ARP_OP_REQUEST: u16 = 1;
pub const ARP_OP_REPLY:   u16 = 2;

pub const MAX_ARP_ENTRIES: usize = 16;

pub struct ArpCache {
    pub ips: [[u8; 4]; MAX_ARP_ENTRIES],
    pub macs: [[u8; ETH_MAC_LEN]; MAX_ARP_ENTRIES],
    pub valid: [bool; MAX_ARP_ENTRIES],
}

impl ArpCache {
    pub fn new() -> Self {
        ArpCache {
            ips: [[0; 4]; MAX_ARP_ENTRIES],
            macs: [[0; ETH_MAC_LEN]; MAX_ARP_ENTRIES],
            valid: [false; MAX_ARP_ENTRIES],
        }
    }

    pub fn insert(&mut self, ip: [u8; 4], mac: [u8; 6]) {
        // Check if already exists
        for i in 0..MAX_ARP_ENTRIES {
            if self.valid[i] && self.ips[i] == ip {
                self.macs[i].copy_from_slice(&mac);
                return;
            }
        }
        // Add new entry
        for i in 0..MAX_ARP_ENTRIES {
            if !self.valid[i] {
                self.ips[i].copy_from_slice(&ip);
                self.macs[i].copy_from_slice(&mac);
                self.valid[i] = true;
                return;
            }
        }
    }

    pub fn lookup(&self, ip: [u8; 4]) -> Option<[u8; 6]> {
        for i in 0..MAX_ARP_ENTRIES {
            if self.valid[i] && self.ips[i] == ip {
                return Some(self.macs[i]);
            }
        }
        None
    }
}

pub struct ArpPacket {
    pub operation: u16,
    pub sender_ip: [u8; 4],
    pub target_ip: [u8; 4],
    pub sender_mac: [u8; 6],
    pub target_mac: [u8; 6],
}

impl ArpPacket {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 28 {
            return None;
        }
        
        let htype = u16::from_be_bytes([bytes[0], bytes[1]]);
        let ptype = u16::from_be_bytes([bytes[2], bytes[3]]);
        if htype != 1 || ptype != 0x0800 {
            return None; // Not Ethernet/IPv4
        }
        
        let operation = u16::from_be_bytes([bytes[6], bytes[7]]);
        let mut sender_mac = [0u8; 6];
        let mut target_mac = [0u8; 6];
        sender_mac.copy_from_slice(&bytes[8..14]);
        target_mac.copy_from_slice(&bytes[18..24]);
        let mut sender_ip = [0u8; 4];
        let mut target_ip = [0u8; 4];
        sender_ip.copy_from_slice(&bytes[14..18]);
        target_ip.copy_from_slice(&bytes[24..28]);

        Some(ArpPacket {
            operation,
            sender_ip,
            target_ip,
            sender_mac,
            target_mac,
        })
    }

    pub fn new_reply(sender_ip: [u8; 4], target_ip: [u8; 4],
                 sender_mac: [u8; 6], target_mac: [u8; 6]) -> ArpPacket {
        ArpPacket {
            operation: ARP_OP_REPLY,
            sender_ip,
            target_ip,
            sender_mac,
            target_mac,
        }
    }

    pub fn to_bytes(&self) -> [u8; 28] {
        let mut b = [0u8; 28];
        b[0..2].copy_from_slice(&1u16.to_be_bytes());  // HTYPE: Ethernet
        b[2..4].copy_from_slice(&0x0800u16.to_be_bytes()); // PTYPE: IPv4
        b[4] = 6;  // HALEN
        b[5] = 4;  // PALEN
        b[6..8].copy_from_slice(&self.operation.to_be_bytes());
        b[8..14].copy_from_slice(&self.sender_mac);
        b[14..18].copy_from_slice(&self.sender_ip);
        b[18..24].copy_from_slice(&self.target_mac);
        b[24..28].copy_from_slice(&self.target_ip);
        b
    }
}