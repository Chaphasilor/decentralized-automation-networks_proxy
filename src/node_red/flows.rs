use std::collections::{HashMap};
use serde::{Serialize, Deserialize, ser::SerializeStruct};
use serde_with::skip_serializing_none;
use std::{error::Error, fmt};

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
    onceDelay: Option<f32>,
    topic: Option<String>,
    payload: Option<String>,
    payloadType: Option<String>,
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
    fieldType: Option<String>,
    template: Option<String>,
    iface: Option<String>,
    port: Option<String>,
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

impl Serialize for Flow {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer
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
        FlowError {
            message,
        }
    }
}

pub static NODE_RED_BASE_URL: &'static str = "http://localhost:1880";

pub async fn get_all_flows(client: &reqwest::Client) -> Result<FlowsResponse, Box<dyn std::error::Error>> {

  let request = client.get(format!("{NODE_RED_BASE_URL}/flows"))
    .header("Node-RED-API-Version", "v2")
    .send();
  
  match request.await {
      Ok(response) => {
          println!("status: {}", response.status());
          match response.json::<FlowsResponse>().await {
            Ok(flows_response) => {
                return Ok(flows_response);
            },
            Err(err) => {
                eprintln!("Couldn't deserialize flows response: {}\n{:?}", err.source().unwrap(), err);
                return Err(Box::new(err));
            }
          }
      },
      Err(err) => {
          eprintln!("Couldn't get flows from Node-RED: {}\n{:?}", err.source().unwrap(), err);
          return Err(Box::new(err));
      }
  }

}

pub fn convert_flows_response_to_flows(response: FlowsResponse) -> Flows {
    let mut flows = Flows{
        flows: HashMap::<String, Flow>::new(),
        rev: response.rev,
    };
    
    response.flows[..].iter().filter(|flow_node| flow_node._type == "tab").for_each(|flow_node| {
        let new_flow = Flow{
            // nodes: vec![flow_node.clone()],
            nodes: vec![],
            id: flow_node.id.to_string(),
            name: flow_node.label.clone(),
            disabled: flow_node.disabled.unwrap_or(false),
            configs: if flow_node.configs.is_some() {flow_node.configs.clone().unwrap()} else {vec![]},
            input_area: None,
            output_area: None,
        };
        flows.flows.insert(new_flow.id.to_string(), new_flow);
    });
    
    for flow_node in response.flows[..].iter().filter(|flow_node| flow_node._type != "tab").collect::<Vec<&FlowNodeResponse>>() {
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
    flows.flows.get(id).map(|flow| flow.clone())
}

pub fn get_flow_id_by_name(flows: &Flows, name: &str) -> Option<String> {
    flows.flows.iter().find(|(_, flow)| {
        if let Some(flow_name) = &flow.name {
            flow_name == name
        } else {
            false
        }
    }).map(|(id, _)| id.to_string())
}

pub async fn update_flow_status(client: &reqwest::Client, id: &str, is_disabled: bool) -> Result<(), FlowError> {

    let flows = convert_flows_response_to_flows(get_all_flows(client).await.unwrap());
    let mut flows = flows.flows;
    
    if let Some(flow) = flows.get_mut(id) {
        // // disable each node in the flow
        // flow.nodes.iter_mut().for_each(|node| {
        //     node.disabled = Some(true);
        // });
        flow.disabled = is_disabled;

        // println!("updated flow: {}", serde_json::to_string(&flow).unwrap());

        // push changes to Node-RED asynchronously
        let client = reqwest::Client::new();
        let request = client.put(format!("{NODE_RED_BASE_URL}/flow/{id}", id=id))
            .header("Node-RED-API-Version", "v2")
            .json(&flow);
        match request.send().await {
            Ok(response) => {
                return Ok(())
            },
            Err(err) => {
                eprintln!("Couldn't update status of flow with id {}: {}\n{:?}", id, err.source().unwrap(), err);
                return Err(FlowError::new(format!("Couldn't update status of flow with id {}", id)));
            }
        }
        // Ok(())
        
    } else {
        eprintln!("Couldn't find flow with id {}", id);
        Err(FlowError::new(format!("Couldn't find flow with id {}", id)))
    }
}


