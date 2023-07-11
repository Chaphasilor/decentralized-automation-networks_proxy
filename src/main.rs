use axum::{
    extract::{rejection::JsonRejection, Json, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Router,
};
use clap::Parser;
use db::Message;
use futures::future;
use serde_json::json;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::{net::UdpSocket, task};

use crate::node_red::flows;

pub mod db;
pub mod latency_test;
pub mod node_red;

// reqwest doesn't seem to support configuring a base URL, this is a workaround
#[derive(Clone)]
pub struct NodeRedHttpClient {
    pub client: reqwest::Client,
    pub base_url: String,
}

// helpers for combining path and base URL
impl NodeRedHttpClient {
    fn path_to_url(&self, path: &str) -> String {
        format!(
            "{}{}{}",
            self.base_url,
            if path.starts_with('/') { "" } else { "/" },
            path
        )
    }
    fn path_to_url_with_base_url(&self, path: &str, base_url: &str) -> String {
        format!(
            "{}{}{}",
            base_url,
            if path.starts_with('/') { "" } else { "/" },
            path
        )
    }
}

// state used by axum, contains everything that is potentially needed within endpoint
struct AppState {
    tx: mpsc::Sender<Message>,
    client: NodeRedHttpClient,
    config: Config,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct Area {
    name: String,
    proxy_ip: String,
    proxy_port_base: u16,
    proxy_webserver_port: u16,
    node_red_ip: String,
    node_red_port: u16,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct Node {
    name: String,
    area: String,
    flow: String,
    ip: String,
    port: u16,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct PortConfig {
    pub port_in_from_input_node_base: u16, // used by proxies and output nodes as inbound port
    pub port_out_to_node_red_base: u16,
    pub port_node_red_in_base: u16, // only used as a target
    port_node_red_out_base: u16,    // only used in Node-RED
    pub port_in_from_node_red_base: u16,
    pub port_out_to_proxy_or_output_node_base: u16,
    pub port_output_node_in_base: u16, // only used as a target
    pub port_range_limit: u16,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct Config {
    pub area: String,
    pub port_base: u16,
    pub webserver_port: u16,
    pub node_red_ip: String,
    pub node_red_port: String,
    pub ports: PortConfig,
    pub areas: Option<Vec<Area>>,
    pub input_nodes: Option<Vec<Node>>,
    pub output_nodes: Option<Vec<Node>>,
}

/// A management server and proxy for Node-RED
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Relative path to the configuration file
    #[arg(short, long)]
    config: String,
    #[arg(long)]
    pub latency_test: bool,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();

    let config = load_config(args.config.as_str()).unwrap();
    println!(
        "Proxy for area '{}' started!\nNode-RED instance running at {}:{}",
        config.area, config.node_red_ip, config.node_red_port
    );

    let mut tasks: Vec<tokio::task::JoinHandle<()>> = vec![];

    let mut default_headers = reqwest::header::HeaderMap::new();
    default_headers.insert(
        reqwest::header::HeaderName::from_static("node-red-api-version"),
        reqwest::header::HeaderValue::from_static("v2"),
    );

    // set up the HTTP client for communicating with Node-RED
    let node_red_http_client = NodeRedHttpClient {
        client: reqwest::Client::builder()
            .default_headers(default_headers)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        base_url: format!(
            "http://{}:{}",
            config.node_red_ip.clone(),
            config.node_red_port
        ),
    };

    // set up message passing senders. because the tokio tasks are implemented as closures, we need to clone the senders for each task
    let (tx, rx) = mpsc::channel::<Message>(32);
    let shared_state_tx = tx.clone();
    let node_receiver_tx = tx.clone();
    let proxy_receiver_tx = tx.clone();
    let latency_test_tx = tx.clone();

    let shared_state = Arc::new(AppState {
        tx: shared_state_tx,
        client: node_red_http_client.clone(),
        config: config.clone(),
    });

    let port_range = 1..=config.ports.port_range_limit;

    // db worker
    tasks.push(tokio::spawn(async move {
        match db::db_worker(rx).await {
            Ok(_) => {}
            Err(err) => {
                eprintln!("Error from database worker: {}", err);
            }
        };
    }));

    // create node receiver workers, handle forwarding results coming from Node-RED to the correct output node
    for port in port_range.clone() {
        let node_receiver_tx = node_receiver_tx.clone();

        // each worker gets its own pair of sockets, because we can't share sockets between threads
        let inbound_port = config.ports.port_in_from_node_red_base + port;
        let inbound_socket_address = SocketAddr::from(([0, 0, 0, 0], inbound_port));
        let inbound_socket = UdpSocket::bind(inbound_socket_address).await.unwrap();

        let outbound_port = config.ports.port_out_to_proxy_or_output_node_base + port;
        let outbound_socket_address = SocketAddr::from(([0, 0, 0, 0], outbound_port));
        let outbound_socket = UdpSocket::bind(outbound_socket_address).await.unwrap();

        let cloned_config = config.clone();

        tasks.push(tokio::spawn(async move {
            match node_red::proxy::udp_node_red_receiver(
                cloned_config,
                node_receiver_tx,
                inbound_socket,
                outbound_socket,
                config.ports.port_output_node_in_base,
            )
            .await
            {
                Ok(_) => {}
                Err(err) => {
                    eprintln!("Error from Node-RED UDP receiver: {}", err);
                }
            };
        }));
    }

    // create proxy receiver workers, handle forwarding messages coming from input nodes to Node-RED
    for port in port_range.clone() {
        let proxy_receiver_tx = proxy_receiver_tx.clone();

        // each worker gets its own pair of sockets, because we can't share sockets between threads
        let inbound_port = config.ports.port_in_from_input_node_base + port;
        let inbound_socket_address = SocketAddr::from(([0, 0, 0, 0], inbound_port));
        let inbound_socket = UdpSocket::bind(inbound_socket_address).await.unwrap();

        let outbound_port = config.ports.port_out_to_node_red_base + port;
        let outbound_socket_address = SocketAddr::from(([0, 0, 0, 0], outbound_port));
        let outbound_socket = UdpSocket::bind(outbound_socket_address).await.unwrap();

        let cloned_config = config.clone();

        tasks.push(tokio::spawn(async move {
            match node_red::proxy::udp_proxy_receiver(
                cloned_config,
                proxy_receiver_tx,
                inbound_socket,
                outbound_socket,
                config.ports.port_node_red_in_base,
            )
            .await
            {
                Ok(_) => {}
                Err(err) => {
                    eprintln!("Error from proxy UDP receiver: {}", err);
                }
            };
        }));
    }

    // perform latency test (requires `input_nodes` and `output_nodes` to be specified in the config)    
    if args.latency_test {
        tasks.push(tokio::spawn(async move {

            if config.input_nodes.is_some() && config.output_nodes.is_some() {
            
                for (device_name, device_ip, device_port) in config
                    .input_nodes
                    .as_ref()
                    .unwrap()
                    .iter()
                    .chain(config.output_nodes.as_ref().unwrap())
                    .map(|area| (area.name.clone(), area.ip.clone(), area.port))
                {
                    let destination = SocketAddr::from((
                        device_ip.parse::<std::net::Ipv4Addr>().unwrap(),
                        device_port,
                    ));
                    let test_result = latency_test::test_latency(destination, 10);

                    if let Err(err) = latency_test_tx
                        .send(Message {
                            message_type: db::MessageType::TimeOffset(db::TimeOffsetConfig {
                                identifier: db::DeviceIdentifier {
                                    device_ip,
                                    device_port,
                                },
                                offset: test_result.receiver_time_offset,
                            }),
                            response: None,
                        })
                        .await
                    {
                        eprintln!("Error sending time offset to database worker: {}", err);
                    }

                    println!("\n=== Test Result ===\n");
                    println!("Device: {}", device_name);
                    println!(
                        "Receiver Time Offset: {} µs",
                        test_result.receiver_time_offset
                    );
                    println!("Round-Trip-Time: {} µs", test_result.round_trip_time);
                    println!();

                    // latency_test::submit_test_result(&node_red_http_client, test_result)
                    //     .await
                    //     .unwrap();
                }

            } else {
                eprintln!("Latency test requires `input_nodes` and `output_nodes` to be specified in the config");
            }

        }));
    }

    // any blocking tasks could be performed in here
    tasks.push(task::spawn_blocking(move || {}));

    // set up a simple HTTP web server for controlling the proxy
    let app = Router::new()
        .route(
            "/proxy/nodeRedBaseUrl",
            get(proxy_node_red_base_url_handler),
        )
        .route("/db/log", get(log_db_handler))
        .route("/db/get", get(get_db_handler))
        .route("/db/save", get(save_db_handler))
        .route("/flows/updateStatus", put(update_flow_status_handler))
        .route("/flow/:flow_name", delete(delete_flow_handler))
        .route("/flows/transfer", post(transfer_flow_handler))
        .route("/flows/transfer", delete(untransfer_flow_handler))
        .route("/flows/analyze", post(analyze_flows_handler))
        .route("/flows/analyze", delete(untransfer_all_flows_handler))
        .with_state(shared_state); // pass the shared state to the router

    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    let addr = SocketAddr::from(([0, 0, 0, 0], config.webserver_port));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();

    // wait for all tasks to finish (should never happen, unless all tasks panic)
    future::join_all(tasks).await;

    Ok(())
}

fn load_config(path: &str) -> Result<Config, Box<dyn Error>> {
    let config = std::fs::read_to_string(path)?;

    let config = serde_yaml::from_str::<Config>(&config);

    println!("config: {:?}", config);

    config.map_err(|err| err.into())
}

// handlers for HTTP endpoints

async fn proxy_node_red_base_url_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (StatusCode::OK, Json(json!(state.client.base_url)))
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
    // get current ISO-8601 timestamp
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();

    if let Ok(_) = state
        .tx
        .send(db::Message {
            message_type: db::MessageType::SaveDB(format!(
                "./data/{timestamp}_dump_{area}.json",
                timestamp = timestamp,
                area = state.config.area
            )),
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
struct UpdateFlowStatusPayload {
    name: String,
    disabled: bool,
}
async fn update_flow_status_handler(
    State(state): State<Arc<AppState>>,
    payload: Result<Json<UpdateFlowStatusPayload>, JsonRejection>,
) -> impl IntoResponse {
    match payload {
        Ok(payload) => {
            // We got a valid JSON payload

            let flows = flows::convert_flows_response_to_flows(
                flows::get_all_flows(&state.client).await.unwrap(),
            );

            if let Some(flow_id) = flows::get_flow_id_by_name(&flows, payload.name.as_str()) {
                if let Err(err) =
                    flows::update_flow_status(&state.client, &flow_id, payload.disabled).await
                {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "message": format!("Flow could not be {}", if payload.disabled {"disabled"} else {"enabled"}),
                            "error": err.to_string()
                        })),
                    )
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
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": format!("Missing fields: {}", err),
                })),
            )
        }
        Err(JsonRejection::JsonSyntaxError(_)) => {
            // Syntax error in the body
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": "Invalid JSON"
                })),
            )
        }
        Err(_) => {
            // `JsonRejection` is marked `#[non_exhaustive]` so match must
            // include a catch-all case.
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "message": "Unknown error"
                })),
            )
        }
    }
}

#[derive(serde::Deserialize)]
struct TransferFlowPayload {
    name: String,
    #[serde(rename = "newArea")]
    new_area: String,
}
async fn transfer_flow_handler(
    State(state): State<Arc<AppState>>,
    payload: Result<Json<TransferFlowPayload>, JsonRejection>,
) -> impl IntoResponse {
    match payload {
        Ok(payload) => {
            // We got a valid JSON payload

            let mut flows = flows::convert_flows_response_to_flows(
                flows::get_all_flows(&state.client).await.unwrap(),
            );

            if let Some(flow_id) = flows::get_flow_id_by_name(&flows, payload.name.as_str()) {
                if let Err(err) = flows::transfer_flow_to_area(
                    &mut flows,
                    &state.config,
                    &state.client,
                    &flow_id,
                    &payload.new_area,
                )
                .await
                {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "message": format!("Flow could not be transferred to area '{}'", payload.new_area),
                            "error": err.to_string()
                        })),
                    )
                } else {
                    (
                        StatusCode::OK,
                        Json(json!({
                            "message": format!("Flow successfully transferred to area '{}'", payload.new_area),
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
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": format!("Missing fields: {}", err),
                })),
            )
        }
        Err(JsonRejection::JsonSyntaxError(_)) => {
            // Syntax error in the body
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": "Invalid JSON"
                })),
            )
        }
        Err(_) => {
            // `JsonRejection` is marked `#[non_exhaustive]` so match must
            // include a catch-all case.
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "message": "Unknown error"
                })),
            )
        }
    }
}

#[derive(serde::Deserialize)]
struct UntransferFlowPayload {
    name: String,
    area: String,
}
async fn untransfer_flow_handler(
    State(state): State<Arc<AppState>>,
    payload: Result<Json<UntransferFlowPayload>, JsonRejection>,
) -> impl IntoResponse {
    match payload {
        Ok(payload) => {
            // We got a valid JSON payload

            let flows = flows::convert_flows_response_to_flows(
                flows::get_all_flows(&state.client).await.unwrap(),
            );

            if let Some(flow_id) = flows::get_flow_id_by_name(&flows, payload.name.as_str()) {
                if let Err(err) = flows::untransfer_flow_from_area(
                    &state.config,
                    &state.client,
                    &flow_id,
                    &payload.area,
                )
                .await
                {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "message": format!("Flow could not be untransferred from area '{}'", payload.area),
                            "error": err.to_string()
                        })),
                    )
                } else {
                    (
                        StatusCode::OK,
                        Json(json!({
                            "message": format!("Flow successfully untransferred from area '{}'", payload.area),
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
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": format!("Missing fields: {}", err),
                })),
            )
        }
        Err(JsonRejection::JsonSyntaxError(_)) => {
            // Syntax error in the body
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": "Invalid JSON"
                })),
            )
        }
        Err(_) => {
            // `JsonRejection` is marked `#[non_exhaustive]` so match must
            // include a catch-all case.
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "message": "Unknown error"
                })),
            )
        }
    }
}

async fn delete_flow_handler(
    State(state): State<Arc<AppState>>,
    Path(params): Path<std::collections::HashMap<String, String>>,
    // payload: Result<Json<DeleteFlowPayload>, JsonRejection>
) -> impl IntoResponse {
    match params.get("flow_name") {
        Some(flow_name) => {
            // We got a valid JSON payload

            let flows = flows::convert_flows_response_to_flows(
                flows::get_all_flows(&state.client).await.unwrap(),
            );

            if let Some(flow_id) = flows::get_flow_id_by_name(&flows, flow_name.as_str()) {
                if let Err(err) = flows::delete_flow(&state.client, &flow_id).await {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "message": format!("Flow could not be deleted"),
                            "error": err.to_string()
                        })),
                    )
                } else {
                    (
                        StatusCode::OK,
                        Json(json!({
                            "message": format!("Flow successfully deleted"),
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
        None => {
            // `JsonRejection` is marked `#[non_exhaustive]` so match must
            // include a catch-all case.
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": "Missing flow name"
                })),
            )
        }
    }
}

#[derive(serde::Deserialize)]
struct AnalyzeFlowsPayload {
    dry_run: Option<bool>,
}
async fn analyze_flows_handler(
    State(state): State<Arc<AppState>>,
    payload: Result<Json<AnalyzeFlowsPayload>, JsonRejection>,
) -> impl IntoResponse {
    match payload {
        Ok(payload) => {
            match flows::analyze_flows(&state.config, &state.client, payload.dry_run).await {
                Ok(analysis) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(analysis).unwrap_or(json!({
                        "message": "Flows analyzed",
                        "error": "Could not serialize analysis"
                    }))),
                ),
                Err(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "message": "Flows could not be analyzed",
                        "error": err.to_string()
                    })),
                ),
            }
        }
        Err(JsonRejection::JsonDataError(err)) => {
            // Couldn't deserialize the body into the target type
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": format!("Missing fields: {}", err),
                })),
            )
        }
        Err(JsonRejection::JsonSyntaxError(_)) => {
            // Syntax error in the body
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": "Invalid JSON"
                })),
            )
        }
        Err(_) => {
            // `JsonRejection` is marked `#[non_exhaustive]` so match must
            // include a catch-all case.
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "message": "Unknown error"
                })),
            )
        }
    }
}

#[derive(serde::Deserialize)]
struct UntransferAllFlowsPayload {
    dry_run: Option<bool>,
}
async fn untransfer_all_flows_handler(
    State(state): State<Arc<AppState>>,
    payload: Result<Json<UntransferAllFlowsPayload>, JsonRejection>,
) -> impl IntoResponse {
    match payload {
        Ok(payload) => {
            match flows::untransfer_all_flows(&state.config, &state.client, payload.dry_run).await {
                Ok(analysis) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(analysis).unwrap_or(json!({
                        "message": "Flows untransferred",
                        "error": "Could not serialize analysis"
                    }))),
                ),
                Err(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "message": "Flows could not be untransferred",
                        "error": err.to_string()
                    })),
                ),
            }
        }
        Err(JsonRejection::JsonDataError(err)) => {
            // Couldn't deserialize the body into the target type
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": format!("Missing fields: {}", err),
                })),
            )
        }
        Err(JsonRejection::JsonSyntaxError(_)) => {
            // Syntax error in the body
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "message": "Invalid JSON"
                })),
            )
        }
        Err(_) => {
            // `JsonRejection` is marked `#[non_exhaustive]` so match must
            // include a catch-all case.
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "message": "Unknown error"
                })),
            )
        }
    }
}
