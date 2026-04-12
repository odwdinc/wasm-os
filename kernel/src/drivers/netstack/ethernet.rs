// drivers/network/ethernet.rs — Ethernet frame handling

use alloc::vec::Vec;


pub const ETH_TYPE_IPV4: u16 = 0x0800;
pub const ETH_TYPE_ARP:  u16 = 0x0806;
pub const ETH_TYPE_IPV6: u16 = 0x86DD;

pub const ETH_MAC_LEN: usize = 6;

#[derive(Clone, Copy, Debug)]
pub struct EthFrame<'a> {
    pub dst: [u8; ETH_MAC_LEN],
    pub src: [u8; ETH_MAC_LEN],
    pub ethertype: u16,
    pub payload: &'a [u8],
}

impl<'a> EthFrame<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < 14 {
            return None;
        }
        let mut dst = [0u8; 6];
        let mut src = [0u8; 6];
        dst.copy_from_slice(&bytes[0..6]);
        src.copy_from_slice(&bytes[6..12]);
        let ethertype = u16::from_be_bytes([bytes[12], bytes[13]]);
        
        Some(EthFrame {
            dst,
            src,
            ethertype,
            payload: &bytes[14..],
        })
    }

    pub fn compose(buf: &mut [u8], dst: &[u8; 6], src: &[u8; 6], ethertype: u16, payload: &[u8]) -> usize {
        buf[0..6].copy_from_slice(dst);
        buf[6..12].copy_from_slice(src);
        buf[12..14].copy_from_slice(&ethertype.to_be_bytes());
        let n = 14 + payload.len();
        buf[14..n].copy_from_slice(payload);
        n
    }

    pub fn is_for_me(&self, mac: &[u8; 6]) -> bool {
        self.dst == *mac
    }

    pub fn is_broadcast(&self) -> bool {
        self.dst == [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
    }
}