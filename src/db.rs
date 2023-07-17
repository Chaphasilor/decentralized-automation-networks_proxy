use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;
use std::{error::Error, fmt};
use tokio::sync::{mpsc, oneshot};

pub type Db = HashMap<String, Vec<Event>>;
pub type TimeOffsetsDb = HashMap<String, i128>;

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

impl fmt::Display for EventIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{flow_name}@{area}",
            flow_name = self.flow_name,
            area = self.execution_area
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DeviceIdentifier {
    pub device_ip: String,
    pub device_port: u16,
}

impl fmt::Display for DeviceIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{device_ip}:{device_port}",
            device_ip = self.device_ip,
            device_port = self.device_port
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum EventHandler {
    ProxyReceiver,
    NodeRedForwarder,
    NodeRedReceiver,
    ProxyForwarder,
}

impl ToString for EventHandler {
    fn to_string(&self) -> String {
        match self {
            EventHandler::ProxyReceiver => "ProxyReceiver".to_string(),
            EventHandler::NodeRedForwarder => "NodeRedForwarder".to_string(),
            EventHandler::NodeRedReceiver => "NodeRedReceiver".to_string(),
            EventHandler::ProxyForwarder => "ProxyForwarder".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Event {
    pub source: Option<std::net::SocketAddr>,
    pub destination: Option<std::net::SocketAddr>,
    pub timestamp: SystemTime,
    pub identifier: EventIdentifier,
    pub handler: EventHandler,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TimeOffsetConfig {
    pub identifier: DeviceIdentifier,
    pub offset: i128,
}

#[derive(Clone)]
pub enum MessageType {
    LogDB,
    GetDB,
    SaveDB(String),
    Event(Event),
    TimeOffset(TimeOffsetConfig),
}

pub struct Message {
    pub message_type: MessageType,
    pub response: Option<oneshot::Sender<Result<serde_json::Value, serde_json::Value>>>,
}

pub async fn db_worker(mut rx: mpsc::Receiver<Message>) -> Result<(), DbWorkerError> {
    let mut db: Db = Db::new();
    let mut time_offsets_db = TimeOffsetsDb::new();

    // receive messages from the inter-thread communication channel
    while let Some(message) = rx.recv().await {
        match message.message_type {
            MessageType::LogDB => {
                log_database(&db);
            }
            MessageType::GetDB => {
                if let Some(response) = message.response {
                    response.send(Ok(get_database(&db))).unwrap();
                }
            }
            MessageType::SaveDB(path) => {
                if let Err(err) = save_database(&db, path.as_str()) {
                    eprintln!("Couldn't save database to file: {err}");
                }
            }
            MessageType::Event(e) => {
                let timestamp = e.timestamp.duration_since(std::time::UNIX_EPOCH).unwrap();

                if !db.contains_key(&e.identifier.to_string()) {
                    db.insert(e.identifier.clone().to_string(), Vec::new());
                }

                if let Some(entries) = db.get_mut(&e.identifier.to_string()) {
                    if e.source.is_some() {
                        println!("DB worker received event: [{handler}] id={id} source={source}, timestamp={timestamp}", handler=e.handler.to_string(), id = e.identifier, source = e.source.unwrap(), timestamp = timestamp.as_secs());
                        entries.push(e);
                    } else if e.destination.is_some() {
                        println!("DB worker received event: [{handler}] id={id} destination={destination}, timestamp={timestamp}", handler=e.handler.to_string(), id = e.identifier, destination = e.destination.unwrap(), timestamp = timestamp.as_secs());
                        entries.push(e);
                    } else {
                        return Err(DbWorkerError {
                            message: "Event neither contained `source` nor `destination`!"
                                .to_string(),
                        });
                    }
                }
            }
            MessageType::TimeOffset(e) => {
                println!(
                    "TimeOffsets worker received event: id={id}, offset={offset}",
                    id = e.identifier,
                    offset = e.offset
                );
                time_offsets_db.insert(e.identifier.to_string(), e.offset);
            }
        }
    }

    Ok(())
}

/*
 * Serializes the database to JSON
 */
pub fn get_database(db: &Db) -> serde_json::Value {
    // serde_json::to_value(db).unwrap()
    serde_json::to_value(db).unwrap_or(serde_json::json!({
      "error": "Couldn't serialize database to JSON"
    }))
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
            let timestamp = event
                .timestamp
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_micros();
            if previous_timestamp.is_some() {
                println!(
                    "    {timestamp}µs",
                    timestamp = timestamp - previous_timestamp.unwrap()
                );
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
