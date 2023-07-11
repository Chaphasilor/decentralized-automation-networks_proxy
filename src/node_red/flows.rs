use itertools::Itertools;
use local_ip_address::local_ip;
use serde::{ser::SerializeStruct, Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::{collections::HashMap, net::SocketAddr, time::Duration};
use std::{error::Error, fmt};
use tokio::time::timeout;

use crate::{Config, NodeRedHttpClient};

// model Node-RED API responses

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FlowNodeResponse {
    id: String,
    #[serde(rename = "type")]
    _type: String,
    label: Option<String>,
    disabled: Option<bool>,
    info: Option<String>,
    configs: Option<Vec<serde_json::Value>>,
    env: Option<Vec<String>>,
    x: Option<u32>,
    y: Option<u32>,
    z: Option<String>,
    wires: Option<Vec<Vec<String>>>,
    props: Option<Vec<HashMap<String, String>>>,
    repeat: Option<String>,
    crontab: Option<String>,
    once: Option<bool>,
    #[serde(rename = "onceDelay")]
    once_delay: Option<f32>,
    topic: Option<String>,
    payload: Option<String>,
    #[serde(rename = "payloadType")]
    payload_type: Option<String>,
    name: Option<String>,
    func: Option<String>,
    outputs: Option<u32>,
    noerr: Option<u32>,
    initialize: Option<String>,
    finalize: Option<String>,
    libs: Option<Vec<String>>,
    method: Option<String>,
    ret: Option<String>,
    paytoqs: Option<String>,
    url: Option<String>,
    field: Option<String>,
    #[serde(rename = "fieldType")]
    field_type: Option<String>,
    template: Option<String>,
    iface: Option<String>,
    port: Option<String>,
    outport: Option<String>,
    ipv: Option<String>,
    multicast: Option<String>,
    group: Option<String>,
    datatype: Option<String>,
    syntax: Option<String>,
    output: Option<String>,
    addr: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FlowsResponse {
    flows: Vec<FlowNodeResponse>,
    rev: String,
}
#[derive(Debug, Clone)]
pub struct Flow {
    nodes: Vec<FlowNodeResponse>,
    id: String,
    name: Option<String>,
    disabled: bool,
    configs: Vec<serde_json::Value>,
    input_area: Option<String>,
    output_area: Option<String>,
}

// custom serializer to remove input and output areas (they are only used internally)
impl Serialize for Flow {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("Flow", 5)?;
        state.serialize_field("nodes", &self.nodes)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("label", &self.name)?;
        state.serialize_field("configs", &self.configs)?;
        state.serialize_field("disabled", &self.disabled)?;
        state.end()
    }
}

#[derive(Debug, Serialize)]
pub struct Flows {
    pub flows: HashMap<String, Flow>,
    rev: String,
}

#[derive(Debug)]
pub struct FlowError {
    message: String,
}

impl Error for FlowError {}

impl fmt::Display for FlowError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{message}", message = self.message)
    }
}

impl FlowError {
    pub fn new(message: String) -> Self {
        FlowError { message }
    }
}

#[derive(Debug, Serialize)]
pub struct FlowTransferResult {
    pub flow_id: String,
    pub flow_name: Option<String>,
    pub new_flow_name: Option<String>,
    pub old_area: String,
    pub new_area: String,
    pub transfer_successful: bool,
}

#[derive(Debug, Serialize)]
pub struct AnalysisResult {
    analyzed_flows: Vec<String>,
    transferred_flows: Vec<FlowTransferResult>,
}

pub async fn get_all_flows(
    client: &NodeRedHttpClient,
) -> Result<FlowsResponse, Box<dyn std::error::Error>> {
    let request = client.client.get(client.path_to_url("/flows")).send();

    match request.await {
        Ok(response) => match response.json::<FlowsResponse>().await {
            Ok(flows_response) => Ok(flows_response),
            Err(err) => {
                eprintln!(
                    "Couldn't deserialize flows response: {}\n{:?}",
                    err.source().unwrap(),
                    err
                );
                Err(Box::new(err))
            }
        },
        Err(err) => {
            eprintln!(
                "Couldn't get flows from Node-RED: {}\n{:?}",
                err.source().unwrap(),
                err
            );
            Err(Box::new(err))
        }
    }
}

/// parse Node-RED API response into a more convenient format
/// also determines input and output areas for each flow
pub fn convert_flows_response_to_flows(response: FlowsResponse) -> Flows {
    let mut flows = Flows {
        flows: HashMap::<String, Flow>::new(),
        rev: response.rev,
    };

    response.flows[..]
        .iter()
        .filter(|flow_node| flow_node._type == "tab")
        .for_each(|flow_node| {
            let new_flow = Flow {
                // nodes: vec![flow_node.clone()],
                nodes: vec![],
                id: flow_node.id.to_string(),
                name: flow_node.label.clone(),
                disabled: flow_node.disabled.unwrap_or(false),
                configs: if flow_node.configs.is_some() {
                    flow_node.configs.clone().unwrap()
                } else {
                    vec![]
                },
                input_area: None,
                output_area: None,
            };
            flows.flows.insert(new_flow.id.to_string(), new_flow);
        });

    for flow_node in response.flows[..]
        .iter()
        .filter(|flow_node| flow_node._type != "tab")
        .collect::<Vec<&FlowNodeResponse>>()
    {
        if let Some(z) = flow_node.z.clone() {
            if let Some(flow) = flows.flows.get_mut(&z) {
                flow.nodes.push(flow_node.clone());
                if flow_node._type == "template" && flow_node.template.is_some() {
                    if let Some(field) = &flow_node.field {
                        match field.as_str() {
                            "input_area" => flow.input_area = flow_node.template.clone(),
                            "output_area" => flow.output_area = flow_node.template.clone(),
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    flows
}

pub fn get_flow_by_id(flows: &Flows, id: &str) -> Option<Flow> {
    flows.flows.get(id).cloned()
}

pub fn get_flow_id_by_name(flows: &Flows, name: &str) -> Option<String> {
    flows
        .flows
        .iter()
        .find(|(_, flow)| {
            if let Some(flow_name) = &flow.name {
                flow_name == name
            } else {
                false
            }
        })
        .map(|(id, _)| id.to_string())
}

pub async fn update_flow_status(
    client: &NodeRedHttpClient,
    id: &str,
    is_disabled: bool,
) -> Result<(), FlowError> {
    let flows = convert_flows_response_to_flows(get_all_flows(client).await.unwrap());
    let mut flows = flows.flows;

    if let Some(flow) = flows.get_mut(id) {
        flow.disabled = is_disabled;

        // println!("updated flow: {}", serde_json::to_string(&flow).unwrap());

        // push changes to Node-RED asynchronously
        let request = client
            .client
            .put(client.path_to_url(format!("/flow/{id}", id = id).as_str()))
            .json(&flow);
        match request.send().await {
            Ok(_response) => Ok(()),
            Err(err) => {
                eprintln!(
                    "Couldn't update status of flow with id {}: {}\n{:?}",
                    id,
                    err.source().unwrap(),
                    err
                );
                Err(FlowError::new(format!(
                    "Couldn't update status of flow with id {}",
                    id
                )))
            }
        }
        // Ok(())
    } else {
        eprintln!("Couldn't find flow with id {}", id);
        Err(FlowError::new(format!("Couldn't find flow with id {}", id)))
    }
}

pub async fn delete_flow(client: &NodeRedHttpClient, id: &str) -> Result<(), FlowError> {
    let flows = convert_flows_response_to_flows(get_all_flows(client).await.unwrap());
    let mut flows = flows.flows;

    if let Some(_flow) = flows.get_mut(id) {
        // delete flow in Node-RED asynchronously
        let request = client
            .client
            .delete(client.path_to_url(format!("/flow/{id}", id = id).as_str()));
        match request.send().await {
            Ok(response) => match response.status() {
                reqwest::StatusCode::NO_CONTENT => {
                    println!("Successfully deleted flow with id {}", id);
                }
                _ => {
                    eprintln!(
                        "Couldn't delete flow with id {}: [{}] {:?}",
                        id,
                        response.status().as_str(),
                        response
                            .json::<serde_json::Value>()
                            .await
                            .unwrap_or("Couldn't deserialize response".into())
                    );
                    return Err(FlowError::new(format!(
                        "Couldn't delete flow with id {}",
                        id
                    )));
                }
            },
            Err(err) => {
                eprintln!(
                    "Couldn't delete flow with id {}: {}\n{:?}",
                    id,
                    err.source().unwrap(),
                    err
                );
                return Err(FlowError::new(format!(
                    "Couldn't delete flow with id {}",
                    id
                )));
            }
        }
        Ok(())
    } else {
        eprintln!("Couldn't find flow with id {}", id);
        Err(FlowError::new(format!("Couldn't find flow with id {}", id)))
    }
}

fn lookup_node_red_base_url_by_area_name(
    config: &Config,
    area_name: &str,
) -> Result<String, FlowError> {
    if let Some(areas) = config.areas.as_ref() {
        for area in areas {
            if area.name == area_name {
                return Ok(format!(
                    "http://{}:{}",
                    area.node_red_ip.clone(),
                    area.node_red_port
                ));
            }
        }
    }
    Err(FlowError::new(format!(
        "Couldn't find Node-RED base URL for area {}",
        area_name
    )))
}

fn lookup_proxy_info_by_area_name(
    config: &Config,
    area_name: &str,
) -> Result<(String, u16, u16), FlowError> {
    if let Some(areas) = config.areas.as_ref() {
        for area in areas {
            if area.name == area_name {
                return Ok((
                    area.proxy_ip.clone(),
                    area.proxy_port_base,
                    area.proxy_webserver_port,
                ));
            }
        }
    }
    Err(FlowError::new(format!(
        "Couldn't find proxy base URL for area {}",
        area_name
    )))
}

pub async fn transfer_flow_to_area(
    flows:  &mut Flows,
    config: &Config,
    client: &NodeRedHttpClient,
    flow_id: &str,
    new_area: &str,
) -> Result<Option<String>, FlowError> {
    let flows = &mut flows.flows;

    if let Some(flow) = flows.get_mut(flow_id) {
        if let Ok((proxy_ip, proxy_port_base, _proxy_webserver_port)) =
            lookup_proxy_info_by_area_name(config, new_area)
        {
            if flow.disabled {
                eprintln!(
                    "Flow with id {} is disabled. Please enable it first.",
                    flow_id
                );
                return Err(FlowError::new(format!(
                    "Flow with id {} is disabled. Please enable it first.",
                    flow_id
                )));
            }

            println!("old flow: {}", serde_json::to_string(&flow).unwrap());

            // Node-RED will replace the flow id with a new one, even if an ID was provided: https://nodered.org/docs/api/admin/methods/post/flow/
            // it will also automatically update all `z` properties of the nodes to the new flow id

            let original_flow_name = flow.name.clone().unwrap_or("".to_string());
            flow.name = to_transferred_flow_name(&original_flow_name, new_area);

            // rewrite flow metadata to new area
            flow.nodes
                .iter_mut()
                .filter(|node| {
                    node._type == "template"
                        && node.field.is_some()
                        && node.field.clone().unwrap() == "meta"
                })
                .for_each(|node| {
                    let mut parsed_template =
                        serde_json::from_str::<serde_json::Value>(&node.template.clone().unwrap())
                            .unwrap();
                    parsed_template["execution_area"] =
                        serde_json::Value::String(new_area.to_string());
                    node.template = Some(parsed_template.to_string());
                });

            // generate a new random id for each node (duplicate ids are not allowed and will result in an error)
            // a id should look something like this: `eb76079d81f6ce58`
            let mut new_node_id_by_old_node_id = HashMap::<String, String>::new();

            flow.nodes.iter_mut().for_each(|node| {
                // generate a uuid and take the first 16 hex characters (excluding the `-`)
                let new_node_id = uuid::Uuid::new_v4().simple().to_string()[..16].to_string();
                new_node_id_by_old_node_id.insert(node.id.clone(), new_node_id.clone());
                node.id = new_node_id;
            });

            // println!("new_node_id_by_old_node_id: {:?}", new_node_id_by_old_node_id);

            // update wires
            flow.nodes.iter_mut().for_each(|node| {
                if let Some(wires) = &mut node.wires {
                    node.wires = wires
                        .iter()
                        .map(|wire| {
                            wire.iter()
                                .map(|node_id| {
                                    // println!("node_id: {}", node_id);
                                    if let Some(new_node_id) =
                                        new_node_id_by_old_node_id.get(node_id)
                                    {
                                        // println!("new_node_id: {}", new_node_id);
                                        new_node_id.clone()
                                    } else {
                                        // println!("node id unchanged");
                                        node_id.clone()
                                    }
                                })
                                .collect::<Vec<String>>()
                        })
                        .collect::<Vec<Vec<String>>>()
                        .into()
                }
            });

            // update sockets to new base port and proxy IP
            // nodes to update: udp in, udp out
            flow.nodes
                .iter_mut()
                .for_each(|node| match node._type.as_str() {
                    "udp in" => {
                        if let Some(port) = &mut node.port {
                            *port = format!(
                                "{}",
                                proxy_port_base + (port.parse::<u32>().unwrap() % 10000) as u16
                            );
                        }
                    }
                    "udp out" => {
                        if let Some(port) = &mut node.port {
                            *port = format!(
                                "{}",
                                proxy_port_base + (port.parse::<u32>().unwrap() % 10000) as u16
                            );
                        }
                        if let Some(outport) = &mut node.outport {
                            *outport = format!(
                                "{}",
                                proxy_port_base + (outport.parse::<u32>().unwrap() % 10000) as u16
                            );
                        }
                        if let Some(addr) = &mut node.addr {
                            *addr = proxy_ip.clone();
                        }
                    }
                    _ => {}
                });

            println!("updated flow: {}", serde_json::to_string(&flow).unwrap());

            let target_area_node_red_base_url =
                lookup_node_red_base_url_by_area_name(config, new_area).unwrap();

            // push changes to Node-RED asynchronously
            let request = client
                .client
                .post(
                    client
                        .path_to_url_with_base_url("/flow", target_area_node_red_base_url.as_str()),
                )
                .json(&flow);
            match request.send().await {
                Ok(response) => {
                    match response.status() {
                        reqwest::StatusCode::OK => {
                            println!("Successfully created flow in new area");

                            // flow successfully created in new area, now update the input node's target
                            if config.input_nodes.is_some() {
                                //TODO find node based on Node-RED input port, mapped to input node port in config?
                                if let Some(input_node) = config
                                    .input_nodes
                                    .as_ref()
                                    .unwrap()
                                    .iter()
                                    .find(|input_node| input_node.flow == original_flow_name)
                                {
                                    // send udp message to input node to update target
                                    let socket =
                                        tokio::net::UdpSocket::bind("0.0.0.0:33000").await.unwrap();
                                    let ip_vec: Vec<u8> = input_node
                                        .ip
                                        .split('.')
                                        .map(|x| x.parse::<u8>().unwrap())
                                        .collect();
                                    let input_node_address = SocketAddr::from((
                                        [ip_vec[0], ip_vec[1], ip_vec[2], ip_vec[3]],
                                        input_node.port,
                                    ));
                                    // socket.connect(input_node_address).unwrap();
                                    // socket.set_read_timeout(Some(Duration::from_millis(10000))).unwrap();
                                    let json = serde_json::json!({
                                        "type": "updateTarget",
                                        "target": proxy_ip,
                                        "target_port_base": proxy_port_base
                                    });
                                    socket
                                        .send_to(json.to_string().as_bytes(), input_node_address)
                                        .await
                                        .unwrap();

                                    // wait for ACK to arrive before continuing
                                    let mut buffer = [0; 1024];
                                    let timeout_duration = 1000;
                                    if (timeout(
                                        Duration::from_millis(timeout_duration),
                                        socket.recv_from(&mut buffer),
                                    )
                                    .await)
                                        .is_err()
                                    {
                                        eprintln!(
                                            "No ACK received from input node within {}ms",
                                            timeout_duration
                                        );
                                        return Err(FlowError::new("ACK from input node timed out. Flow still running in old area.".to_string()));
                                    }

                                    println!("ACK received, disabling flow in old area");
                                    update_flow_status(client, flow_id, true).await?;
                                } else {
                                    eprintln!(
                                        "Couldn't find input node for flow {}",
                                        original_flow_name
                                    );
                                    return Err(FlowError::new(format!(
                                        "Couldn't find input node for flow {}",
                                        original_flow_name
                                    )));
                                }
                            }
                        }
                        _ => {
                            eprintln!(
                                "Couldn't create flow in new area: [{}] {:?}",
                                response.status().as_str(),
                                response
                                    .json::<serde_json::Value>()
                                    .await
                                    .unwrap_or("Couldn't deserialize response".into())
                            );
                            return Err(FlowError::new(
                                "Couldn't create flow in new area".to_string(),
                            ));
                        }
                    }

                }
                Err(err) => {
                    eprintln!(
                        "Couldn't update status of flow with id {}: {:?}",
                        flow_id, err
                    );
                    return Err(FlowError::new(format!(
                        "Couldn't update status of flow with id {}",
                        flow_id
                    )))
                }
            }

            Ok(flow.name.clone())

        } else {
            eprintln!("Couldn't find proxy IP for area {}", new_area);

            Err(FlowError::new(format!(
                "Couldn't find proxy IP for area {}",
                new_area
            )))
        }
    } else {
        eprintln!("Couldn't find flow with id {}", flow_id);
        Err(FlowError::new(format!(
            "Couldn't find flow with id {}",
            flow_id
        )))
    }
}

pub async fn untransfer_flow_from_area(
    config: &Config,
    client: &NodeRedHttpClient,
    flow_id: &str,
    untransfer_from_area: &str,
) -> Result<Option<String>, FlowError> {
    let flows = convert_flows_response_to_flows(get_all_flows(client).await.unwrap());
    let mut flows = flows.flows;

    if let Some(flow) = flows.get_mut(flow_id) {
        if let Ok((proxy_ip, _proxy_port_base, proxy_webserver_port)) =
            lookup_proxy_info_by_area_name(config, untransfer_from_area)
        {
            let original_flow_name = flow.name.clone().unwrap_or("".to_string());
            let transferred_flow_name =
                to_transferred_flow_name(&original_flow_name, untransfer_from_area);

            let target_area_proxy_base_url =
                format!("http://{}:{}", proxy_ip, proxy_webserver_port);

            // enable flow in old area
            match update_flow_status(client, flow_id, false).await {
                Ok(_) => {
                    if config.input_nodes.is_some() {
                        //TODO find node based on Node-RED input port, mapped to input node port in config?
                        if let Some(input_node) = config
                            .input_nodes
                            .as_ref()
                            .unwrap()
                            .iter()
                            .find(|input_node| input_node.flow == original_flow_name)
                        {
                            // send udp message to input node to update target
                            let socket =
                                tokio::net::UdpSocket::bind("0.0.0.0:33000").await.unwrap();
                            let ip_vec: Vec<u8> = input_node
                                .ip
                                .split('.')
                                .map(|x| x.parse::<u8>().unwrap())
                                .collect();
                            let input_node_address = SocketAddr::from((
                                [ip_vec[0], ip_vec[1], ip_vec[2], ip_vec[3]],
                                input_node.port,
                            ));
                            // socket.connect(input_node_address).unwrap();
                            // socket.set_read_timeout(Some(Duration::from_millis(10000))).unwrap();
                            let json = serde_json::json!({
                                "type": "updateTarget",
                                "target": local_ip().unwrap().to_string(),
                                "target_port_base": config.port_base
                            });
                            socket
                                .send_to(json.to_string().as_bytes(), input_node_address)
                                .await
                                .unwrap();

                            // wait for ACK to arrive before continuing
                            let mut buffer = [0; 1024];
                            let timeout_duration = 1000;
                            if (timeout(
                                Duration::from_millis(timeout_duration),
                                socket.recv_from(&mut buffer),
                            )
                            .await)
                                .is_err()
                            {
                                eprintln!(
                                    "No ACK received from input node within {}ms",
                                    timeout_duration
                                );
                                return Err(FlowError::new("ACK from input node timed out. Flow still running in old area.".to_string()));
                            }

                            println!(
                                "ACK received, deleting flow from area '{}'",
                                untransfer_from_area
                            );

                            // delete flow from other area
                            let request = client
                                .client
                                .delete(client.path_to_url_with_base_url(
                                    format!("/flow/{}", transferred_flow_name.unwrap()).as_str(),
                                    target_area_proxy_base_url.as_str(),
                                ))
                                .json(&flow);
                            match request.send().await {
                                Ok(response) => match response.status() {
                                    reqwest::StatusCode::OK => {
                                        println!(
                                            "Successfully deleted flow from area '{}'",
                                            untransfer_from_area
                                        );
                                    }
                                    _ => {
                                        eprintln!(
                                            "Couldn't delete flow from area '{}': [{}] {:?}",
                                            untransfer_from_area,
                                            response.status().as_str(),
                                            response
                                                .json::<serde_json::Value>()
                                                .await
                                                .unwrap_or("Couldn't deserialize response".into())
                                        );
                                        return Err(FlowError::new(format!(
                                            "Couldn't delete flow from area '{}'",
                                            untransfer_from_area
                                        )));
                                    }
                                },
                                Err(err) => {
                                    eprintln!(
                                        "Couldn't delete flow from area '{}': {:?}",
                                        untransfer_from_area, err
                                    );
                                    return Err(FlowError::new(format!(
                                        "Couldn't delete flow from area '{}'",
                                        untransfer_from_area
                                    )));
                                }
                            }
                        } else {
                            eprintln!("Couldn't find input node for flow {}", original_flow_name);
                            return Err(FlowError::new(format!(
                                "Couldn't find input node for flow {}",
                                original_flow_name
                            )));
                        }
                    }

                    Ok(Some(original_flow_name))
                }
                Err(err) => {
                    eprintln!(
                        "Couldn't update status of flow with id {}: {:?}",
                        flow_id, err
                    );
                    Err(FlowError::new(format!(
                        "Couldn't update status of flow with id {}",
                        flow_id
                    )))
                }
            }
            // Ok(())
        } else {
            eprintln!("Couldn't find proxy IP for area {}", untransfer_from_area);

            Err(FlowError::new(format!(
                "Couldn't find proxy IP for area {}",
                untransfer_from_area
            )))
        }
    } else {
        eprintln!("Couldn't find flow with id {}", flow_id);
        Err(FlowError::new(format!(
            "Couldn't find flow with id {}",
            flow_id
        )))
    }
}

pub async fn analyze_flows(
    config: &Config,
    client: &NodeRedHttpClient,
    dry_run: Option<bool>,
) -> Result<AnalysisResult, FlowError> {
    let mut flows = convert_flows_response_to_flows(get_all_flows(client).await.unwrap());

    let mut analyzed_flows = Vec::<String>::new();
    let mut transferrable_flows = Vec::<FlowTransferResult>::new();

    for flow in flows.flows.values() {
        analyzed_flows.push(flow.name.clone().unwrap_or("Unnamed Flow".to_string()));

        if let Some(input_area) = &flow.input_area {
            if let Some(output_area) = &flow.output_area {
                if input_area != "base"
                    && !flow.disabled
                    && input_area == output_area
                    // area specified in config
                    && config
                        .areas
                        .as_ref()
                        .unwrap()
                        .iter()
                        .any(|area| area.name == *input_area)
                {
                    transferrable_flows.push(FlowTransferResult {
                        flow_id: flow.id.clone(),
                        flow_name: flow.name.clone(),
                        new_flow_name: None,
                        old_area: config.area.clone(),
                        new_area: input_area.clone(),
                        transfer_successful: false,
                    });
                }
            }
        }
    }

    if !dry_run.unwrap_or(false) {
        println!("Transferring flows...");

        for flow_transfer in transferrable_flows.iter_mut() {
            println!(
                "Transferring flow '{}' from area '{}' to area '{}'",
                flow_transfer.flow_name.clone().unwrap_or("Unnamed Flow".to_string()),
                flow_transfer.old_area,
                flow_transfer.new_area
            );
            match transfer_flow_to_area(&mut flows, config, client, &flow_transfer.flow_id, flow_transfer.new_area.as_str()).await {
                Ok(new_name) => {
                    flow_transfer.transfer_successful = true;
                    flow_transfer.new_flow_name = new_name;
                },
                Err(err) => {
                    eprintln!("Couldn't transfer flow: {}", err);
                }
            }
        };
        
    }

    Ok(AnalysisResult {
        analyzed_flows: analyzed_flows.into_iter().sorted().collect(),
        transferred_flows: transferrable_flows
            .into_iter()
            .sorted_by(|a, b| a.flow_name.cmp(&b.flow_name))
            .collect(),
    })
}

// more or less the reverse of `analyze_flows()`
pub async fn untransfer_all_flows(
    config: &Config,
    client: &NodeRedHttpClient,
    dry_run: Option<bool>,
) -> Result<AnalysisResult, FlowError> {
    let flows = convert_flows_response_to_flows(get_all_flows(client).await.unwrap());

    let mut analyzed_flows = Vec::<String>::new();
    let mut transferrable_flows = Vec::<FlowTransferResult>::new();

    for flow in flows.flows.values() {
        analyzed_flows.push(flow.name.clone().unwrap_or("Unnamed Flow".to_string()));

        if let Some(input_area) = &flow.input_area {
            if let Some(output_area) = &flow.output_area {
                if input_area != "base"
                    // && flow.disabled
                    && input_area == output_area
                    && !flow.name.clone().unwrap_or_default().ends_with("(transferred)")
                    // area specified in config
                    && config
                        .areas
                        .as_ref()
                        .unwrap()
                        .iter()
                        .any(|area| area.name == *input_area)
                {
                    transferrable_flows.push(FlowTransferResult {
                        flow_id: flow.id.clone(),
                        flow_name: flow.name.clone(),
                        new_flow_name: None,
                        old_area: input_area.clone(),
                        new_area: "base".to_string(),
                        transfer_successful: false,
                    });
                }
            }
        }
    }

    if !dry_run.unwrap_or(false) {
        println!("Untransferring flows...");

        for flow_transfer in transferrable_flows.iter_mut() {
            println!(
                "Untransferring flow '{}' from area '{}' to area '{}'",
                flow_transfer.flow_name.clone().unwrap_or("Unnamed Flow".to_string()),
                flow_transfer.old_area,
                flow_transfer.new_area
            );
            match untransfer_flow_from_area(config, client, &flow_transfer.flow_id, flow_transfer.old_area.as_str()).await {
                Ok(new_name) => {
                    flow_transfer.transfer_successful = true;
                    flow_transfer.new_flow_name = new_name;
                },
                Err(err) => {
                    eprintln!("Couldn't untransfer flow: {}", err);
                }
            }
        };
        
    }

    Ok(AnalysisResult {
        analyzed_flows: analyzed_flows.into_iter().sorted().collect(),
        transferred_flows: transferrable_flows
            .into_iter()
            .sorted_by(|a, b| a.flow_name.cmp(&b.flow_name))
            .collect(),
    })
}

pub fn to_transferred_flow_name(original_flow_name: &str, _new_area: &str) -> Option<String> {
    Some(format!("{} (transferred)", original_flow_name))
}
