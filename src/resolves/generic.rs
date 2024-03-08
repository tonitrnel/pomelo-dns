use crate::resolves::DNSResolver;
use tokio::net::UdpSocket;

pub struct Generic<'input> {
    target: &'input str,
    udp_payload_size: usize,
}

impl<'input> Generic<'input> {
    pub fn new(target: &'input str) -> Self {
        Generic { target, udp_payload_size: 4096 }
    }
    pub fn set_udp_payload_size(&mut self, udp_payload_size: usize){
        self.udp_payload_size = udp_payload_size;
    }
}

impl<'input> DNSResolver for Generic<'input> {
    async fn resolve(&mut self, bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.send_to(bytes, self.target).await?;
        let mut response = vec![0; self.udp_payload_size];
        let (len, _) = socket.recv_from(&mut response).await?;
        response.truncate(len);
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::op::Message;
    use hickory_proto::rr::RecordType;
    use hickory_proto::serialize::binary::BinDecodable;

    #[tokio::test]
    async fn it_works() {
        let mut dns = Generic::new("1.1.1.1:53");
        // query example.com
        let bytes = [
            0x00, 0x02, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x65,
            0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x03, 0x63, 0x6f, 0x6d, 0x00, 0x00, 0x01, 0x00,
            0x01,
        ];
        let response = dns.resolve(&bytes).await.unwrap();
        let message = Message::from_bytes(&response).unwrap();
        assert!(!message.answers().is_empty());
        assert_eq!(message.answers()[0].name().to_utf8(), "example.com.");
        assert_eq!(message.answers()[0].record_type(), RecordType::A);
    }
}
