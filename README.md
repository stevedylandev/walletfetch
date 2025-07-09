# Walletfetch

![cover](https://files.stevedylan.dev/walletfetch.png)

Like neofetch but for your wallet

## About

I used this small project as a way to get more experience with Rust, so if things look weird, that's why lol. Overall it was a lot of fun, and I enjoyed using AI as a learning resource only and writing the code myself (check out (agents.md)[/agents.md]).

## Installation

For the time being you can install `walletfetch` by building from source, which is actually pretty easy. Just follow the steps below:

1. Make sure you have [Rust installed](https://www.rust-lang.org/tools/install)

```bash
cargo --version
```

2. Clone the repo and `cd` into it

```bash
git clone https://github.com/stevedylandev/walletfetch
cd walletfetch
```

3. Install locally

```bash
cargo install --path .
```

## Usage

To start, run the `walletfetch` command followed by an ENS or address

```bash
walletfetch vitalik.eth
```

This will create a default config file at `~/.config/walletfetch/config.toml`. Inside that config file you can configure RPC URLs for different chains as well as any tokens you want to include. The format for main balances is as follows: `networks.chain_id`, followed by the `name` and `rpc_url`.

```toml
[networks.1]
name = "Mainnet"
rpc_url = "https://eth.drpc.org"
```

For tokens, follow the same pattern as above but add `.tokens` to the header, then fill in the details such as the name, `address` and `decimals`

```toml
[networks.1.tokens]
USDC = { address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", decimals = 6 }
```

You can also designate a default `address` at the top so you can just run `walletfetch` without any arguments. Check out the example config below:

```toml
address = "stevedylandev.eth"

[networks.1]
name = "Mainnet"
rpc_url = "https://eth.drpc.org"

[networks.1.tokens]
USDC = { address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", decimals = 6 }


[networks.8453]
name = "Base"
rpc_url = "https://base.drpc.org"

[networks.8453.tokens]
USDC = { address = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", decimals = 6 }
lemon3 = { address = "0xe0907762b1d9cdfbe8061ae0cc4a0501fa077421", decimals = 18 }

[networks.42161]
name = "Arbitrum"
rpc_url = "https://arbitrum.drpc.org"

[networks.42161.tokens]
USDC = { address = "0xaf88d065e77c8cC2239327C5EDb3A432268e5831", decimals = 6 }
```

## Questions

Feel free to [reach out](https://stevedylan.dev/links)!
