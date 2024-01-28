use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::mem::{transmute, MaybeUninit};
use std::net::{IpAddr, SocketAddrV4, SocketAddrV6};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

static ACC_SEQ: OnceCell<Arc<Mutex<u16>>> = OnceCell::const_new();

/// ping
/// Returns duration, unit: ms
///
/// ### Example
///
/// ```rust
/// ping(IpAddr::from_str("1.1.1.1").unwrap()).await?
/// ping(IpAddr::from_str("2606:4700:4700::1111").unwrap()).await?
/// ```
#[allow(unused)]
pub async fn ping(addr: IpAddr) -> anyhow::Result<u32> {
    ping_with_timeout(addr, Duration::from_secs(1)).await
}
async fn acc_seq() -> anyhow::Result<u16> {
    let mut guard = ACC_SEQ
        .get_or_init(|| async { Arc::new(Mutex::new(1)) })
        .await
        .lock()
        .map_err(|err| anyhow::format_err!("Failed to get accumulate seq {}", err))?;
    let seq = *guard;
    if *guard == u16::MAX - 1 {
        *guard = 1;
    } else {
        *guard += 1;
    }
    Ok(seq)
}
pub async fn ping_with_timeout(addr: IpAddr, timeout: Duration) -> anyhow::Result<u32> {
    let id = ((std::process::id() % 0xFF) as u16).to_be_bytes();
    let seq = acc_seq().await?.to_be_bytes() as [u8; 2];
    let type_ = Type::RAW;

    let (domain, protocol, dest_addr, mut icmp_packet) = match addr {
        IpAddr::V4(addr) => (
            Domain::IPV4,
            Protocol::ICMPV4,
            SockAddr::from(SocketAddrV4::new(addr, 0)),
            [
                0x08, // icmp v4 type
                0x00, // code
                0x00, 0x00, // checksum
                id[0], id[1], // id
                seq[0], seq[1], // seq
                0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e,
                0x6f, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x61, 0x62, 0x63, 0x64, 0x65,
                0x66, 0x67, 0x68, 0x69, // data
            ],
        ),
        IpAddr::V6(addr) => (
            Domain::IPV6,
            Protocol::ICMPV6,
            SockAddr::from(SocketAddrV6::new(addr, 0, 0, 0)),
            [
                0x80, // icmp v6 type
                0x00, // code
                0x00, 0x00, // checksum
                id[0], id[1], // id
                seq[0], seq[1], // seq
                0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e,
                0x6f, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x61, 0x62, 0x63, 0x64, 0x65,
                0x66, 0x67, 0x68, 0x69, // data
            ],
        ),
    };
    let socket = Socket::new(domain, type_, Some(protocol))?;
    calculate_checksum(&mut icmp_packet);
    let start = Instant::now();
    socket.send_to(&icmp_packet, &dest_addr)?;
    socket.set_read_timeout(Some(timeout))?;
    let is_icmp_echo_reply = async {
        if addr.is_ipv4() {
            let mut buf = [MaybeUninit::zeroed(); 60]; // ipv4: 20 bytes, icmp packet: 40 bytes, so, why does network layer data exist here?
            let (len, _) = socket.recv_from(&mut buf)?;
            let buf: [u8; 60] = unsafe { transmute(buf) };
            Ok(is_icmp_echo_reply(&buf[20..len], &id, &seq)) as anyhow::Result<bool>
        } else {
            let mut buf = [MaybeUninit::zeroed(); 40]; // icmp packet: 40 bytes
            socket.recv_from(&mut buf)?;
            let buf: [u8; 40] = unsafe { transmute(buf) };
            Ok(is_icmp_echo_reply(&buf, &id, &seq))
        }
    }
    .await?;
    if !is_icmp_echo_reply {
        anyhow::bail!("Received packet is not an ICMP echo reply")
    }
    let duration = start.elapsed().as_millis();
    Ok(duration as u32)
}
fn calculate_checksum(packet: &mut [u8]) {
    let mut sum = 0u32;

    for chunk in packet.chunks(2) {
        let word = if chunk.len() == 2 {
            ((chunk[0] as u16) << 8) | (chunk[1] as u16)
        } else {
            (chunk[0] as u16) << 8
        };
        sum += word as u32;
    }

    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    let checksum = !sum as u16;
    packet[2] = (checksum >> 8) as u8;
    packet[3] = checksum as u8;
}
// 检查是否是 ICMP Echo 回应
fn is_icmp_echo_reply(packet: &[u8], id: &[u8], req: &[u8]) -> bool {
    packet.len() >= 4
        && (packet[0] == 0x00 || packet[0] == 0x81)
        && packet[1] == 0
        && packet[4] == id[0]
        && packet[5] == id[1]
        && packet[6] == req[0]
        && packet[7] == req[1]
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::str::FromStr;
    use tokio::task;

    #[tokio::test]
    async fn it_works() {
        match ping(IpAddr::from(Ipv4Addr::from_str("1.1.1.1").unwrap())).await {
            Ok(dur) => {
                println!("Ipv4 收到回应，耗时：{}ms", dur)
            }
            Err(err) => {
                panic!("{:?}", err)
            }
        }
        match ping(IpAddr::from(
            Ipv6Addr::from_str("2606:4700:4700::1111").unwrap(),
        ))
        .await
        {
            Ok(dur) => {
                println!("Ipv6 收到回应，耗时：{}ms", dur)
            }
            Err(err) => {
                panic!("{:?}", err);
            }
        }
    }

    #[tokio::test]
    async fn test_multiple() -> anyhow::Result<()> {
        let addr = IpAddr::from_str("127.0.0.1").unwrap();
        let bad_addr = IpAddr::from_str("199.0.0.0").unwrap();
        let mut tasks = task::JoinSet::new();
        for i in 0..4 {
            tasks.spawn(async move { (i, ping(addr).await) });
        }
        tasks.spawn(async move { (4, ping(bad_addr).await) });
        let mut results = Vec::with_capacity(tasks.len());
        while let Some(task) = tasks.join_next().await {
            let (i, r) = task?;
            results.insert(i, r.ok());
        }
        println!("{:?}", results);
        for result in results.iter().take(4) {
            assert!(result.is_some())
        }
        assert!(results[4].is_none());
        Ok(())
    }
}
