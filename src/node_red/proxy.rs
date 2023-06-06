use std::net::SocketAddr;
use std::time::{Duration, SystemTime};
use std::{error::Error, fmt};
use tokio::net::UdpSocket;
use std::io;

#[derive(Debug)]
pub struct ProxyError {
    kind: String,
    message: String,
}

impl Error for ProxyError {}

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Node-RED proxy error [{kind}]: {message}",
            kind = self.kind,
            message = self.message
        )
    }
}

impl From<std::io::Error> for ProxyError {
    fn from(error: std::io::Error) -> Self {
        if error.raw_os_error().unwrap() == 10060 {
            ProxyError {
                kind: String::from("timeout"),
                message: String::from("Socket timed out while waiting for a response"),
            }
        } else {
            ProxyError {
                kind: String::from("io"),
                message: error.to_string(),
            }
        }
    }
}

pub fn forward_message_to_node_red(
    msg: String,
    timeout: Option<Duration>,
) -> Result<(), ProxyError> {
    // {

    //   let socket = UdpSocket::bind("127.0.0.1:35000").await?;
    //   // socket.set_read_timeout(timeout).expect("Couldn't set socket timeout");

    //   let start = SystemTime::now();
    //   let time = start.duration_since(std::time::UNIX_EPOCH).expect("Couldn't get system time");
    //   let mut buf: [u8; 2048] = [0; 2048];

    //   let destination = SocketAddr::from(([127, 0, 0, 1], 35001));

    //   println!("sending message to Node-RED: '{msg}'");
    //   socket.send_to(msg.as_bytes(), destination)?;

    //   let (_message_length, _src) = socket.recv_from(&mut buf)?;
    //   let time_since_sent = start.elapsed().expect("Couldn't measure time").as_micros() as u64;

    //   let message = String::from_utf8(buf.into());

    //   if let Ok(message) = message {
    //     println!("received message from Node-RED: '{message}'");
    //   } else {
    //     eprintln!("couldn't parse message from Node-RED!");
    //   }

    // }  // the socket is closed here

    Ok(())
}

pub async fn udp_sender_receiver() -> io::Result<()> {
    let sock = UdpSocket::bind("0.0.0.0:35000").await?;
    let mut buf = [0; 1024];
    loop {
        let destination = SocketAddr::from(([127, 0, 0, 1], 35001));

        let len = sock.send_to("hi mom".as_bytes(), destination).await?;
        // println!("{:?} bytes sent", len);
        print!("{:?} bytes sent", len);

        let (len, addr) = sock.recv_from(&mut buf).await?;
        // println!(
        //     "{:?} bytes received from {:?}: {}",
        //     len,
        //     addr,
        //     String::from_utf8(buf.into()).unwrap()
        // );
    }
}
