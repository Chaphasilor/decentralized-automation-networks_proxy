use std::{error::Error, fmt};
use std::time::{SystemTime};
use std::net::{UdpSocket, SocketAddr};

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
        LatencyTestError {
            kind: String::from("io"),
            message: error.to_string(),
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
    
    Ok(())
}

fn test_latency(iterations: usize) -> LatencyTestResult {

    let mut result = LatencyTestResult {
        trip_time: 0,
        round_trip_time: 0,
    };
    
    let mut successful_iterations = 0;
    
    for i in 0..iterations {
        print!("Iteration {i}: ");
        match test_latency_once() {
            Ok(current_result ) => {
                print!("TT: {} µs, ", current_result.trip_time);
                print!("TTL: {} µs", current_result.round_trip_time);
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

fn test_latency_once() -> Result<LatencyTestResult, LatencyTestError> {

    let result;
    {

        let socket = UdpSocket::bind("127.0.0.1:34254")?;
    
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
