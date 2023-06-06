use std::collections::{HashMap};
use serde::{Serialize, Deserialize};
use std::{error::Error, fmt};

#[derive(Deserialize, Debug, Clone)]
pub struct FlowNodeResponse {
    id: String,
    #[serde(rename = "type")]
    _type: String,
    label: Option<String>,
    disabled: Option<bool>,
    info: Option<String>,
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
}
#[derive(Deserialize, Debug)]
pub struct FlowsResponse {
    flows: Vec<FlowNodeResponse>,
    rev: String,
}
#[derive(Debug)]
pub struct Flow {
    nodes: Vec<FlowNodeResponse>,
    id: String,
    input_area: Option<String>,
    output_area: Option<String>,
}
#[derive(Debug)]
pub struct Flows {
    flows: HashMap<String, Flow>,
    rev: String,
}

pub static NODE_RED_BASE_URL: &'static str = "http://localhost:1880";

pub fn get_all_flows() -> Result<FlowsResponse, Box<dyn std::error::Error>> {

  let client = reqwest::blocking::Client::new();
  let request = client.get(format!("{NODE_RED_BASE_URL}/flows"))
    .header("Node-RED-API-Version", "v2")
    .send();
  
  match request {
      Ok(response) => {
          println!("status: {}", response.status());
          match response.json::<FlowsResponse>() {
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
            nodes: vec![flow_node.clone()],
            id: flow_node.id.to_string(),
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
