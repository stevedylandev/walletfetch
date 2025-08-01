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
use alloy::providers::{ProviderBuilder};
use alloy_ccip_read::CCIPReader;

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

fn read_config() -> Result<Config, Box<dyn Error>> {
  let home_dir = dirs::home_dir().ok_or("Could not find home directory")?;
  let config_dir = home_dir.join(".config").join("walletfetch");
  let config_path = config_dir.join("config.toml");

  if !config_path.exists(){

    std::fs::create_dir_all(&config_dir)?;

    let default_config = r#"# WalletFetch Configuration
# You can set a default address here (optional)
# address = "0x..."

[networks.1]
name = "Mainnet"
rpc_url = "https://eth.drpc.org"

[networks.1.tokens]
USDC = { address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", decimals = 6 }

[networks.42161]
name = "Arbitrum"
rpc_url = "https://arbitrum.drpc.org"

[networks.42161.tokens]
USDC = { address = "0xaf88d065e77c8cC2239327C5EDb3A432268e5831", decimals = 6 }

[networks.8453]
name = "Base"
rpc_url = "https://base.drpc.org"

[networks.8453.tokens]
USDC = { address = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", decimals = 6 }
"#;

    std::fs::write(&config_path, default_config)?;

    println!("Created default config at: {}", config_path.display());
    println!("You can edit this file to customize your networks and tokens.");
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

fn format_balance_smart(balance: f64, symbol: &str) -> String {
    let formatted = if balance >= 1_000_000.0 {
        format!("{:.2}M", balance / 1_000_000.0)
    } else if balance >= 1_000.0 {
        format!("{:.2}K", balance / 1_000.0)
    } else if balance >= 1.0 {
        format!("{:.2}", balance)
    } else if balance > 0.0 {
        format!("{:.6}", balance).trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        "0".to_string()
    };

    format!("{} {}", formatted, symbol)
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
r#"------------------------------
--------------4%--------------
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
--------------4H--------------
------------------------------"#
}

async fn fetch_all_balances(
  address: &str,
  networks: HashMap<u64, Network>
) -> Result<Vec<BalanceResult>, Box<dyn Error>> {
  let client = Client::new();

  let mut tasks: Vec<JoinHandle<Result<BalanceResult, Box<dyn Error + Send + Sync>>>> = Vec::new();

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

async fn resolve_address_or_ens(
  input: &str,
  networks: &HashMap<u64, Network>
) -> Result<String, Box<dyn Error>> {
  if input.contains(".") {
    let mainnet = networks.get(&1);

    let rpc_url = match mainnet {
      Some(network) => &network.rpc_url,
      None => return Err("Ethereum mainnet configuration not found for ENS resolution".into())
    };

    let provider = ProviderBuilder::new()
      .on_http(rpc_url.parse().unwrap());

    let reader = CCIPReader::new(provider.boxed());

    let resolution_result = match reader.resolve_name(input).await {
      Ok(result) => result,
      Err(e) => return Err(format!("Failed to resolve address for {}: {}", input, e).into())
    };

    let eth_address = format!("{}", resolution_result.addr.value);

    return Ok(eth_address);
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

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
            .template("{spinner:.green} {msg}")
            .unwrap()
    );
    spinner.set_message("Fetching balances...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

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

    let address = resolve_address_or_ens(&input, &networks).await?;


    let balances = fetch_all_balances(&address, networks).await?;

    spinner.finish_and_clear();

    if balances.is_empty(){
      println!("No balances found for address {}", address);
    } else {
      let mut network_balances: HashMap<String, Vec<String>> = HashMap::new();

      for balance in balances {
        match balance {
          BalanceResult::Native(eth_balance, network_name) => {
            let balance_str = format_balance_smart(eth_balance, "ETH");
            network_balances.entry(network_name).or_default().push(balance_str);
          },
          BalanceResult::Token(token_balance) => {
            let balance_str = format_balance_smart(token_balance.balance, &token_balance.symbol);
            network_balances.entry(token_balance.network_name).or_default().push(balance_str);
          }
        }
      }

      let logo = get_eth_logo();
      let logo_lines: Vec<&str> = logo.lines().collect();
      let logo_height = logo_lines.len();
      let logo_width = logo_lines.iter().map(|line| line.len()).max().unwrap_or(0);


      println!();
      if input.contains("."){
        let ens_display = format!("{}", input.bright_green());
        println!("{}", format!("ENS: {}", ens_display).bright_cyan());
      }
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
        let logo_line = if i < logo_height {
          logo_lines[i]
        } else {
          &" ".repeat(logo_width)
        };
        let info_line = if i < info_lines.len() { &info_lines[i] } else { "" };

        println!("{}    {}", logo_line.bright_cyan(), info_line);
      }

      println!();
    }

    Ok(())
}
