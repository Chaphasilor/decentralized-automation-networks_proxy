use std::io;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};
use std::{error::Error, fmt};
use serde_json::json;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

use crate::db::{Db, Event, EventIdentifier, Message, MessageType};

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

pub async fn forward_message_to_node_red(
    msg: serde_json::Value,
    timeout: Option<Duration>,
    tx: mpsc::Sender<Message>,
) -> Result<(), ProxyError> {
    {
        let socket = UdpSocket::bind("127.0.0.1:35000").await?;
        // socket.set_read_timeout(timeout).expect("Couldn't set socket timeout");

        let start = SystemTime::now();
        let time = start
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Couldn't get system time");
        //TODO store time locally so that it can be accessed from `udp_proxy_receiver()`

        let destination = SocketAddr::from(([127, 0, 0, 1], 35001));

        // println!("sending message to Node-RED: {msg}", msg=msg.to_string());
        socket.send_to(msg.to_string().as_bytes(), destination).await?;

        if let Err(err) = tx
            .send(Message{
                message_type: MessageType::Event(Event {
                    source: None,
                    destination: Some(destination),
                    timestamp: start,
                    identifier: EventIdentifier{
                        flow_name: msg["meta"].as_object().unwrap()["flow_name"].as_str().unwrap().to_string(),
                        execution_area: msg["meta"].as_object().unwrap()["execution_area"].as_str().unwrap().to_string(),
                    }
                }),
                response: None,
            }).await
        {
            return Err(ProxyError {
                kind: "MessagePassing".to_string(),
                message: err.to_string(),
            });
        }
    } // the socket is closed here

    Ok(())
}

/**
 * Receives messages from Node-RED and forwards them to the target
 */
pub async fn udp_node_red_receiver(tx: mpsc::Sender<Message>) -> Result<(), ProxyError> {
    let sock = UdpSocket::bind("0.0.0.0:35000").await?;
    let mut buf = [0; 2048];
    loop {
        let (len, addr) = sock.recv_from(&mut buf).await?;
        let received = SystemTime::now();

        let message = String::from_utf8(buf.into());

        if let Ok(message) = message {
            let message = message.trim_matches(char::from(0)); // trim any NULL characters that are left over from the buffer
            // println!("received message from Node-RED: {message}");

            let message_json: serde_json::Value = serde_json::from_str(message).unwrap();

            //TODO forward message to actual target

            if let Err(err) = tx
                .send(Message{
                    message_type: MessageType::Event(Event {
                        source: Some(addr),
                        destination: None,
                        timestamp: received,
                        identifier: EventIdentifier{
                            flow_name: message_json["meta"]["flow_name"].as_str().unwrap().to_string(),
                            execution_area: message_json["meta"]["execution_area"].as_str().unwrap().to_string(),
                        }
                    }),
                    response: None,
                }).await
            {
                return Err(ProxyError {
                    kind: "MessagePassing".to_string(),
                    message: err.to_string(),
                });
            }
            
        } else {
            eprintln!("couldn't parse message from Node-RED!");
        }

    }
}

/**
 * Receives messages from sensors or other proxies and forwards them to Node-RED or another proxy
 */
pub async fn udp_proxy_receiver(tx: mpsc::Sender<Message>) -> Result<(), ProxyError> {
    let sock = UdpSocket::bind("0.0.0.0:34000").await?;
    let mut buf = [0; 2048];
    loop {
        let (len, addr) = sock.recv_from(&mut buf).await?;
        let received = SystemTime::now();

        let message = std::str::from_utf8(&buf);

        if let Ok(message) = message {
            let message = message.trim_matches(char::from(0)); // trim any NULL characters that are left over from the buffer
            // println!("received message from sensor or proxy: '{message}'");
            let message_json: serde_json::Value = serde_json::from_str(message).unwrap();

            if let Err(err) = tx
                .send(Message{
                    message_type: MessageType::Event(Event {
                        source: Some(addr),
                        destination: None,
                        timestamp: received,
                        identifier: EventIdentifier{
                            flow_name: message_json["meta"]["flow_name"].as_str().unwrap().to_string(),
                            execution_area: message_json["meta"]["execution_area"].as_str().unwrap().to_string(),
                        }
                    }),
                    response: None,
                }).await
            {
                return Err(ProxyError {
                    kind: "MessagePassing".to_string(),
                    message: err.to_string(),
                });
            }

            if let Err(err) =
                forward_message_to_node_red(message_json, Some(Duration::from_millis(250)), tx.clone()).await
            {
                eprintln!("Failed to forward message to Node-RED: {}", err.to_string());
            };
            let time_since_sent = received.elapsed().expect("Couldn't measure time").as_micros() as u64;

        } else {
            eprintln!("couldn't parse message from sensor or proxy!");
        }

        //TODO forward message to actual target
    }
}
