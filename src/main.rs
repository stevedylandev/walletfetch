use clap::{Arg, Command};
use reqwest::Client;
use std::collections::HashMap;
use std::error::Error;
use std::env;
use serde::{Deserialize,Serialize};
use futures::future::join_all;
use tokio::task::JoinHandle;

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
}


// #[derive(Deserialize)]
// struct Config {
//   address: Option<String>,
//   networks: Option<HashMap<String, NetworkConfig>>,
// }

// #[derive(Deserialize)]
// struct NetworkConfig {
//   name: String,
//   rpc_url: String,
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
            chain_id: chain_id,
            name: name.to_string(),
            rpc_url: value,
          });
        };
      }
    }
  }

  networks
}

async fn fetch_balance(
  client: &Client,
  address: &str,
  network: &Network
) -> Result<(u64, f64, String), Box<dyn Error + Send + Sync >> {
  let request_data = JsonRpcRequest {
    jsonrpc: "2.0".to_string(),
    method: "eth_getBalance".to_string(),
    params: vec![address.to_string(), "latest".to_string()],
    id: 1,
  };

  let response = client.post(&network.rpc_url)
    .json(&request_data)
    .send()
    .await?;

  let status = response.status();

  if !response.status().is_success() {
    let error_text = response.text().await?;
    return Err(format!("HTTP error {}: {}", status, error_text).into());
  }

  let response_text = response.text().await?;
  let response_body: JsonRpcResponse = serde_json::from_str(&response_text)?;

  if let Some(hex_str) = response_body.result.strip_prefix("0x"){
    if let Ok(balance) = u128::from_str_radix(hex_str, 16){
      let eth_balance = balance as f64 / 1_000_000_000_000_000_000.0;
      return Ok((network.chain_id, eth_balance, network.name.clone()));
    }
  }
  Err(format!("Failed to parse balance for network {}", network.name).into())
}

async fn fetch_all_balances(
  address: &str,
  networks: HashMap<u64, Network>
) -> Result<Vec<(u64, f64, String)>, Box<dyn Error>> {
  let client = Client::new();

  let mut tasks: Vec<JoinHandle<Result<(u64, f64, String), Box<dyn Error + Send + Sync>>>> = Vec::new();

  for (_, network) in networks {
    let client_clone = client.clone();
    let address_clone = address.to_string();
    let network_clone = network;

    let task = tokio::spawn(async move {
      fetch_balance(&client_clone, &address_clone, &network_clone).await
    });

    tasks.push(task);
  }

  let results = join_all(tasks).await;

  let mut balances = Vec::new();
  for result in results {
    match result {
      Ok(Ok((chain_id, balance, name))) => {
        balances.push((chain_id, balance, name));
      },
      Ok(Err(e)) => {
        eprintln!("Error fetching balance: {}", e);
      },
      Err(e) => {
        eprintln!("Task error: {}", e);
      }
    }
  }
  Ok(balances)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
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

    let networks = collect_rpc_urls();

    if networks.is_empty(){
      eprintln!("RPC URLs are not defined");
    }

    let balances = fetch_all_balances(&address, networks).await?;

    if balances.is_empty(){
      println!("No balances found for address {}", address);
    } else {
      println!("Balances for {}", address);
      println!("------------------------");
      for (_, balance, name) in balances {
        println!("{}: {:.4} ETH", name, balance);
      }
    }

    Ok(())
}
