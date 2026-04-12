// drivers/netstack/tcp.rs — TCP socket state machine and packet building

use super::ip::IpAddr;
use alloc::vec::Vec;

pub const MAX_TCP_PAYLOAD: usize = 1460;

// TCP flag bits
pub const TCP_FIN: u8 = 0x01;
pub const TCP_SYN: u8 = 0x02;
pub const TCP_RST: u8 = 0x04;
pub const TCP_PSH: u8 = 0x08;
pub const TCP_ACK: u8 = 0x10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynRecv,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

#[derive(Copy, Clone)]
pub struct TcpSocket {
    pub local_port:  u16,
    pub remote_port: u16,
    pub remote_ip:   IpAddr,
    pub state:       TcpState,
    pub seq:         u32,
    pub ack:         u32,
    pub last_seq:    u32,
    pub last_ack:    u32,
    pub recv_buf:    [u8; MAX_TCP_PAYLOAD],
    pub recv_len:    usize,
}

/// Describes a TCP segment that `NetworkStack` should transmit in response
/// to an incoming packet.
pub struct TcpReply {
    pub flags: u8,
    pub seq:   u32,
    pub ack:   u32,
}

impl TcpSocket {
    pub fn listen(port: u16) -> Self {
        TcpSocket {
            local_port:  port,
            remote_port: 0,
            remote_ip:   IpAddr::default(),
            state:       TcpState::Listen,
            seq:         1,   // ISN for passive-open sockets
            ack:         0,
            last_seq:    0,
            last_ack:    0,
            recv_buf:    [0; MAX_TCP_PAYLOAD],
            recv_len:    0,
        }
    }

    pub fn connect(local_port: u16) -> Self {
        TcpSocket {
            local_port,
            remote_port: 0,
            remote_ip:   IpAddr::default(),
            state:       TcpState::SynSent,
            seq:         1000,  // ISN for active-open sockets
            ack:         0,
            last_seq:    0,
            last_ack:    0,
            recv_buf:    [0; MAX_TCP_PAYLOAD],
            recv_len:    0,
        }
    }

    /// Process an incoming TCP segment.  Returns `Some(TcpReply)` if a
    /// response segment should be sent immediately (SYN-ACK, ACK, etc.),
    /// or `None` if no reply is needed.
    pub fn handle_pkt(&mut self, pkt: TcpPacket<'_>, src: IpAddr) -> Option<TcpReply> {
        self.last_seq = pkt.seq;
        self.last_ack = pkt.ack_no;

        // RST always closes the connection.
        if pkt.rst() {
            self.state = TcpState::Closed;
            return None;
        }

        match self.state {
            TcpState::Listen => {
                if pkt.syn() && !pkt.ack() {
                    self.remote_ip   = src;
                    self.remote_port = pkt.src_port;
                    self.ack         = pkt.seq.wrapping_add(1);
                    self.state       = TcpState::SynRecv;
                    // Reply: SYN|ACK
                    return Some(TcpReply { flags: TCP_SYN | TCP_ACK, seq: self.seq, ack: self.ack });
                }
                None
            }

            TcpState::SynSent => {
                if pkt.syn() && pkt.ack() {
                    // Three-way handshake: remote sent SYN-ACK.
                    self.seq   = pkt.ack_no;          // advance past our SYN
                    self.ack   = pkt.seq.wrapping_add(1);
                    self.state = TcpState::Established;
                    return Some(TcpReply { flags: TCP_ACK, seq: self.seq, ack: self.ack });
                } else if pkt.syn() {
                    // Simultaneous open.
                    self.remote_ip   = src;
                    self.remote_port = pkt.src_port;
                    self.ack         = pkt.seq.wrapping_add(1);
                    self.state       = TcpState::SynRecv;
                    return Some(TcpReply { flags: TCP_SYN | TCP_ACK, seq: self.seq, ack: self.ack });
                }
                None
            }

            TcpState::SynRecv => {
                if pkt.ack() {
                    // SYN we sent has been acknowledged; SYN consumed one seq number.
                    self.seq   = self.seq.wrapping_add(1);
                    self.state = TcpState::Established;
                }
                None
            }

            TcpState::Established => {
                if pkt.fin() {
                    self.ack   = pkt.seq.wrapping_add(1);
                    self.state = TcpState::CloseWait;
                    return Some(TcpReply { flags: TCP_ACK, seq: self.seq, ack: self.ack });
                }
                if !pkt.payload.is_empty() {
                    let n = pkt.payload.len().min(MAX_TCP_PAYLOAD - self.recv_len);
                    self.recv_buf[self.recv_len..self.recv_len + n]
                        .copy_from_slice(&pkt.payload[..n]);
                    self.recv_len += n;
                    self.ack = pkt.seq.wrapping_add(pkt.payload.len() as u32);
                    return Some(TcpReply { flags: TCP_ACK, seq: self.seq, ack: self.ack });
                }
                None
            }

            TcpState::FinWait1 => {
                if pkt.fin() && pkt.ack() {
                    self.ack   = pkt.seq.wrapping_add(1);
                    self.state = TcpState::Closed;
                    return Some(TcpReply { flags: TCP_ACK, seq: self.seq, ack: self.ack });
                } else if pkt.fin() {
                    self.ack   = pkt.seq.wrapping_add(1);
                    self.state = TcpState::Closing;
                    return Some(TcpReply { flags: TCP_ACK, seq: self.seq, ack: self.ack });
                } else if pkt.ack() {
                    self.state = TcpState::FinWait2;
                }
                None
            }

            TcpState::FinWait2 => {
                if pkt.fin() {
                    self.ack   = pkt.seq.wrapping_add(1);
                    self.state = TcpState::Closed;
                    return Some(TcpReply { flags: TCP_ACK, seq: self.seq, ack: self.ack });
                }
                None
            }

            TcpState::Closing => {
                if pkt.ack() {
                    self.state = TcpState::Closed;
                }
                None
            }

            _ => None,
        }
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

pub struct TcpPacket<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq:      u32,
    pub ack_no:   u32,
    pub data_off: u8,
    pub flags:    u8,
    pub window:   u16,
    pub checksum: u16,
    pub payload:  &'a [u8],
}

impl<'a> TcpPacket<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < 20 {
            return None;
        }
        let src_port = u16::from_be_bytes([bytes[0],  bytes[1]]);
        let dst_port = u16::from_be_bytes([bytes[2],  bytes[3]]);
        let seq      = u32::from_be_bytes([bytes[4],  bytes[5],  bytes[6],  bytes[7]]);
        let ack_no   = u32::from_be_bytes([bytes[8],  bytes[9],  bytes[10], bytes[11]]);
        let data_off = bytes[12];
        let flags    = bytes[13];
        let window   = u16::from_be_bytes([bytes[14], bytes[15]]);
        let checksum = u16::from_be_bytes([bytes[16], bytes[17]]);

        let hdr_len = ((data_off >> 4) as usize) * 4;
        if hdr_len < 20 || hdr_len > bytes.len() {
            return None;
        }
        Some(TcpPacket { src_port, dst_port, seq, ack_no, data_off, flags, window, checksum, payload: &bytes[hdr_len..] })
    }

    pub fn syn(&self) -> bool { self.flags & TCP_SYN != 0 }
    pub fn ack(&self) -> bool { self.flags & TCP_ACK != 0 }
    pub fn fin(&self) -> bool { self.flags & TCP_FIN != 0 }
    pub fn rst(&self) -> bool { self.flags & TCP_RST != 0 }
    pub fn psh(&self) -> bool { self.flags & TCP_PSH != 0 }
}

/// Build a complete TCP segment (header + payload) with a correct checksum.
pub fn make_tcp_packet(
    src_port: u16, dst_port: u16,
    seq: u32, ack: u32,
    flags: u8, window: u16,
    payload: &[u8],
    src_ip: &IpAddr, dst_ip: &IpAddr,
) -> Vec<u8> {
    let mut v = Vec::with_capacity(20 + payload.len());

    v.extend_from_slice(&src_port.to_be_bytes());
    v.extend_from_slice(&dst_port.to_be_bytes());
    v.extend_from_slice(&seq.to_be_bytes());
    v.extend_from_slice(&ack.to_be_bytes());
    v.push(0x50);                            // data offset: 5 × 4 = 20 bytes
    v.push(flags);
    v.extend_from_slice(&window.to_be_bytes());
    v.extend_from_slice(&0u16.to_be_bytes()); // checksum placeholder
    v.extend_from_slice(&0u16.to_be_bytes()); // urgent pointer
    v.extend_from_slice(payload);

    // RFC 793 checksum: one's-complement sum of pseudo-header + TCP segment.
    // All 16-bit words are in network (big-endian) order.
    let mut sum: u32 = 0;

    // Pseudo-header: src IP, dst IP, zero, protocol (6), TCP length
    let s = src_ip.0;
    let d = dst_ip.0;
    sum += ((s[0] as u32) << 8) | s[1] as u32;
    sum += ((s[2] as u32) << 8) | s[3] as u32;
    sum += ((d[0] as u32) << 8) | d[1] as u32;
    sum += ((d[2] as u32) << 8) | d[3] as u32;
    sum += 6u32;                              // protocol: TCP
    sum += v.len() as u32;                    // TCP segment length

    // Entire TCP segment (header + data), checksum field is still zero.
    for chunk in v.chunks(2) {
        if chunk.len() == 2 {
            sum += ((chunk[0] as u32) << 8) | chunk[1] as u32;
        } else {
            sum += (chunk[0] as u32) << 8;
        }
    }

    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    let cksum = !(sum as u16);
    v[16] = (cksum >> 8) as u8;
    v[17] =  cksum        as u8;

    v
}
