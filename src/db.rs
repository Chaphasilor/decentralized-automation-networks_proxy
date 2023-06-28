use std::collections::HashMap;
use std::{error::Error, fmt};
use std::time::{SystemTime};
use serde::{Serialize, Deserialize};
use tokio::sync::{mpsc, oneshot};

pub type Db = HashMap<String, Vec<Event>>;

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

#[derive(Clone, Serialize, Deserialize)]
pub struct EventIdentifier {
  pub flow_name: String,
  pub execution_area: String,
}

impl EventIdentifier {
  pub fn to_string(&self) -> String {
    format!("{flow_name}@{area}", flow_name = self.flow_name, area = self.execution_area)
  }
}

impl fmt::Display for EventIdentifier {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      write!(f, "{flow_name}@{area}", flow_name = self.flow_name, area = self.execution_area)
  }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Event {
  pub source: Option<std::net::SocketAddr>,
  pub destination: Option<std::net::SocketAddr>,
  pub timestamp: SystemTime,
  pub identifier: EventIdentifier,
}

#[derive(Clone)]
pub enum MessageType {
  LogDB,
  GetDB,
  SaveDB(String),
  Event(Event),
}

pub struct Message {
  pub message_type: MessageType,
  pub response: Option<oneshot::Sender<Result<serde_json::Value, serde_json::Value>>>,
}

pub async fn db_worker(mut rx: mpsc::Receiver<Message>) -> Result<(), DbWorkerError> {

  let mut db: Db = HashMap::new();

  while let Some(message) = rx.recv().await {

    match message.message_type {
      MessageType::LogDB => {
        log_database(&db);
        continue;
      },
      MessageType::GetDB => {
        if let Some(response) = message.response {
          response.send(Ok(get_database(&db))).unwrap();
        }
        continue;
      },
      MessageType::SaveDB(path) => {
        if let Err(err) = save_database(&db, path.as_str()) {
          eprintln!("Couldn't save database to file: {err}");
        }
        continue;
      },
      MessageType::Event(e) => {

        let timestamp = e.timestamp.duration_since(std::time::UNIX_EPOCH).unwrap();
    
        if !db.contains_key(&e.identifier.to_string()) {
          db.insert(e.identifier.clone().to_string(), Vec::new());
        }
    
        if let Some(entries) = db.get_mut(&e.identifier.to_string()) {
          if e.source.is_some() {
            println!("DB worker received event: id={id} source={source}, timestamp={timestamp}", id = e.identifier, source = e.source.unwrap().to_string(), timestamp = timestamp.as_secs());
            entries.push(e);
          } else if e.destination.is_some() {
            println!("DB worker received event: id={id} destination={destination}, timestamp={timestamp}", id = e.identifier, destination = e.destination.unwrap().to_string(), timestamp = timestamp.as_secs());
            entries.push(e);
          } else {
            return Err(DbWorkerError{message: "Event neither contained `source` nor `destination`!".to_string()});
          }
        }

      },
    }

  }

  Ok(())
  
}

/*
 * Serializes the database to JSON
 */
pub fn get_database(db: &Db) -> serde_json::Value {
  // serde_json::to_value(db).unwrap_or(json!({
  //   "error": "Couldn't serialize database to JSON"
  // }))
  serde_json::to_value(db).unwrap()
}

/*
 * Pretty-prints the database to stdout
 * The timings between events are also printed in microseconds
 */
pub fn log_database(db: &Db) {
  println!("Database contents:");
  for (identifier, events) in db {
    println!("  {id}:", id = identifier);
    let mut previous_timestamp: Option<u128> = None;
    for event in events {
      //TODO annotate the kind of actions that took place
      let timestamp = event.timestamp.duration_since(std::time::UNIX_EPOCH).unwrap().as_micros();
      if previous_timestamp.is_some() {
        println!("    {timestamp}µs", timestamp = timestamp - previous_timestamp.unwrap());
      }
      previous_timestamp = Some(timestamp);
    }
  }
}

pub fn save_database(db: &Db, path: &str) -> Result<(), Box<dyn Error>> {

  // check if path exists, otherwise create it
  let path = std::path::Path::new(path);
  if !path.exists() {
    std::fs::create_dir_all(path.parent().unwrap())?;
  }
  
  let file = std::fs::File::create(path)?;

  // serialize `db` to `file`
  serde_json::to_writer(file, db)?;

  Ok(())
}
