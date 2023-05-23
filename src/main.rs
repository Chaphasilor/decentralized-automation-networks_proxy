use std::{error::Error, fmt};
use std::time::{SystemTime, Duration};
use std::net::{UdpSocket, SocketAddr};
use std::collections::HashMap;

#[derive(Debug)]
struct LatencyTestError {
    kind: String,
    message: String,
}

impl Error for LatencyTestError {}

impl fmt::Display for LatencyTestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Latency test error [{kind}]: {message}", kind = self.kind, message = self.message)
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

struct LatencyTestResult {
    trip_time: u64,
    round_trip_time: u64,
}

fn main() -> std::io::Result<()> {
    let test_result = test_latency(10);

    println!("\n=== Test Result ===\n");
    println!("One-way latency: {} µs", test_result.trip_time);
    println!("Round-Trip-Time: {} µs", test_result.round_trip_time);
    println!();

    submit_test_result(test_result).unwrap();
    
    Ok(())
}

fn submit_test_result(result: LatencyTestResult) -> Result<(), Box<dyn std::error::Error>> {

    let mut body = HashMap::new();
    body.insert("trip_time", result.trip_time); 
    body.insert("round_trip_time", result.round_trip_time); 
    
    let client = reqwest::blocking::Client::new();
    let request = client.post("http://localhost:1880/test-result").json(&body).send();
    
    match request {
        Ok(response) => {
            println!("status: {}", response.status());
        },
        Err(err) => {
            eprintln!("Couldn't submit test result to Node-RED: {}\n{:?}", err.source().unwrap(), err);
        }
    }

    Ok(())
    
}

fn test_latency(iterations: usize) -> LatencyTestResult {

    let mut result = LatencyTestResult {
        trip_time: 0,
        round_trip_time: 0,
    };
    
    let mut successful_iterations = 0;
    
    for i in 1..=iterations {
        print!("Iteration {i}: ");
        match test_latency_once(Some(Duration::from_millis(250))) {
            Ok(current_result ) => {
                print!("TT: {} µs, ", current_result.trip_time);
                print!("RTT: {} µs", current_result.round_trip_time);
                println!("");
        
                result.trip_time += current_result.trip_time;
                result.round_trip_time += current_result.round_trip_time;
                successful_iterations += 1;
            }
            Err(err) => eprintln!("{}", err.to_string())
        }
    }

    result.trip_time /= successful_iterations;
    result.round_trip_time /= successful_iterations;

    result
    
}

fn test_latency_once(timeout: Option<Duration>) -> Result<LatencyTestResult, LatencyTestError> {

    let result;
    {

        let socket = UdpSocket::bind("127.0.0.1:34254")?;
        socket.set_read_timeout(timeout).expect("Couldn't set socket timeout");
    
        let start = SystemTime::now();
        let time = start.duration_since(std::time::UNIX_EPOCH).expect("Couldn't get system time");
        let mut buf = (time.as_micros() as u64).to_be_bytes();
    
        let destination = SocketAddr::from(([127, 0, 0, 1], 34001));
    
        socket.send_to(&buf, destination)?;
    
        let (_message_length, _src) = socket.recv_from(&mut buf)?;
        let time_since_sent = start.elapsed().expect("Couldn't measure time").as_micros() as u64;
        let receiver_time = (std::time::Duration::from_micros(u64::from_be_bytes(buf)) - time).as_micros() as u64;

        result = LatencyTestResult {
            trip_time: receiver_time,
            round_trip_time: time_since_sent,
        };
        
    }  // the socket is closed here

    Result::Ok(result)
    
}
