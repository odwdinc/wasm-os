// drivers/network/ip.rs — IPv4 packet handling

use super::ethernet::ETH_MAC_LEN;
use alloc::vec::Vec;


pub const IP_PROTO_ICMP: u8 = 1;
pub const IP_PROTO_TCP: u8 = 6;
pub const IP_PROTO_UDP: u8 = 17;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpAddr(pub [u8; 4]);

impl IpAddr {
    pub fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        IpAddr([a, b, c, d])
    }

    pub fn from_u32(v: u32) -> Self {
        IpAddr(v.to_le_bytes())
    }

    pub fn to_u32(&self) -> u32 {
        u32::from_le_bytes(self.0)
    }

    pub fn is_zero(&self) -> bool {
        self.0 == [0, 0, 0, 0]
    }

    pub fn is_loopback(&self) -> bool {
        self.0[0] == 127
    }

    pub fn is_private(&self) -> bool {
        // 10.0.0.0/8
        self.0[0] == 10 ||
        // 172.16.0.0/12
        (self.0[0] == 172 && self.0[1] >= 16 && self.0[1] <= 31) ||
        // 192.168.0.0/16
        (self.0[0] == 192 && self.0[1] == 168)
    }
}

impl Default for IpAddr {
    fn default() -> Self {
        IpAddr([0, 0, 0, 0])
    }
}

pub struct Ipv4Packet<'a>{
    pub version_ihl: u8,
    pub tos: u8,
    pub total_len: u16,
    pub id: u16,
    pub flags_offset: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src: IpAddr,
    pub dst: IpAddr,
    pub payload: &'a [u8],
}

impl<'a> Ipv4Packet<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < 20 {
            return None;
        }
        
        let version_ihl = bytes[0];
        if version_ihl >> 4 != 4 {
            return None; // Not IPv4
        }
        
        let ihl = (version_ihl & 0x0F) as usize * 4;
        if ihl < 20 || bytes.len() < ihl {
            return None;
        }

        let total_len = u16::from_be_bytes([bytes[2], bytes[3]]);
        let id = u16::from_be_bytes([bytes[4], bytes[5]]);
        let flags_offset = u16::from_be_bytes([bytes[6], bytes[7]]);
        let ttl = bytes[8];
        let protocol = bytes[9];
        let checksum = u16::from_be_bytes([bytes[10], bytes[11]]);
        
        let mut src = [0u8; 4];
        let mut dst = [0u8; 4];
        src.copy_from_slice(&bytes[12..16]);
        dst.copy_from_slice(&bytes[16..20]);

        Some(Ipv4Packet {
            version_ihl,
            tos: bytes[1],
            total_len,
            id,
            flags_offset,
            ttl,
            protocol,
            checksum,
            src: IpAddr(src),
            dst: IpAddr(dst),
            payload: &bytes[ihl..],
        })
    }

    pub fn new(src: IpAddr, dst: IpAddr, protocol: u8, payload: &'a [u8]) -> Self {
        let total_len = 20 + payload.len() as u16;
        Ipv4Packet {
            version_ihl: 0x45, // version 4, IHL 5 (20 bytes)
            tos: 0,
            total_len,
            id: 0,
            flags_offset: 0,
            ttl: 64,
            protocol,
            checksum: 0,
            src,
            dst,
            payload,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(20 + self.payload.len());
        
        v.push(self.version_ihl);
        v.push(self.tos);
        v.extend_from_slice(&self.total_len.to_be_bytes());
        v.extend_from_slice(&self.id.to_be_bytes());
        v.extend_from_slice(&self.flags_offset.to_be_bytes());
        v.push(self.ttl);
        v.push(self.protocol);
        
        // Placeholder checksum
        v.extend_from_slice(&0u16.to_be_bytes());
        
        v.extend_from_slice(&self.src.0);
        v.extend_from_slice(&self.dst.0);
        
        v.extend_from_slice(self.payload);
        
        // Fix up checksum (IP header only — first 20 bytes)
        let sum = ip_chksum(&v[..20]);
        v[10] = (sum >> 8) as u8;
        v[11] = sum as u8;
        
        v
    }
}

pub fn ip_chksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in data.chunks(2) {
        if chunk.len() == 2 {
            sum += ((chunk[0] as u32) << 8) | chunk[1] as u32;
        } else {
            sum += (chunk[0] as u32) << 8;
        }
    }
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}