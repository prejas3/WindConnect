//! CCS/1 IO:IPv4 broadcast -> untrusted peer hints | SEC:no trust changes
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::BTreeSet,
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};
use tokio::net::UdpSocket;

pub const BROADCAST_PORT: u16 = 47821;
pub const PACKET_SIZE: usize = 39;
const MAGIC: &[u8; 4] = b"WCD1";

#[derive(Clone, Debug)]
pub struct BroadcastConfig {
    pub listen: SocketAddrV4,
    /// Empty means directed broadcasts of current interfaces, falling back to 255.255.255.255.
    pub destinations: Vec<SocketAddrV4>,
    pub interval: Duration,
    pub expiry: Duration,
}

impl Default for BroadcastConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, BROADCAST_PORT),
            destinations: Vec::new(),
            interval: Duration::from_secs(2),
            expiry: Duration::from_secs(8),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Packet {
    pub query: bool,
    pub port: u16,
    pub fingerprint: [u8; 32],
}

impl Packet {
    pub fn encode(&self) -> [u8; PACKET_SIZE] {
        let mut bytes = [0; PACKET_SIZE];
        bytes[..4].copy_from_slice(MAGIC);
        bytes[4] = if self.query { 1 } else { 2 };
        bytes[5..7].copy_from_slice(&self.port.to_be_bytes());
        bytes[7..].copy_from_slice(&self.fingerprint);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != PACKET_SIZE || &bytes[..4] != MAGIC || !matches!(bytes[4], 1 | 2) {
            return None;
        }
        let port = u16::from_be_bytes([bytes[5], bytes[6]]);
        if port == 0 {
            return None;
        }
        Some(Self {
            query: bytes[4] == 1,
            port,
            fingerprint: bytes[7..].try_into().ok()?,
        })
    }
}

pub struct BroadcastDiscovery {
    socket: UdpSocket,
    fingerprint: [u8; 32],
    config: BroadcastConfig,
}

impl BroadcastDiscovery {
    pub fn bind(fingerprint: [u8; 32], config: BroadcastConfig) -> io::Result<Self> {
        if config.listen.port() == 0
            || config.interval.is_zero()
            || config.expiry <= config.interval
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid broadcast port/timing",
            ));
        }
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        // CCS/1 RULE:multiple local devices/processes can receive the same broadcast discovery port.
        socket.set_reuse_address(true)?;
        socket.set_broadcast(true)?;
        socket.set_nonblocking(true)?;
        socket.bind(&SocketAddr::V4(config.listen).into())?;
        Ok(Self {
            socket: UdpSocket::from_std(socket.into())?,
            fingerprint,
            config,
        })
    }

    pub fn config(&self) -> &BroadcastConfig {
        &self.config
    }

    pub async fn query(&self, port: u16) -> io::Result<()> {
        let destinations = if self.config.destinations.is_empty() {
            let mut targets = BTreeSet::new();
            for interface in if_addrs::get_if_addrs()? {
                if let if_addrs::IfAddr::V4(address) = interface.addr {
                    if !address.ip.is_loopback() && !address.ip.is_unspecified() {
                        if let Some(broadcast) = address.broadcast {
                            targets.insert(SocketAddrV4::new(broadcast, self.config.listen.port()));
                        }
                    }
                }
            }
            if targets.is_empty() {
                targets.insert(SocketAddrV4::new(
                    Ipv4Addr::BROADCAST,
                    self.config.listen.port(),
                ));
            }
            targets.into_iter().collect()
        } else {
            self.config.destinations.clone()
        };
        let bytes = Packet {
            query: true,
            port,
            fingerprint: self.fingerprint,
        }
        .encode();
        let mut sent = false;
        let mut error = None;
        for target in destinations {
            match self.socket.send_to(&bytes, target).await {
                Ok(_) => sent = true,
                Err(e) => error = Some(e),
            }
        }
        if sent {
            Ok(())
        } else {
            Err(error.unwrap_or_else(|| io::Error::other("no broadcast destinations")))
        }
    }

    pub async fn receive(&self) -> io::Result<Option<(Packet, SocketAddrV4)>> {
        // CCS/1 SEC:one extra byte detects and rejects truncated oversized datagrams.
        let mut bytes = [0; PACKET_SIZE + 1];
        let (length, source) = match self.socket.recv_from(&mut bytes).await {
            Ok(received) => received,
            // Windows reports, on the next receive, two things that aren't
            // this socket's failure: 10040 (WSAEMSGSIZE) for an oversized
            // datagram, and 10054 (WSAECONNRESET) for an ICMP "port
            // unreachable" caused by an earlier send to a peer that is gone
            // (WN-85). Both are skipped; treating 10054 as an error would
            // switch broadcast discovery off until the next network change.
            Err(error) if cfg!(windows) && matches!(error.raw_os_error(), Some(10040 | 10054)) => {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        let SocketAddr::V4(source) = source else {
            return Ok(None);
        };
        let Some(packet) = Packet::decode(&bytes[..length]) else {
            return Ok(None);
        };
        if source.port() != self.config.listen.port()
            || source.ip().is_unspecified()
            || source.ip().is_multicast()
            || source.ip().is_broadcast()
            || packet.fingerprint == self.fingerprint
        {
            return Ok(None);
        }
        Ok(Some((packet, source)))
    }

    /// Caller rate limits replies; answers are the same size as requests and never trigger replies.
    pub async fn answer(&self, port: u16, destination: SocketAddrV4) -> io::Result<()> {
        let bytes = Packet {
            query: false,
            port,
            fingerprint: self.fingerprint,
        }
        .encode();
        self.socket.send_to(&bytes, destination).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packet_is_bounded_versioned_and_uses_network_order() {
        let packet = Packet {
            query: true,
            port: 47820,
            fingerprint: [0xab; 32],
        };
        let bytes = packet.encode();
        assert_eq!(Packet::decode(&bytes), Some(packet));
        assert_eq!(&bytes[5..7], &47820u16.to_be_bytes());
        for length in 0..PACKET_SIZE {
            assert!(Packet::decode(&bytes[..length]).is_none());
        }
        let mut invalid = bytes;
        invalid[0] = b'X';
        assert!(Packet::decode(&invalid).is_none());
        invalid = bytes;
        invalid[4] = 3;
        assert!(Packet::decode(&invalid).is_none());
        invalid = bytes;
        invalid[5..7].fill(0);
        assert!(Packet::decode(&invalid).is_none());
        let mut oversized = bytes.to_vec();
        oversized.push(0);
        assert!(Packet::decode(&oversized).is_none());
    }
}
