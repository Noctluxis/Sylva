// SPDX-License-Identifier: MPL-2.0
use std::{
    io,
    net::{SocketAddr, UdpSocket},
};

fn string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(value.as_bytes());
    out.push(0);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}
pub fn chat_packet(text: &str) -> io::Result<Vec<u8>> {
    if text.contains('\0') || text.trim().is_empty() || text.encode_utf16().count() > 144 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "消息须为 1–144 字符且不能含 NUL",
        ));
    }
    let mut out = Vec::with_capacity(text.len() + 32);
    string(&mut out, "/chatbox/input");
    // OSC booleans are encoded in the type tag: send immediately, notification off.
    string(&mut out, ",sTF");
    string(&mut out, text);
    Ok(out)
}
pub struct Osc {
    socket: UdpSocket,
    destination: SocketAddr,
    typing_enabled: bool,
    typing: bool,
}
impl Osc {
    pub fn new(destination: SocketAddr) -> io::Result<Self> {
        let socket = UdpSocket::bind(if destination.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            destination,
            typing_enabled: false,
            typing: false,
        })
    }
    pub fn send(&self, text: &str) -> io::Result<()> {
        let packet = chat_packet(text)?;
        self.socket.send_to(&packet, self.destination)?;
        Ok(())
    }
    pub fn set_typing_enabled(&mut self, enabled: bool) -> io::Result<()> {
        self.typing_enabled = enabled;
        if !enabled {
            self.typing(false)?;
        }
        Ok(())
    }
    pub fn typing(&mut self, active: bool) -> io::Result<()> {
        let active = active && self.typing_enabled;
        if self.typing == active {
            return Ok(());
        }
        let mut out = Vec::new();
        string(&mut out, "/chatbox/typing");
        string(&mut out, if active { ",T" } else { ",F" });
        self.socket.send_to(&out, self.destination)?;
        self.typing = active;
        Ok(())
    }
}
impl Drop for Osc {
    fn drop(&mut self) {
        let _ = self.typing(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_and_booleans_reach_udp_receiver() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        let osc = Osc::new(receiver.local_addr().unwrap()).unwrap();
        osc.send("你好👋").unwrap();
        let mut b = [0u8; 512];
        let n = receiver.recv(&mut b).unwrap();
        assert_eq!(n % 4, 0);
        assert_eq!(&b[..16], b"/chatbox/input\0\0");
        assert_eq!(&b[16..24], b",sTF\0\0\0\0");
        assert_eq!(&b[24..34], "你好👋".as_bytes());
        assert!(b[34..n].iter().all(|x| *x == 0));
    }
    #[test]
    fn typing_indicator_is_opt_in_and_stops_when_disabled() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(std::time::Duration::from_millis(50)))
            .unwrap();
        let mut osc = Osc::new(receiver.local_addr().unwrap()).unwrap();
        let mut packet = [0u8; 512];
        let expect_typing = |active: bool, packet: &mut [u8]| {
            let len = receiver.recv(packet).unwrap();
            let mut expected = Vec::new();
            string(&mut expected, "/chatbox/typing");
            string(&mut expected, if active { ",T" } else { ",F" });
            assert_eq!(&packet[..len], expected);
        };
        osc.typing(true).unwrap();
        assert!(receiver.recv(&mut packet).is_err());
        osc.send("照常发送").unwrap();
        let len = receiver.recv(&mut packet).unwrap();
        assert_eq!(&packet[..len], chat_packet("照常发送").unwrap());
        osc.set_typing_enabled(true).unwrap();
        assert!(receiver.recv(&mut packet).is_err());
        osc.typing(true).unwrap();
        expect_typing(true, &mut packet);
        osc.typing(true).unwrap();
        assert!(receiver.recv(&mut packet).is_err());
        osc.set_typing_enabled(false).unwrap();
        expect_typing(false, &mut packet);
        osc.typing(true).unwrap();
        assert!(receiver.recv(&mut packet).is_err());
        osc.set_typing_enabled(true).unwrap();
        osc.typing(true).unwrap();
        expect_typing(true, &mut packet);
        drop(osc);
        expect_typing(false, &mut packet);
    }
    #[test]
    fn message_boundary() {
        assert!(chat_packet(&"中".repeat(144)).is_ok());
        assert!(chat_packet(&"中".repeat(145)).is_err());
        assert!(chat_packet(&"👋".repeat(73)).is_err());
        assert!(chat_packet(" \n").is_err());
        assert!(chat_packet("a\0b").is_err());
    }
}
