use clap::{Arg, Command};
use reqwest::blocking::Client;
use std::collections::HashMap;
use std::error::Error;
use std::env;
use serde::{Deserialize,Serialize};
use futures::future::join_all;
//use serde_json::json;

#[derive(Serialize)]
struct JsonRpcRequest {
  jsonrpc: String,
  method: String,
  params: Vec<String>,
  id: u32,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
  // id: u32,
  // jsoonrpc: String,
  result: String,
//#[serde(default)]
 // error: Option<JsonRpcError>,
}

// #[derive(Debug, Deserialize)]
// struct JsonRpcError {
//   code: i32,
//   message: String,
// }

struct Network {
  chain_id: u64,
  name: String,
  rpc_url: String,
}

fn collect_rpc_urls() -> HashMap<u64, Network> {
  let mut networks = HashMap::new();

  for (key, value) in env::vars() {
    if key.starts_with("RPC_URL_"){
      if let Some(chain_id_str) = key.strip_prefix("RPC_URL_"){
        if let Ok(chain_id) = chain_id_str.parse::<u64>(){
          let name = match chain_id {
            1 => "Ethereum Mainnet",
            8453 => "Base",
            _ => "Uknown Network"
          };

          networks.insert(chain_id, Network{
            chain_id,
            name: name.to_string(),
            rpc_url: value,
          })
        }
      }
    }
  }
}

fn main() -> Result<(), Box<dyn Error>> {
    let matches = Command::new("wallet-fetch")
        .version("0.0.1")
        .author("Steve Simkins")
        .about("Neofetch but for your wallet")
        .arg(
            Arg::new("address")
                .help("Address to fetch info for ")
                .index(1)
                .required(false)
        )
        .get_matches();

    let address = match matches.get_one::<String>("address"){
      Some(addr) if !addr.is_empty() => addr.to_string(),
      _ => {
        match env::var("WALLET_FETCH_ADDRESS") {
          Ok(addr) if !addr.is_empty() => addr,
          _ => {
            eprintln!("Error: No Ethereum address provided. Either pass it as an argument or set the WALLET_FETCH_ADDRESS environment variable");
            return Err("No Ethereum address provided".into());
          }
        }
      }
    };

    let rpc_url = match env::var("RPC_URL"){
      Ok(url) => url,
      Err(_) => {
        eprintln!("RPC URL not defined");
        return Err("RPC_URL environment variable not set".into());
      }
    };

    let client = Client::new();

    let request_data = JsonRpcRequest {
      jsonrpc: "2.0".to_string(),
      method:"eth_getBalance".to_string(),
      params: vec![address.to_string(), "latest".to_string()],
      id: 1,
    };


    let response = match client.post(&rpc_url)
      .json(&request_data)
      .send() {
        Ok(resp) => resp,
        Err(e) => {
          eprintln!("Error sending request: {}", e);
          return Err(e.into());
        }
      };

    if !response.status().is_success() {
      eprintln!("Error: HTTP status {}", response.status());
      eprintln!("Error: {}", response.text()?);
      return Err(format!("Http error").into());
    }

    let response_text = response.text()?;

   let response_body: JsonRpcResponse = match
    serde_json::from_str(&response_text){
      Ok(body) => body,
      Err(e) => {
        eprintln!("Error parsing response: {}", e);
        return Err(e.into());
      }
    };

   if let Some(hex_str) = response_body.result.strip_prefix("0x"){
     if let Ok(balance) = u128::from_str_radix(hex_str, 16){
       let eth_balance = balance as f64 / 1_000_000_000_000_000_000.0;
       println!("Balance in ETH: {:.4}", eth_balance);
     }
   }

    Ok(())
}
