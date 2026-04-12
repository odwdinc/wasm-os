// drivers/network/udp.rs — UDP handling

use super::ip::IpAddr;
use alloc::vec::Vec;


pub const MAX_UDP_PAYLOAD: usize = 1472;

#[derive(Copy, Clone)]
pub struct UdpSocket {
    pub local_port: u16,
    pub remote_port: u16,
    pub remote_ip: IpAddr,
    pub recv_buf: [u8; MAX_UDP_PAYLOAD],
    pub recv_len: usize,
}

impl UdpSocket {
    pub fn new(port: u16) -> Self {
        UdpSocket {
            local_port: port,
            remote_port: 0,
            remote_ip: IpAddr::default(),
            recv_buf: [0; MAX_UDP_PAYLOAD],
            recv_len: 0,
        }
    }

    pub fn handle_pkt(&mut self, pkt: UdpPacket<'_>, src: IpAddr) {
        self.remote_ip = src;
        self.remote_port = pkt.src_port;
        
        let n = pkt.payload.len().min(MAX_UDP_PAYLOAD);
        self.recv_buf[..n].copy_from_slice(&pkt.payload[..n]);
        self.recv_len = n;
    }

    pub fn recv(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if self.recv_len == 0 {
            return Err(());
        }

        let n = self.recv_len.min(buf.len());
        buf[..n].copy_from_slice(&self.recv_buf[..n]);
        self.recv_len = 0;
        Ok(n)
    }
}

pub struct UdpPacket<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub length: u16,
    pub checksum: u16,
    pub payload: &'a [u8],
}

impl<'a> UdpPacket<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < 8 {
            return None;
        }

        let src_port = u16::from_be_bytes([bytes[0], bytes[1]]);
        let dst_port = u16::from_be_bytes([bytes[2], bytes[3]]);
        let length = u16::from_be_bytes([bytes[4], bytes[5]]);
        let checksum = u16::from_be_bytes([bytes[6], bytes[7]]);

        Some(UdpPacket {
            src_port,
            dst_port,
            length,
            checksum,
            payload: &bytes[8..],
        })
    }

    pub fn new(src_port: u16, dst_port: u16, payload: &'a [u8]) -> Self {
        let length = 8 + payload.len() as u16;
        UdpPacket {
            src_port,
            dst_port,
            length,
            checksum: 0,
            payload,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(8 + self.payload.len());
        v.extend_from_slice(&self.src_port.to_be_bytes());
        v.extend_from_slice(&self.dst_port.to_be_bytes());
        v.extend_from_slice(&self.length.to_be_bytes());
        v.extend_from_slice(&self.checksum.to_be_bytes());
        v.extend_from_slice(self.payload);
        v
    }
}