# Decentralized Automation Networks - Proxy Server

[![Release Build (linux-x64)](https://github.com/Chaphasilor/decentralized-automation-networks_proxy/actions/workflows/rust.yml/badge.svg)](https://github.com/Chaphasilor/decentralized-automation-networks_proxy/actions/workflows/rust.yml)

## General Overview

This project contains a Rust-based server that can proxy UDP messages for Node-RED. This way, timestamps for incoming and outgoing messages can be recorded, from which the runtime of automations in Node-RED can be calculated. The whole process works like this:

![Diagram of the message flow](<Message Flow.png>)

The source code of the companion programs can be found at the following locations:

- Virtual Input Node: <https://github.com/Chaphasilor/decentralized-automation-networks_virtual-input-node>  
  *Rust program for sending out dummy messages at a certain interval*
- Virtual Output Node: <https://github.com/Chaphasilor/decentralized-automation-networks_virtual-output-node>  
  *Rust program for receiving and logging dummy messages*
- ESP32 Input Node: <https://github.com/Chaphasilor/decentralized-automation-networks_esp32-input-node>  
  *Rust-based firmware for the ESP32 micro-controller that sends out messages whenever a button is pressed*
- ESP32 Output Node: <https://github.com/Chaphasilor/decentralized-automation-networks_esp32-output-node>  
  *Rust-based firmware for the ESP32 micro-controller that flashes a LED whenever a message is received*

Additionally, there's a Python-based Jupyter notebook used for analyzing and plotting the results:  
<https://github.com/Chaphasilor/decentralized-automation-networks_evaluation>

## Usage

```sh-session
$ cargo run -- --help
A management server and proxy for Node-RED

Usage: decentralized-automation-networks_proxy.exe [OPTIONS] --config <CONFIG>

Options:
  -c, --config <CONFIG>  Path to the configuration file
      --latency-test     Measure the UDP latency to all input and output nodes on startup
  -h, --help             Print help
  -V, --version          Print version
```

The program is made up of three main parts: A UDP proxy that forwards messages to and from Node-RED, a HTTP webserver that can be used to perform certain actions within Node-RED, and a UDP-based ping implementation that is used to test real-world latencies between proxy and input/output nodes.  
To get started, the config file has to be properly set up. For this, the IP address and port of the Node-RED instance have to be specified, as well as the port used for the integrated webserver, the name of the area in which the proxy is running, and the *port base*. This port base is used to route between different proxies running on the same machine, as they cannot use the same ports.  
This is tied to the `ports` section, where the base port for each type of UDP socket is specified (e.g. sockets for messages received from input nodes, messages received from Node-RED, etc.). Sockets are not re-used for multiple functions as this would require a more complex routing configuration. Instead, a *port base* is specified for each type of port, using a number that is divisible by 1000. Any port between this number and the next number divisible by 1000 can then be used for that function.  
To limit the amount of bound UDP sockets, a *port range limit* has to be specified. If set to a number `n`, this will allocate the first `n` ports for the respective function, starting at the respective port base.  
The `areas` section of the config contains a list of any additional areas where a proxy is running. The config options are the same as for the current instance, but additionally include the IP of the proxy server.  
The `input_nodes` and `output_nodes` sections specify a list of all input or output nodes, including the name, area, IP address, configuration port, and automation name. The automation name, called `flow` to stay consistent with Node-RED terminology, specifies which automation (*flow*) the input or output node is connected to. This is important for updating the target IP of input nodes when an automation is transferred to a new area, and also needed to properly log the timestamps for received messages, grouped by the automation.

### HTTP Endpoints

#### `GET /db/save`

Saves the current state of the database to the file system

#### `GET /db/get`

Fetches the current state of the database as a JSON response

#### `GET /db/log`

Pretty-prints the current state of the database to `stdout`

#### `POST /flows/transfer`

```json
{
  "name": "Flow 1",
  "newArea": "room1"
}
```

Transfers the flow with name "Flow 1" from the `base` area to the area `room1`, including updating the target of the related input node and disabling the flow in the base area.

#### `DELETE /flows/transfer`

```json
{
  "name": "Flow 1",
  "area": "room1"
}
```

Un-transfers the flow with name "Flow 1" (original name) from the area `room1` back to the `base` area, including updating the target of the related input node, deleting the flow at area `room1` and,  re-enabling the flow in the base area.

#### `PUT /flows/updateStatus`

```json
{
  "name": "Flow 1",
  "disabled": false
}
```

Updates the status of the flow with the name "Flow 1" at the Node-RED instance managed by this proxy.

#### `DELETE /flow/<Flow Name>`

Deletes the flow with name "\<Flow Name\>" at the Node-RED instance managed by this proxy.

#### `GET /proxy/nodeRedBaseUrl`

Returns the HTTP base URL of the Node-RED instance managed by this proxy.

#### `POST /flows/analyze`

```json
{
  "dry_run": true
}
```
(body optional)

Analyzes all the flows at the Node-RED instance managed by this proxy, and transfers suitable ones to the respective area.  
If `dry_run` is enabled, automations will be analyzed, but not actually transferred. The response will indicate which automations have been analyzed, and which of these are suitable for transfer.

#### `DELETE /flows/analyze`

```json
{
  "dry_run": true
}
```
(body optional)

Analyzes all the flows at the Node-RED instance managed by this proxy, and tries to un-transfer any disabled automations from the respective area.  
This will only work if the automations have not been updated since they were transferred. Consequently, this endpoint should be called in case automations need to be updated, to make sure that nothing breaks.   
If `dry_run` is enabled, automations will be analyzed, but not actually un-transferred. The response will indicate which automations have been analyzed, and which of these are suitable for un-transfer.

## Setting Things Up

MSRV (Minimum Supported Rust Version): 1.69

1. Install Rust (e.g. from <https://rustup.rs>)
2. Clone the Repo:  
   ```sh-session
   git clone https://github.com/Chaphasilor/decentralized-automation-networks_proxy`
   ```
3. Build the project:  
   ```sh-session
   cd decentralized-automation-networks_proxy
   cargo build
   ```
4. Set up the config file:  
   *You can use [the provided demo config](data/base.config.yaml) for this*
5. Run the proxy server:  
   ```sh-session
   cargo run -- --config path/to/config.yaml
   ```
