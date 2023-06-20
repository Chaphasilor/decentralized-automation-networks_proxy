use axum::{
    extract::{State, Json, rejection::JsonRejection},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post, put},
    Router,
};
use db::{Event, Message};
use futures::{future, Future};
use serde_json::json;
use serde_yaml;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use std::{error::Error, fmt};
use tokio::sync::{mpsc, oneshot};
use tokio::{net::UdpSocket, task};
use clap::Parser;

use crate::node_red::flows;

pub mod db;
pub mod latency_test;
pub mod node_red;

#[derive(Clone)]
pub struct NodeRedHttpClient {
    pub client: reqwest::Client,
    pub base_url: String,
}

impl NodeRedHttpClient {
    fn path_to_url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path)
    }
}

struct AppState {
    tx: mpsc::Sender<Message>,
    client: NodeRedHttpClient,
}

#[derive(serde::Deserialize, Debug)]
struct Area {
    name: String,
    proxy_ip: String,
}

#[derive(serde::Deserialize, Debug)]
struct InputNode {
    area: String,
    name: String,
    ip: String,
    port: u16,
}

#[derive(serde::Deserialize, Debug)]
struct PortConfig {
    port_in_from_input_node_base: u16, // used by proxies and output nodes as inbound port
    port_out_to_node_red_base: u16,
    port_node_red_in_base: u16, // only used as a target
    port_node_red_out_base: u16, // only used in Node-RED
    port_in_from_node_red_base: u16,
    port_out_to_proxy_or_output_node_base: u16,
    port_range_limit: u16,
}

#[derive(serde::Deserialize, Debug)]
struct Config {
    area: String,
    node_red_base_url: String,
    ports: PortConfig,
    areas: Option<Vec<Area>>,
    input_nodes: Option<Vec<InputNode>>,
}

/// A management server and proxy for Node-RED
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Relative path to the configuration file
    #[arg(short, long)]
    config: String,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {

    let args = Args::parse();

    let config = load_config(args.config.as_str()).unwrap();
    println!("Proxy for area '{}' started!\nNode-RED instance running at {}", config.area, config.node_red_base_url);
    
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = vec![];

    let mut default_headers = reqwest::header::HeaderMap::new();
    default_headers.insert(reqwest::header::HeaderName::from_static("node-red-api-version"), reqwest::header::HeaderValue::from_static("v2"));

    let node_red_http_client = NodeRedHttpClient{
        client: reqwest::Client::builder()
            .default_headers(default_headers)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        base_url: config.node_red_base_url.clone(),
    };

    let (tx, mut rx) = mpsc::channel::<Message>(32);
    let local_tx = tx.clone();
    let shared_state_tx = tx.clone();
    let node_receiver_tx = tx.clone();
    let proxy_receiver_tx = tx.clone();

    let shared_state = Arc::new(AppState {
        tx: shared_state_tx,
        client: node_red_http_client.clone(),
    });

    let port_range = 1..=config.ports.port_range_limit;

    // db
    tasks.push(tokio::spawn(async move {
        match db::db_worker(rx).await {
            Ok(_) => {}
            Err(err) => {
                eprintln!("Error from database worker: {}", err.to_string());
            }
        };
    }));

    for port in port_range.clone() {

        let node_receiver_tx = node_receiver_tx.clone();

        let inbound_port = config.ports.port_in_from_node_red_base + port;
        let inbound_socket_address = SocketAddr::from(([127, 0, 0, 1], inbound_port)); 
        let inbound_socket = UdpSocket::bind(inbound_socket_address).await.unwrap();

        let outbound_port = config.ports.port_out_to_proxy_or_output_node_base + port;
        let outbound_socket_address = SocketAddr::from(([127, 0, 0, 1], outbound_port));
        let outbound_socket = UdpSocket::bind(outbound_socket_address).await.unwrap();
        
        tasks.push(tokio::spawn(async move {
            match node_red::proxy::udp_node_red_receiver(node_receiver_tx, inbound_socket, outbound_socket, config.ports.port_in_from_input_node_base).await {
                Ok(_) => {}
                Err(err) => {
                    eprintln!("Error from Node-RED UDP receiver: {}", err.to_string());
                }
            };
        }));
    }

    for port in port_range.clone() {

        let proxy_receiver_tx = proxy_receiver_tx.clone();

        let inbound_port = config.ports.port_in_from_input_node_base + port;
        let inbound_socket_address = SocketAddr::from(([127, 0, 0, 1], inbound_port));
        let inbound_socket = UdpSocket::bind(inbound_socket_address).await.unwrap();

        let outbound_port = config.ports.port_out_to_node_red_base + port;
        let outbound_socket_address = SocketAddr::from(([127, 0, 0, 1], outbound_port));
        let outbound_socket = UdpSocket::bind(outbound_socket_address).await.unwrap();

        tasks.push(tokio::spawn(async move {
            match node_red::proxy::udp_proxy_receiver(proxy_receiver_tx, inbound_socket, outbound_socket, config.ports.port_node_red_in_base).await {
                Ok(_) => {}
                Err(err) => {
                    eprintln!("Error from proxy UDP receiver: {}", err.to_string());
                }
            };
        }));
    }


    tasks.push(tokio::spawn(async move {
        let test_result = latency_test::test_latency(10);

        println!("\n=== Test Result ===\n");
        println!("One-way latency: {} µs", test_result.trip_time);
        println!("Round-Trip-Time: {} µs", test_result.round_trip_time);
        println!();

        latency_test::submit_test_result(&node_red_http_client, test_result).await.unwrap();

        let flows = node_red::flows::convert_flows_response_to_flows(
            node_red::flows::get_all_flows(&node_red_http_client).await.unwrap(),
        );
        println!("flows: {}", serde_json::to_string(&flows).unwrap());

        let test_socket = UdpSocket::bind("127.0.0.1:34999").await.unwrap();

        if let Err(err) = node_red::proxy::forward_message_to_node_red(
            &test_socket,
            config.ports.port_node_red_in_base + 999,
            json!({
                "message": "hi mom",
                "meta": {
                    "flow_name": "Flow 0",
                    "execution_area": "room0"
                }
            }),
            Some(Duration::from_millis(250)),
            local_tx.clone(),
        ).await {
            eprintln!("Error while forwarding message: {}", err.to_string());
        };
    }));

    tasks.push(task::spawn_blocking(move || {

    }));

    // set up a simple HTTP web server for controlling the proxy
    let app = Router::new()
        .route("/db/log", get(log_db_handler))
        .route("/db/get", get(get_db_handler))
        .route("/db/save", get(save_db_handler))
        .route("/flows/updateStatus", put(update_flow_status_handler))
        .route("/flows/transfer", post(transfer_flow_handler))
        .with_state(shared_state);

    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();

    future::join_all(tasks).await;

    Ok(())
}

fn load_config(path: &str) -> Result<Config, Box<dyn Error>> {
    let config = std::fs::read_to_string(path)?;

    let config = serde_yaml::from_str::<Config>(&config);

    println!("config: {:?}", config);

    config.map_err(|err| err.into())
}

async fn log_db_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Ok(_) = state
        .tx
        .send(db::Message {
            message_type: db::MessageType::LogDB,
            response: None,
        })
        .await
    {
        (
            StatusCode::OK,
            Json(json!({
                "message": "Database logged"
            })),
        )
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "message": "Database could not be logged"
            })),
        )
    }
}

async fn save_db_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Ok(_) = state
        .tx
        .send(db::Message {
            message_type: db::MessageType::SaveDB("./data/db.json".to_string()),
            response: None,
        })
        .await
    {
        (
            StatusCode::OK,
            Json(json!({
                "message": "Database saved"
            })),
        )
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "message": "Database could not be saved"
            })),
        )
    }
}

async fn get_db_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let (response_tx, response_rx) = oneshot::channel();

    if let Ok(_) = state
        .tx
        .send(db::Message {
            message_type: db::MessageType::GetDB,
            response: Some(response_tx),
        })
        .await
    {
        if let Ok(response) = response_rx.await {
            match response {
                Ok(db) => (
                    StatusCode::OK,
                    Json(json!({
                        "message": "Database retrieved",
                        "db": db
                    })),
                ),
                Err(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "message": "Database could not be retrieved",
                        "error": err.to_string()
                    })),
                ),
            }
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "message": "Database could not be retrieved"
                })),
            )
        }
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "message": "Database could not be retrieved"
            })),
        )
    }
}

#[derive(serde::Deserialize)]
struct UpdateFlowStatusPayload{
    name: String,
    disabled: bool,
}
async fn update_flow_status_handler(
    State(state): State<Arc<AppState>>,
    payload: Result<Json<UpdateFlowStatusPayload>, JsonRejection>
) -> impl IntoResponse {

    match payload {
        Ok(payload) => {
            // We got a valid JSON payload

            let flows = flows::convert_flows_response_to_flows(flows::get_all_flows(&state.client).await.unwrap());
            
            if let Some(flow_id) = flows::get_flow_id_by_name(&flows, payload.name.as_str()) {
                if let Err(err) = flows::update_flow_status(&state.client, &flow_id, payload.disabled).await {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "message": format!("Flow could not be {}", if payload.disabled {"disabled"} else {"enabled"}),
                            "error": err.to_string()
                        })),
                    );
                } else {
                    (
                        StatusCode::OK,
                        Json(json!({
                            "message": format!("Flow {}", if payload.disabled {"disabled"} else {"enabled"}),
                        })),
                    )
                }
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "message": "Flow not found"
                    })),
                )
            }
            
        }
        Err(JsonRejection::JsonDataError(err)) => {
            // Couldn't deserialize the body into the target type
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": format!("Missing fields: {}", err.to_string()), 
                })),
            )
        }
        Err(JsonRejection::JsonSyntaxError(_)) => {
            // Syntax error in the body
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": "Invalid JSON"
                })),
            )
        }
        Err(_) => {
            // `JsonRejection` is marked `#[non_exhaustive]` so match must
            // include a catch-all case.
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "message": "Unknown error"
                })),
            )
        }
    }
    
}

#[derive(serde::Deserialize)]
struct TransferFlowPayload{
    name: String,
    newArea: String,
}
async fn transfer_flow_handler(
    State(state): State<Arc<AppState>>,
    payload: Result<Json<TransferFlowPayload>, JsonRejection>
) -> impl IntoResponse {

    match payload {
        Ok(payload) => {
            // We got a valid JSON payload

            let flows = flows::convert_flows_response_to_flows(flows::get_all_flows(&state.client).await.unwrap());
            
            if let Some(flow_id) = flows::get_flow_id_by_name(&flows, payload.name.as_str()) {
                if let Err(err) = flows::transfer_flow_to_area(&state.client, &flow_id, &payload.newArea).await {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "message": format!("Flow could not be transferred to area '{}'", payload.newArea),
                            "error": err.to_string()
                        })),
                    );
                } else {
                    (
                        StatusCode::OK,
                        Json(json!({
                            "message": format!("Flow successfully transferred to area '{}'", payload.newArea),
                        })),
                    )
                }
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "message": "Flow not found"
                    })),
                )
            }
            
        }
        Err(JsonRejection::JsonDataError(err)) => {
            // Couldn't deserialize the body into the target type
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": format!("Missing fields: {}", err.to_string()), 
                })),
            )
        }
        Err(JsonRejection::JsonSyntaxError(_)) => {
            // Syntax error in the body
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": "Invalid JSON"
                })),
            )
        }
        Err(_) => {
            // `JsonRejection` is marked `#[non_exhaustive]` so match must
            // include a catch-all case.
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "message": "Unknown error"
                })),
            )
        }
    }
    
}
