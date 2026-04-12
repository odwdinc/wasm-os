// drivers/netstack/mod.rs — Minimal network stack (Sprint E.2)
//
// Layers implemented:
//   Ethernet  — frame parse/compose
//   ARP       — request/reply + cache; pending SYN flush on ARP resolution
//   IPv4      — parse/build with correct one's-complement checksum
//   ICMP      — echo reply
//   DHCP      — client (DISCOVER → OFFER → REQUEST → ACK), auto-started on init
//   TCP       — passive (listen/accept) and active (connect) with full three-way
//               handshake, data transfer, and FIN/ACK teardown
//   UDP       — bind/send/recv

pub mod arp;
pub mod dhcp;
pub mod ethernet;
pub mod ip;
pub mod tcp;
pub mod udp;

pub use arp::{ArpCache, ArpPacket};
pub use dhcp::DhcpClient;
pub use ethernet::{EthFrame, ETH_TYPE_ARP, ETH_TYPE_IPV4};
pub use ip::{IpAddr, Ipv4Packet, ip_chksum};
pub use tcp::{TcpSocket, TcpState};
pub use udp::UdpSocket;

use crate::drivers::virtio_net::VirtioNet;

pub const MAX_SOCKETS: usize = 8;

static mut NETWORK_STACK: Option<NetworkStack> = None;

pub fn try_init_network() -> bool {
    match NetworkStack::try_init() {
        Some(stack) => { unsafe { NETWORK_STACK = Some(stack); } true }
        None        => false,
    }
}

pub fn with_network<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut NetworkStack) -> R,
{
    unsafe { NETWORK_STACK.as_mut().map(f) }
}

#[derive(Clone, Copy)]
pub struct NetworkHandle(usize);
impl NetworkHandle {
    pub fn as_usize(self) -> usize { self.0 }
}

pub struct NetworkStack {
    pub net:         VirtioNet,
    pub mac:         [u8; 6],
    pub ip:          IpAddr,
    pub dhcp:        DhcpClient,
    pub arp:         ArpCache,
    pub tcp_sockets: [Option<TcpSocket>; MAX_SOCKETS],
    pub udp_sockets: [Option<UdpSocket>; MAX_SOCKETS],
}

impl NetworkStack {
    pub fn try_init() -> Option<Self> {
        let mut net = VirtioNet::try_init()?;
        let mac = net.get_mac();

        if !net.is_link_up() {
            crate::println!("[net] warning: link is down");
        }
        crate::println!("[net] MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);

        let mut stack = NetworkStack {
            net,
            mac,
            ip:          IpAddr([0, 0, 0, 0]),
            dhcp:        DhcpClient::new(&mac),
            arp:         ArpCache::new(),
            tcp_sockets: [const { None }; MAX_SOCKETS],
            udp_sockets: [const { None }; MAX_SOCKETS],
        };
        stack.dhcp_start();
        Some(stack)
    }

    // ── Receive pump ─────────────────────────────────────────────────────────

    /// Drain one received Ethernet frame and dispatch it.
    /// Called every scheduler iteration to keep the stack responsive.
    pub fn poll(&mut self) -> Option<usize> {
        let mut buf = [0u8; 1514];
        match self.net.net_recv(&mut buf) {
            Ok(n) if n > 0 => { self.handle_eth(&buf[..n]); Some(n) }
            _ => None,
        }
    }

    // ── Ethernet ─────────────────────────────────────────────────────────────

    fn handle_eth(&mut self, frame: &[u8]) {
        let eth = match EthFrame::parse(frame) {
            Some(e) => e,
            None    => return,
        };
        if !eth.is_for_me(&self.mac) && !eth.is_broadcast() {
            return;
        }
        match eth.ethertype {
            ETH_TYPE_ARP  => {
                if let Some(p) = arp::ArpPacket::parse(eth.payload) { self.handle_arp(p); }
            }
            ETH_TYPE_IPV4 => {
                if let Some(p) = ip::Ipv4Packet::parse(eth.payload) {
                    // Learn the sender's MAC from every unicast IP frame so we
                    // can reply even without a prior ARP exchange (e.g. TCP from
                    // the SLiRP gateway 10.0.2.2 which never ARPs before SYN).
                    if !eth.is_broadcast() {
                        self.arp.insert(p.src.0, eth.src);
                    }
                    self.handle_ip(p);
                }
            }
            _ => {}
        }
    }

    fn send_eth(&mut self, dst_mac: &[u8; 6], ethertype: u16, payload: &[u8]) {
        let mut frame = [0u8; 1514];
        let n = EthFrame::compose(&mut frame, dst_mac, &self.mac, ethertype, payload);
        let _ = self.net.net_send(&frame[..n]);
    }

    // ── ARP ──────────────────────────────────────────────────────────────────

    fn handle_arp(&mut self, pkt: arp::ArpPacket<'_>) {
        if pkt.operation == arp::ARP_OP_REQUEST && pkt.target_ip == self.ip.0 {
            let reply = arp::ArpPacket::new_reply(
                self.ip.0,       // sender_ip: us
                pkt.sender_ip,   // target_ip: them
                self.mac,        // sender_mac: our MAC
                pkt.sender_mac,  // target_mac: their MAC
            );
            self.send_eth(&pkt.sender_mac, ETH_TYPE_ARP, &reply.to_bytes());
        }
        if pkt.operation == arp::ARP_OP_REPLY {
            self.arp.insert(pkt.sender_ip, pkt.sender_mac);
            self.flush_pending_syncs(IpAddr(pkt.sender_ip));
        }
    }

    fn send_arp_request(&mut self, target_ip: IpAddr) {
        let req = arp::ArpPacket {
            operation:  arp::ARP_OP_REQUEST,
            sender_mac: self.mac,
            sender_ip:  self.ip.0,
            target_mac: [0u8; 6],
            target_ip:  target_ip.0,
            payload:    &[],
        };
        let broadcast = [0xFF_u8; 6];
        self.send_eth(&broadcast, ETH_TYPE_ARP, &req.to_bytes());
    }

    // ── IP helpers ───────────────────────────────────────────────────────────

    /// Send an IPv4 datagram.  Handles 255.255.255.255 as a direct broadcast
    /// (no ARP needed).  For other destinations, looks up the MAC in the ARP
    /// cache; if not present, triggers an ARP request and drops the datagram.
    fn send_ip(&mut self, dst_ip: IpAddr, ip_payload: &[u8]) {
        if dst_ip.0 == [255, 255, 255, 255] {
            let broadcast = [0xFF_u8; 6];
            self.send_eth(&broadcast, ETH_TYPE_IPV4, ip_payload);
            return;
        }
        if let Some(mac) = self.arp.lookup(dst_ip.0) {
            self.send_eth(&mac, ETH_TYPE_IPV4, ip_payload);
        } else {
            self.send_arp_request(dst_ip);
        }
    }

    // ── IPv4 dispatch ────────────────────────────────────────────────────────

    fn handle_ip(&mut self, pkt: Ipv4Packet<'_>) {
        match pkt.protocol {
            ip::IP_PROTO_ICMP => {
                if pkt.payload.len() >= 8 && pkt.payload[0] == 8 {
                    self.send_icmp_reply(pkt.src, pkt.payload);
                }
            }
            ip::IP_PROTO_TCP => {
                if let Some(seg) = tcp::TcpPacket::parse(pkt.payload) {
                    self.handle_tcp(pkt.src, seg);
                }
            }
            ip::IP_PROTO_UDP => {
                if let Some(seg) = udp::UdpPacket::parse(pkt.payload) {
                    self.handle_udp(pkt.src, seg);
                }
            }
            _ => {}
        }
    }

    // ── ICMP ─────────────────────────────────────────────────────────────────

    fn send_icmp_reply(&mut self, dst: IpAddr, request: &[u8]) {
        if request.len() < 8 { return; }

        let reply_len = request.len().min(64);
        let mut icmp = [0u8; 64];
        icmp[0] = 0; // type: Echo Reply
        icmp[1] = 0; // code: 0
        let copy = (reply_len - 4).min(request.len() - 4);
        icmp[4..4 + copy].copy_from_slice(&request[4..4 + copy]);

        let cksum = ip_chksum(&icmp[..reply_len]);
        icmp[2] = (cksum >> 8) as u8;
        icmp[3] =  cksum        as u8;

        let ip_bytes = ip::Ipv4Packet::new(self.ip, dst, ip::IP_PROTO_ICMP, &icmp[..reply_len])
            .to_bytes();
        self.send_ip(dst, &ip_bytes);
    }

    // ── DHCP ─────────────────────────────────────────────────────────────────

    /// Send a DHCPDISCOVER and enter the Selecting state.
    pub fn dhcp_start(&mut self) {
        self.dhcp.state = dhcp::DhcpState::Selecting;
        let pkt = self.dhcp.build_discover(&self.mac);
        self.send_dhcp_udp(&pkt);
        crate::println!("[net] DHCP DISCOVER sent");
    }

    /// Returns `true` once a lease has been confirmed by DHCPACK.
    pub fn is_dhcp_bound(&self) -> bool {
        self.dhcp.state == dhcp::DhcpState::Bound
    }

    /// Send a DHCP UDP datagram from 0.0.0.0:68 → 255.255.255.255:67 via
    /// Ethernet broadcast (used before we have an assigned IP).
    fn send_dhcp_udp(&mut self, payload: &[u8]) {
        let src_ip = IpAddr([0, 0, 0, 0]);
        let dst_ip = IpAddr([255, 255, 255, 255]);
        let udp_pkt   = udp::UdpPacket::new(dhcp::CLIENT_PORT, dhcp::SERVER_PORT, payload);
        let udp_bytes = udp_pkt.to_bytes();
        let ip_bytes  = ip::Ipv4Packet::new(src_ip, dst_ip, ip::IP_PROTO_UDP, &udp_bytes)
            .to_bytes();
        let broadcast = [0xFF_u8; 6];
        self.send_eth(&broadcast, ETH_TYPE_IPV4, &ip_bytes);
    }

    fn handle_dhcp(&mut self, payload: &[u8]) {
        // Copy payload so we don't borrow self through it while also mutating.
        // DHCP packets are at most ~576 bytes in practice; 1024 is safe.
        let mut buf = [0u8; 1024];
        let n = payload.len().min(buf.len());
        buf[..n].copy_from_slice(&payload[..n]);
        let data = &buf[..n];

        match self.dhcp.handle_packet(data) {
            Some(dhcp::DhcpAction::SendRequest) => {
                crate::println!("[net] DHCP OFFER {}.{}.{}.{} from {}.{}.{}.{}",
                    self.dhcp.offered_ip[0], self.dhcp.offered_ip[1],
                    self.dhcp.offered_ip[2], self.dhcp.offered_ip[3],
                    self.dhcp.server_ip[0],  self.dhcp.server_ip[1],
                    self.dhcp.server_ip[2],  self.dhcp.server_ip[3]);
                let req = self.dhcp.build_request(&self.mac);
                self.send_dhcp_udp(&req);
            }
            Some(dhcp::DhcpAction::Bound(ip)) => {
                self.ip = IpAddr(ip);
                crate::println!(
                    "[net] DHCP ACK — IP {}.{}.{}.{}  GW {}.{}.{}.{}  lease {}s",
                    ip[0], ip[1], ip[2], ip[3],
                    self.dhcp.gateway[0], self.dhcp.gateway[1],
                    self.dhcp.gateway[2], self.dhcp.gateway[3],
                    self.dhcp.lease_secs,
                );
            }
            Some(dhcp::DhcpAction::Nak) => {
                crate::println!("[net] DHCP NAK — restarting discovery");
                self.dhcp_start();
            }
            None => {}
        }
    }

    // ── TCP ──────────────────────────────────────────────────────────────────

    fn find_tcp_socket(&self, local_port: u16, remote_port: u16) -> Option<usize> {
        for (i, s) in self.tcp_sockets.iter().enumerate() {
            if let Some(s) = s {
                if s.local_port == local_port && s.remote_port == remote_port {
                    return Some(i);
                }
            }
        }
        for (i, s) in self.tcp_sockets.iter().enumerate() {
            if let Some(s) = s {
                if s.local_port == local_port && s.state == TcpState::Listen {
                    return Some(i);
                }
            }
        }
        None
    }

    fn handle_tcp(&mut self, src_ip: IpAddr, pkt: tcp::TcpPacket<'_>) {
        let idx = match self.find_tcp_socket(pkt.dst_port, pkt.src_port) {
            Some(i) => i,
            None    => return,
        };

        let reply = self.tcp_sockets[idx].as_mut().unwrap().handle_pkt(pkt, src_ip);

        if let Some(r) = reply {
            let (lport, rport, rip) = {
                let s = self.tcp_sockets[idx].as_ref().unwrap();
                (s.local_port, s.remote_port, s.remote_ip)
            };
            self.send_tcp_seg(lport, rport, rip, r.seq, r.ack, r.flags, &[]);
        }
    }

    fn send_tcp_seg(
        &mut self,
        lport: u16, rport: u16, rip: IpAddr,
        seq: u32, ack: u32, flags: u8,
        payload: &[u8],
    ) {
        let seg      = tcp::make_tcp_packet(lport, rport, seq, ack, flags, 65535, payload, &self.ip, &rip);
        let ip_bytes = ip::Ipv4Packet::new(self.ip, rip, ip::IP_PROTO_TCP, &seg).to_bytes();
        self.send_ip(rip, &ip_bytes);
    }

    fn flush_pending_syncs(&mut self, resolved_ip: IpAddr) {
        for i in 0..MAX_SOCKETS {
            let info = self.tcp_sockets[i].as_ref().and_then(|s| {
                if s.state == TcpState::SynSent && s.remote_ip == resolved_ip {
                    Some((s.local_port, s.remote_port, s.seq, s.remote_ip))
                } else {
                    None
                }
            });
            if let Some((lp, rp, seq, rip)) = info {
                self.send_tcp_seg(lp, rp, rip, seq, 0, tcp::TCP_SYN, &[]);
            }
        }
    }

    fn alloc_ephemeral_port(&self) -> u16 {
        let mut port = 49152u16;
        'outer: loop {
            for slot in &self.tcp_sockets {
                if let Some(s) = slot {
                    if s.local_port == port {
                        port = port.wrapping_add(1);
                        continue 'outer;
                    }
                }
            }
            return port;
        }
    }

    // ── TCP public API ───────────────────────────────────────────────────────

    pub fn tcp_listen(&mut self, port: u16) -> Option<usize> {
        for (i, slot) in self.tcp_sockets.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(TcpSocket::listen(port));
                return Some(i);
            }
        }
        None
    }

    pub fn tcp_connect(&mut self, dst_ip: IpAddr, dst_port: u16) -> Option<usize> {
        let local_port = self.alloc_ephemeral_port();
        for (i, slot) in self.tcp_sockets.iter_mut().enumerate() {
            if slot.is_none() {
                let mut s     = TcpSocket::connect(local_port);
                s.remote_ip   = dst_ip;
                s.remote_port = dst_port;
                let seq       = s.seq;
                *slot         = Some(s);

                if self.arp.lookup(dst_ip.0).is_some() {
                    self.send_tcp_seg(local_port, dst_port, dst_ip, seq, 0, tcp::TCP_SYN, &[]);
                } else {
                    self.send_arp_request(dst_ip);
                }
                return Some(i);
            }
        }
        None
    }

    pub fn tcp_send(&mut self, sock_idx: usize, data: &[u8]) -> Result<usize, ()> {
        let (state, seq, ack, rip, rport, lport) = match &self.tcp_sockets[sock_idx] {
            Some(s) => (s.state, s.seq, s.ack, s.remote_ip, s.remote_port, s.local_port),
            None    => return Err(()),
        };
        if state != TcpState::Established { return Err(()); }

        let n = data.len().min(tcp::MAX_TCP_PAYLOAD);
        self.send_tcp_seg(lport, rport, rip, seq, ack, tcp::TCP_PSH | tcp::TCP_ACK, &data[..n]);
        self.tcp_sockets[sock_idx].as_mut().unwrap().seq = seq.wrapping_add(n as u32);
        Ok(n)
    }

    pub fn tcp_recv(&mut self, sock_idx: usize, buf: &mut [u8]) -> Result<usize, ()> {
        match self.tcp_sockets[sock_idx].as_mut() {
            Some(s) => s.recv(buf),
            None    => Err(()),
        }
    }

    /// If the socket at `listen_idx` has completed a three-way handshake
    /// (state == Established), clone it into a new free slot and reset the
    /// original back to Listen so the caller can accept the next connection.
    /// Returns the new connected socket's index, or `None` if not ready.
    pub fn tcp_accept(&mut self, listen_idx: usize) -> Option<usize> {
        let state = self.tcp_sockets[listen_idx].as_ref()?.state;
        if state != TcpState::Established { return None; }

        // Find a free slot for the connected socket.
        let free_slot = self.tcp_sockets.iter().position(|s| s.is_none())?;

        // Copy (socket is Copy) established connection to free slot.
        self.tcp_sockets[free_slot] = self.tcp_sockets[listen_idx];

        // Reset the listener so it can accept the next connection.
        let port = self.tcp_sockets[listen_idx].as_ref().unwrap().local_port;
        self.tcp_sockets[listen_idx] = Some(TcpSocket::listen(port));

        Some(free_slot)
    }

    /// Return the current state of a TCP socket as an integer:
    ///   0 = closed/invalid, 1 = listening, 2 = handshaking (SYN sent/recv),
    ///   3 = established, 4 = half-closed / teardown
    pub fn tcp_status(&self, sock_idx: usize) -> i32 {
        match self.tcp_sockets.get(sock_idx).and_then(|s| s.as_ref()) {
            None => 0,
            Some(s) => match s.state {
                TcpState::Closed                => 0,
                TcpState::Listen                => 1,
                TcpState::SynSent
                | TcpState::SynRecv             => 2,
                TcpState::Established           => 3,
                _                               => 4,
            },
        }
    }

    /// Return the kernel's current IP address as a u32 (same byte order as
    /// `IpAddr::from_u32` / `to_u32`, i.e. little-endian octets).
    /// Returns 0 if DHCP has not yet bound.
    pub fn get_ip(&self) -> u32 {
        self.ip.to_u32()
    }

    pub fn tcp_close(&mut self, sock_idx: usize) -> Result<(), ()> {
        let (state, seq, ack, rip, rport, lport) = match &self.tcp_sockets[sock_idx] {
            Some(s) => (s.state, s.seq, s.ack, s.remote_ip, s.remote_port, s.local_port),
            None    => return Err(()),
        };
        match state {
            TcpState::Established | TcpState::CloseWait => {
                self.tcp_sockets[sock_idx].as_mut().unwrap().state = TcpState::FinWait1;
                self.send_tcp_seg(lport, rport, rip, seq, ack, tcp::TCP_FIN | tcp::TCP_ACK, &[]);
                Ok(())
            }
            _ => Err(()),
        }
    }

    // ── UDP public API ───────────────────────────────────────────────────────

    pub fn udp_bind(&mut self, port: u16) -> Option<usize> {
        for (i, slot) in self.udp_sockets.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(UdpSocket::new(port));
                return Some(i);
            }
        }
        None
    }

    pub fn udp_send(&mut self, sock_idx: usize, data: &[u8]) -> Result<usize, ()> {
        let (lport, rport, rip) = match &self.udp_sockets[sock_idx] {
            Some(s) => (s.local_port, s.remote_port, s.remote_ip),
            None    => return Err(()),
        };
        if rport == 0 || rip.is_zero() { return Err(()); }

        let n        = data.len().min(udp::MAX_UDP_PAYLOAD);
        let pkt      = udp::UdpPacket::new(lport, rport, &data[..n]);
        let pb       = pkt.to_bytes();
        let ip_bytes = ip::Ipv4Packet::new(self.ip, rip, ip::IP_PROTO_UDP, &pb).to_bytes();
        self.send_ip(rip, &ip_bytes);
        Ok(n)
    }

    /// Set the remote address/port on a UDP socket (required before `udp_send`).
    pub fn udp_connect(&mut self, sock_idx: usize, dst_ip: IpAddr, dst_port: u16) -> Result<(), ()> {
        match self.udp_sockets.get_mut(sock_idx).and_then(|s| s.as_mut()) {
            Some(s) => { s.remote_ip = dst_ip; s.remote_port = dst_port; Ok(()) }
            None    => Err(()),
        }
    }

    /// Free a UDP socket slot.
    pub fn udp_close(&mut self, sock_idx: usize) {
        if sock_idx < MAX_SOCKETS {
            self.udp_sockets[sock_idx] = None;
        }
    }

    pub fn udp_recv(&mut self, sock_idx: usize, buf: &mut [u8]) -> Result<usize, ()> {
        match self.udp_sockets[sock_idx].as_mut() {
            Some(s) => s.recv(buf),
            None    => Err(()),
        }
    }

    // ── UDP dispatch ─────────────────────────────────────────────────────────

    fn handle_udp(&mut self, src_ip: IpAddr, pkt: udp::UdpPacket<'_>) {
        // Intercept DHCP replies (server port 67 → client port 68).
        if pkt.src_port == dhcp::SERVER_PORT && pkt.dst_port == dhcp::CLIENT_PORT {
            self.handle_dhcp(pkt.payload);
            return;
        }

        for slot in self.udp_sockets.iter_mut() {
            if let Some(s) = slot {
                if s.local_port == pkt.dst_port {
                    s.handle_pkt(pkt, src_ip);
                    return;
                }
            }
        }
    }

    // ── Misc ─────────────────────────────────────────────────────────────────

    pub fn set_ip(&mut self, ip: IpAddr) {
        self.ip = ip;
        crate::println!("[net] IP {}.{}.{}.{}", ip.0[0], ip.0[1], ip.0[2], ip.0[3]);
    }
}
