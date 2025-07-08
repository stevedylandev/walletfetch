use clap::{Arg, Command};
use reqwest::Client;
use std::collections::HashMap;
use std::error::Error;
use serde::{Deserialize,Serialize};
use futures::future::join_all;
use tokio::task::JoinHandle;
use toml;
use dirs;
use indicatif::{ProgressBar, ProgressStyle};
use colored::*;
use tiny_keccak::{Hasher, Keccak};
use unicode_normalization::UnicodeNormalization;
use hex;

#[derive(Serialize)]
struct JsonRpcRequest {
  jsonrpc: String,
  method: String,
  params: Vec<String>,
  id: u32,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
  result: String,
}


#[derive(Deserialize)]
struct Config {
  address: Option<String>,
  networks: Option<HashMap<String, NetworkConfig>>,
}

#[derive(Deserialize)]
struct TokenConfig {
  address: String,
  decimals: u8,
}

#[derive(Deserialize)]
struct NetworkConfig {
  name: String,
  rpc_url: String,
  tokens: Option<HashMap<String, TokenConfig>>,
}

#[derive(Clone)]
struct TokenInfo {
  symbol: String,
  address: String,
  decimals: u8,
}

#[derive(Clone)]
struct Network {
  chain_id: u64,
  name: String,
  rpc_url: String,
  tokens: Vec<TokenInfo>,
}

#[derive(Clone)]
struct TokenBalance {
  network_name: String,
  symbol: String,
  balance: f64,
}

enum BalanceResult {
  Native(f64, String),
  Token(TokenBalance),
}

fn keccak256(data: &[u8]) -> [u8; 32] {
  let mut hasher = Keccak::v256();
  let mut output = [0u8; 32];
  hasher.update(data);
  hasher.finalize(&mut output);
  output
}

fn normalize_name(name: &str) -> String {
  name.nfkc().collect::<String>().to_lowercase()
}

fn namehash(name: &str) -> [u8; 32] {
  let normalized = normalize_name(name);

  let mut node = [0u8; 32];

  if normalized.is_empty(){
    return node;
  }

  let labels: Vec<&str> = normalized.split('.').collect();

  for label in labels.iter().rev() {
    let label_hash = keccak256(label.as_bytes());

    let mut combined = [0u8; 64];
    combined[0..32].copy_from_slice(&node);
    combined[32..64].copy_from_slice(&label_hash);

    node = keccak256(&combined);
  }
  node
}

fn read_config() -> Result<Config, Box<dyn Error>> {
  let home_dir = dirs::home_dir().ok_or("Could not find home directory")?;

  let config_path = home_dir.join(".config").join("walletfetch").join("config.toml");

  if !config_path.exists(){
    return Err(format!("Config file not found at {}", config_path.display()).into());
  }

  let config_content = std::fs::read_to_string(config_path)?;
  let config: Config = toml::from_str(&config_content)?;

  Ok(config)
}

fn collect_rpc_urls(config: &Config) -> HashMap<u64, Network> {
  let mut networks = HashMap::new();

  if let Some(network_configs) = &config.networks {
    for (chain_id_str, network_config) in network_configs {
      if let Ok(chain_id) = chain_id_str.parse::<u64>(){
        let mut tokens = Vec::new();
        if let Some(token_configs) = &network_config.tokens {
          for (symbol, token_config) in token_configs {
            tokens.push(TokenInfo {
              symbol: symbol.clone(),
              address: token_config.address.clone(),
              decimals: token_config.decimals,
            });
          }
        }

        networks.insert(chain_id, Network{
          chain_id,
          name: network_config.name.clone(),
          rpc_url: network_config.rpc_url.clone(),
          tokens,
        });
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

async fn fetch_token_balance(
  client: &Client,
  address: &str,
  token: &TokenInfo,
  network: &Network,
) -> Result<TokenBalance, Box<dyn Error + Send + Sync>> {
  let clean_address = address.strip_prefix("0x").unwrap_or(address).to_lowercase();
  let data = format!("0x70a08231000000000000000000000000{}", clean_address);

  let request_data = serde_json::json!({
    "jsonrpc": "2.0",
    "method": "eth_call",
    "params": [
      {
        "to": token.address,
        "data": data
      },
      "latest"
    ],
    "id": 1
  });

  let response = client.post(&network.rpc_url)
    .json(&request_data)
    .send()
    .await?;

  let status = response.status();

  if !status.is_success(){
    let error_text = response.text().await?;
    return Err(format!("HTTP error {}: {}", status, error_text).into());
  }

  let response_text = response.text().await?;

  let response_body: JsonRpcResponse = serde_json::from_str(&response_text)?;

  if let Some(hex_str) = response_body.result.strip_prefix("0x") {
    if let Ok(raw_balance) = u128::from_str_radix(hex_str, 16){
      let divisor = 10_u128.pow(token.decimals as u32) as f64;
      let balance = raw_balance as f64 / divisor;

      return Ok(TokenBalance {
        network_name: network.name.clone(),
        symbol: token.symbol.clone(),
        balance,
      });
    }
  }

  Err(format!("Failed to parse balance for token {} on network {}", token.symbol, network.name).into())
}

fn get_eth_logo() -> &'static str {
r#"    --------------4%--------------
    -------------44HH-------------
    ------------444HHH------------
    -----------4444HHHH-----------
    ---------~44444HHHHH~---------
    --------4444444HHHHHHW--------
    -------4444HHHHWWWWHHHH-------
    ------KHHHHHHHHWWWWWWWWW------
    ---------HHHHHHWWWWWW---------
    -------44---HHHWWW---HH-------
    --------~444?----4HHH~--------
    ----------44444HHHHH----------
    -----------L444HHHq-----------
    -------------44HH-------------
    --------------4H--------------"#
}

async fn fetch_all_balances(
  address: &str,
  networks: HashMap<u64, Network>
) -> Result<Vec<BalanceResult>, Box<dyn Error>> {
  let client = Client::new();

  let mut tasks: Vec<JoinHandle<Result<BalanceResult, Box<dyn Error + Send + Sync>>>> = Vec::new();

  let spinner = ProgressBar::new_spinner();
  spinner.set_style(
      ProgressStyle::default_spinner()
          .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
          .template("{spinner:.green} {msg}")
          .unwrap()
  );
  spinner.set_message("Fetching balances...");

  for (_, network) in &networks {
    let client_clone = client.clone();
    let address_clone = address.to_string();
    let network_clone = network.clone();

    let task = tokio::spawn(async move {
      let (_, balance, name) = fetch_balance(&client_clone, &address_clone, &network_clone).await?;
      Ok(BalanceResult::Native(balance, name))
    });

    tasks.push(task);

    for token in &network.tokens {
      let client_clone = client.clone();
      let address_clone = address.to_string();
      let token_clone = token.clone();
      let network_clone = network.clone();

      let task = tokio::spawn(async move {
        let token_balance = fetch_token_balance(&client_clone, &address_clone, &token_clone, &network_clone).await?;
        Ok(BalanceResult::Token(token_balance))
      });

      tasks.push(task);
    }
  }

  let results = join_all(tasks).await;

  spinner.finish_and_clear();

  let mut balances = Vec::new();
  for result in results {
    match result {
      Ok(Ok(balance_result)) => {
        balances.push(balance_result);
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

async fn get_resolver_address(
  client: &Client,
  rpc_url: &str,
  namehash: [u8; 32]
) -> Result<String, Box<dyn Error>>{
  let registry_address = "0x00000000000C2E074eC69A0dFb2997BA6C7d2e1e";

  let namehash_hex = format!("0x{}", hex::encode(namehash));

  let data = format!("0x0178b8bf{}", &namehash_hex[2..]);

  let request_data = serde_json::json!({
    "jsonrpc": "2.0",
    "method": "eth_call",
    "params": [
      {
        "to": registry_address,
        "data": data
      },
      "latest"
    ],
    "id": 1
  });

  let response = client.post(rpc_url)
    .json(&request_data)
    .send()
    .await?;

  let status = response.status();

  if !status.is_success(){
    let error_text = response.text().await?;
    return Err(format!("HTTP error {}: {}", status, error_text).into());
  }

  let response_text = response.text().await?;
  let response_body: JsonRpcResponse = serde_json::from_str(&response_text)?;

  if response_body.result == "0x0000000000000000000000000000000000000000" {
    return Err("No resolver found for this name".into());
  }

  let address = format!("0x{}", &response_body.result[26..]);

  Ok(address)
}

async fn resolve_ens_address(
  client: &Client,
  rpc_url: &str,
  resolver_address: &str,
  namehash: [u8; 32]
) -> Result<String, Box<dyn Error>>{

  let namehash_hex = format!("0x{}", hex::encode(namehash));

  println!("Namehash hex: {}", namehash_hex);

  let data = format!("0x3b3b57de{}", &namehash_hex[2..]);

  let request_data = serde_json::json!({
    "jsonrpc": "2.0",
    "method": "eth_call",
    "params": [
      {
        "to": resolver_address,
        "data": data
      },
      "latest"
    ],
    "id": 1
  });

  let response = client.post(rpc_url)
    .json(&request_data)
    .send()
    .await?;

  let status = response.status();

  if !status.is_success(){
    let error_text = response.text().await?;
    return Err(format!("HTTP error {}: {}", status, error_text).into());
  }

  let response_text = response.text().await?;
  let response_body: JsonRpcResponse = serde_json::from_str(&response_text)?;

  if response_body.result == "0x0000000000000000000000000000000000000000" {
    return Err("No address found for this name".into());
  }

  let address = format!("0x{}", &response_body.result[26..]);

  Ok(address)
}

async fn resolve_address_or_ens(
  client: &Client,
  input: &str,
  networks: &HashMap<u64, Network>
) -> Result<String, Box<dyn Error>> {
  if input.to_lowercase().ends_with(".eth"){
    let mainnet = networks.get(&1);

    if let Some(network) = mainnet {
      let namehash = namehash(input);

      let resolver_address = get_resolver_address(client, &network.rpc_url, namehash).await?;

      println!("Resolver address: {}", resolver_address);

      let eth_address = resolve_ens_address(client, &network.rpc_url, &resolver_address, namehash).await?;

      return Ok(eth_address);
    } else {
      return Err("Ethereum mainnet configuration not found for ENS resolution".into());
    }
  } else {
    if !input.starts_with("0x") || input.len() != 42 {
      return Err("Invalid Ethereum address format".into());
    }

    Ok(input.to_string())
  }
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

    let config = read_config()?;

    let input = match matches.get_one::<String>("address"){
      Some(addr) if !addr.is_empty() => addr.to_string(),
      _ => match &config.address {
        Some(addr) if !addr.is_empty() => addr.clone(),
        _ => {
          eprintln!("Error: No address provided. Either pass it as an argument or set it in the config file");
          return Err("No address provided".into());
        }
      },
    };

    let networks = collect_rpc_urls(&config);

    if networks.is_empty(){
      eprintln!("RPC URLs are not defined");
    }

    let client = Client::new();

    let address = resolve_address_or_ens(&client, &input, &networks).await?;

    let balances = fetch_all_balances(&address, networks).await?;

    if balances.is_empty(){
      println!("No balances found for address {}", address);
    } else {
      let mut network_balances: HashMap<String, Vec<String>> = HashMap::new();

      for balance in balances {
        match balance {
          BalanceResult::Native(eth_balance, network_name) => {
            let balance_str = format!("{:.4} ETH", eth_balance);
            network_balances.entry(network_name).or_default().push(balance_str);
          },
          BalanceResult::Token(token_balance) => {
            let balance_str = format!("{:.4} {}", token_balance.balance, token_balance.symbol);
            network_balances.entry(token_balance.network_name).or_default().push(balance_str);
          }
        }
      }

      let logo = get_eth_logo();
      let logo_lines: Vec<&str> = logo.lines().collect();
      let logo_height = logo_lines.len();

      println!();
      let address_display = format!("{}", address.bright_green());
      println!("{}", format!("Wallet: {}", address_display).bright_cyan());
      println!("{}", "=".repeat(50).bright_blue());

      let mut info_lines = Vec::new();

      for (network, balances) in network_balances {
        if !info_lines.is_empty() {
          info_lines.push("".to_string());
        }
        info_lines.push(format!("{}: ", network.bright_yellow()));
        for balance in balances {
          info_lines.push(format!("  {} {}", "•".bright_green(), balance.bright_white()));
        }
      }

      let display_lines = std::cmp::max(logo_height, info_lines.len());

      for i in 0..display_lines {
        let logo_line = if i < logo_height { logo_lines[i] } else { "" };
        let info_line = if i < info_lines.len() { &info_lines[i] } else { "" };

        println!("{}    {}", logo_line.bright_cyan(), info_line);
      }

      println!();
    }

    Ok(())
}
