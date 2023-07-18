use std::{
    collections::HashMap,
    error::Error,
    fmt,
    net::{SocketAddr, UdpSocket},
    time::{Duration, SystemTime},
};
use local_ip_address::local_ip;

use crate::NodeRedHttpClient;

#[derive(Debug)]
pub struct LatencyTestError {
    pub kind: String,
    pub message: String,
}

impl Error for LatencyTestError {}

impl fmt::Display for LatencyTestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Latency test error [{kind}]: {message}",
            kind = self.kind,
            message = self.message
        )
    }
}

impl From<std::io::Error> for LatencyTestError {
    fn from(error: std::io::Error) -> Self {
        if error.raw_os_error().unwrap() == 10060 {
            LatencyTestError {
                kind: String::from("timeout"),
                message: String::from("Socket timed out while waiting for a response"),
            }
        } else {
            LatencyTestError {
                kind: String::from("io"),
                message: error.to_string(),
            }
        }
    }
}

impl From<std::time::SystemTimeError> for LatencyTestError {
    fn from(error: std::time::SystemTimeError) -> Self {
        LatencyTestError {
            kind: String::from("time"),
            message: error.to_string(),
        }
    }
}

impl From<local_ip_address::Error> for LatencyTestError {
    fn from(error: local_ip_address::Error) -> Self {
        LatencyTestError {
            kind: String::from("ip"),
            message: error.to_string(),
        }
    }
}

pub struct LatencyTestResult {
    pub receiver_time_offset: i128,
    pub round_trip_time: u64,
}

pub async fn submit_test_result(
    client: &NodeRedHttpClient,
    result: LatencyTestResult,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut body = HashMap::<&str, i128>::new();
    body.insert("receiver_time_offset", result.receiver_time_offset as i128);
    body.insert("round_trip_time", result.round_trip_time as i128);

    let request = client
        .client
        .post(client.path_to_url("/test-result"))
        .json(&body)
        .send()
        .await;

    match request {
        Ok(_response) => {
            //   println!("status: {}", response.status());
        }
        Err(err) => {
            eprintln!(
                "Couldn't submit test result to Node-RED: {}\n{:?}",
                err.source().unwrap(),
                err
            );
        }
    }

    Ok(())
}

pub fn test_latency(destination: SocketAddr, iterations: usize) -> LatencyTestResult {
    let mut result = LatencyTestResult {
        receiver_time_offset: 0,
        round_trip_time: 0,
    };

    let mut successful_iterations: u64 = 0;

    for i in 1..=iterations {
        print!("Iteration {i}: ");
        match test_latency_once(destination, Some(Duration::from_millis(250))) {
            Ok(current_result) => {
                print!("Receiver time offset: {} µs, ", current_result.receiver_time_offset);
                print!("RTT: {} µs", current_result.round_trip_time);
                println!();

                result.receiver_time_offset += current_result.receiver_time_offset;
                result.round_trip_time += current_result.round_trip_time;
                successful_iterations += 1;
            }
            Err(err) => eprintln!("{}", err),
        }
    }

    if successful_iterations > 0 {
        result.receiver_time_offset /= successful_iterations as i128;
        result.round_trip_time /= successful_iterations;
    }

    result
}

// sends a UDP ping from a random port to the given destination
fn test_latency_once(
    destination: SocketAddr,
    timeout: Option<Duration>,
) -> Result<LatencyTestResult, LatencyTestError> {
    let result;
    {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket
            .set_read_timeout(timeout)
            .expect("Couldn't set socket timeout");

        let start = SystemTime::now();
        let time = start
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Couldn't get system time");

        let data = serde_json::json!({
            "type": "udpPing",
            "replyTo": format!("{}:{}", local_ip()?, socket.local_addr()?.port()),
        });
        
        socket.send_to(&data.to_string().into_bytes(), destination)?;
        
        let mut buffer = [0; 8];
        let (_message_length, _src) = socket.recv_from(&mut buffer)?;
        let time_since_sent = start.elapsed().expect("Couldn't measure time");
        let receiver_time = std::time::Duration::from_micros(u64::from_be_bytes(buffer));
        // dbg!(receiver_time);
        let receiver_time_offset = -((time + time_since_sent/2).as_micros() as i128 - receiver_time.as_micros() as i128);

        result = LatencyTestResult {
            receiver_time_offset,
            round_trip_time: time_since_sent.as_micros() as u64,
        };
    } // the socket is closed here

    Result::Ok(result)
}
