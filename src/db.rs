use std::collections::HashMap;
use std::{error::Error, fmt};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, Duration};
use tokio::sync::mpsc;

pub type Db = HashMap<String, Vec<SystemTime>>;

#[derive(Debug)]
pub struct DbWorkerError {
    message: String,
}

impl Error for DbWorkerError {}

impl fmt::Display for DbWorkerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{message}", message = self.message)
    }
}

pub struct Event {
  pub source: Option<String>,
  pub destination: Option<String>,
  pub timestamp: SystemTime,
}

pub async fn db_worker(mut rx: mpsc::Receiver<Event>) -> Result<(), DbWorkerError> {

  let db: Db = HashMap::new();


  while let Some(message) = rx.recv().await {

    let timestamp = message.timestamp.duration_since(std::time::UNIX_EPOCH).unwrap();

    if message.source.is_some() {
      println!("DB worker received event: source={source}, timestamp={timestamp}", source = message.source.unwrap(), timestamp = timestamp.as_secs());
    } else if message.destination.is_some() {
      println!("DB worker received event: destination={destination}, timestamp={timestamp}", destination = message.destination.unwrap(), timestamp = timestamp.as_secs());
    } else {
      return Err(DbWorkerError{message: "Event neither contained `source` nor `destination`!".to_string()});
    }

  }

  Ok(())
  
}
