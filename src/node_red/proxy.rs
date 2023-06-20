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
    outbound_socket: &UdpSocket,
    destination_port: u16,
    msg: serde_json::Value,
    timeout: Option<Duration>,
    tx: mpsc::Sender<Message>,
) -> Result<(), ProxyError> {
    {
        // socket.set_read_timeout(timeout).expect("Couldn't set socket timeout");

        let destination = SocketAddr::from(([127, 0, 0, 1], destination_port));

        // println!("sending message to Node-RED: {msg}", msg=msg.to_string());
        outbound_socket.send_to(msg.to_string().as_bytes(), destination).await?;
        let sent = SystemTime::now();

        if let Err(err) = tx
            .send(Message{
                message_type: MessageType::Event(Event {
                    source: None,
                    destination: Some(destination),
                    timestamp: sent,
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
pub async fn udp_node_red_receiver(tx: mpsc::Sender<Message>, inbound_socket: UdpSocket, outbound_socket: UdpSocket, destination_port_base: u16) -> Result<(), ProxyError> {

    let mut buf = [0; 2048];
    loop {

        buf.fill(0); // clear the buffer
        
        let (len, addr) = inbound_socket.recv_from(&mut buf).await?;
        let received = SystemTime::now();
        let reception_port = inbound_socket.local_addr().unwrap().port();
        // println!("received message from Node-RED on port {port}", port=reception_port.to_string());

        let message = String::from_utf8(buf.into());

        if let Ok(message) = message {
            let message = message.trim_matches(char::from(0)); // trim any NULL characters that are left over from the buffer
            // println!("received message from Node-RED: {message}");

            let message_json: serde_json::Value = serde_json::from_str(message).unwrap();

            // log incoming message
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

            let destination_port = destination_port_base + (reception_port % 1000);
            let ip_string = message_json["target_ip"].as_str().unwrap();
            let ip: Vec<u8> = ip_string.split(".").map(|x| x.parse::<u8>().unwrap()).collect();
            let destination = SocketAddr::from(([ip[0], ip[1], ip[2], ip[3]], destination_port));

            // println!("destination: {destination}", destination=destination.to_string());

            if let Err(err) = outbound_socket.send_to(message_json.to_string().as_bytes(), destination).await {
                eprintln!("couldn't send message to destination: {}", err.to_string());
            } else {
                let sent = SystemTime::now();
    
                // log outgoing message
                if let Err(err) = tx
                    .send(Message{
                        message_type: MessageType::Event(Event {
                            source: None,
                            destination: Some(destination),
                            timestamp: sent,
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
            }
            
        } else {
            eprintln!("couldn't parse message from Node-RED!");
        }

    }
}

/**
 * Receives messages from sensors or other proxies and forwards them to Node-RED or another proxy
 */
pub async fn udp_proxy_receiver(tx: mpsc::Sender<Message>, inbound_socket: UdpSocket, outbound_socket: UdpSocket, destination_port_base: u16) -> Result<(), ProxyError> {

    let mut buf = [0; 2048];
    loop {

        buf.fill(0); // clear the buffer

        let (len, addr) = inbound_socket.recv_from(&mut buf).await?;
        let received = SystemTime::now();
        let reception_port = inbound_socket.local_addr().unwrap().port();
        // println!("received message from input node on port {port}", port=reception_port.to_string());

        let message = std::str::from_utf8(&buf);

        if let Ok(message) = message {
            let message = message.trim_matches(char::from(0)); // trim any NULL characters that are left over from the buffer
            // println!("received message from sensor or proxy: '{message}'");
            let message_json: serde_json::Value = serde_json::from_str(message).unwrap();

            let destination_port = destination_port_base + (reception_port % 1000);

            if let Err(err) =
                forward_message_to_node_red(&outbound_socket, destination_port, message_json.clone(), Some(Duration::from_millis(250)), tx.clone()).await
            {
                eprintln!("Failed to forward message to Node-RED: {}", err.to_string());
            };
            let time_since_sent = received.elapsed().expect("Couldn't measure time").as_micros() as u64;

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
            eprintln!("couldn't parse message from sensor or proxy!");
        }

        //TODO forward message to actual target
    }
}
