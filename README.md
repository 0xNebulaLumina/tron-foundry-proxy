# Tron Foundry Proxy

A Rust HTTP proxy server designed for Ethereum JSON-RPC requests with specific override rules for Tron Foundry compatibility.

## Features

- **HTTP Proxy Server**: High-performance HTTP proxy built with axum framework
- **JSON-RPC 2.0 Processing**: Intelligent parsing and processing of Ethereum JSON-RPC requests
- **Method-Specific Override Rules**:
  1. **eth_getTransactionCount**: Returns consistent "0x0" nonce value
  2. **eth_call**: Parameter normalization (input/data fields, chainId removal)
  3. **eth_estimateGas**: Full Foundry compatibility with address conversion and parameter normalization
  4. **Block Response Enhancement**: Fixes invalid/missing stateRoot in block responses
- **Request/Response Processing**:
  - Automatic parameter normalization for TRON API compatibility
  - Response enhancement for Ethereum client compatibility
  - Proper JSON-RPC 2.0 compliance (no null error fields)
  - Content-Length header correction for modified responses
- **Comprehensive Logging**: Multi-level structured logging with tracing framework
- **Header Management**: Intelligent header forwarding and filtering
- **Error Handling**: Graceful handling of malformed requests and network errors
- **CLI Interface**: Simple command-line configuration with clap

## Installation

### Prerequisites
- Rust 1.70+ (with Cargo)

### Build from Source
```bash
git clone <repository-url>
cd tron-foundry-proxy
cargo build --release
```

## Usage

### Basic Usage
```bash
./target/release/tron-foundry-proxy --port 8545 --dest https://api.trongrid.io/jsonrpc
```

### Command Line Arguments
- `--port <PORT>` or `-p <PORT>`: Port number to listen on (required)
- `--dest <DEST>` or `-d <DEST>`: Destination URL to forward requests to (required)

### Example
```bash
# Start proxy on port 8080, forwarding to local json-rpc-enabled Tron node
./target/release/tron-foundry-proxy --port 8080 --dest http://localhost:8545

# Start proxy on port 8545, forwarding to remote node
# https://api.trongrid.io/jsonrpc for tron mainnet
# https://api.shasta.trongrid.io/jsonrpc for tron shasta testnet
./target/release/tron-foundry-proxy --port 8545 --dest https://api.trongrid.io/jsonrpc
```

### Foundry Integration

The proxy is specifically designed to work with Foundry tools for Tron development. Once the proxy is running, you can use standard Foundry commands:

#### Contract Deployment
```bash
# Deploy contract using Foundry (gas estimation works automatically)
forge create src/Counter.sol:Counter \
  --rpc-url http://localhost:8545/ \
  --private-key 0x8f7dc3d0f5961df9c5ee2fcb59886b87262afad6a00a335aa2f384a74b24c14d \
  --legacy

# Deploy with manual gas limit (if preferred)
forge create src/Counter.sol:Counter \
  --rpc-url http://localhost:8545/ \
  --private-key 0x8f7dc3d0f5961df9c5ee2fcb59886b87262afad6a00a335aa2f384a74b24c14d \
  --legacy \
  --gas-limit 50000000
```

#### Other Foundry Commands
```bash
# Cast commands work through the proxy
cast block latest --rpc-url http://localhost:8545/
cast call <contract_address> "balanceOf(address)" <address> --rpc-url http://localhost:8545/
```

**Note**: The proxy automatically handles the differences between Ethereum and Tron APIs, including address format conversion and parameter normalization, making Foundry tools work seamlessly with Tron networks.

## Request Processing

The proxy implements intelligent JSON-RPC request/response processing with method-specific override rules and response enhancements to ensure compatibility between Ethereum tooling and TRON blockchain APIs.

### JSON-RPC Request Flow

1. **Request Parsing**: All incoming POST requests are parsed as JSON-RPC 2.0 requests
2. **Method Detection**: The proxy identifies the RPC method and applies appropriate processing rules
3. **Parameter Normalization**: Method-specific parameter transformations are applied
4. **Request Forwarding**: Modified requests are forwarded to the destination TRON API
5. **Response Enhancement**: Responses are processed to ensure Ethereum client compatibility
6. **JSON-RPC Compliance**: Final responses conform to JSON-RPC 2.0 specification

### Override Rules

#### 1. eth_getTransactionCount Override
**Purpose**: Provides consistent nonce value for Ethereum tooling compatibility

**Behavior**:
- **Input**: Any `eth_getTransactionCount` JSON-RPC request
- **Processing**: Request is NOT forwarded to destination
- **Output**: Returns `{"jsonrpc": "2.0", "result": "0x0", "id": <request_id>}` immediately
- **Use Case**: Prevents nonce-related issues in Ethereum development tools

**Example**:
```json
// Request
{"jsonrpc": "2.0", "method": "eth_getTransactionCount", "params": ["0x123...", "latest"], "id": 1}

// Response (immediate, not forwarded)
{"jsonrpc": "2.0", "result": "0x0", "id": 1}
```

#### 2. eth_call Parameter Normalization
**Purpose**: Ensures TRON API compatibility by normalizing transaction call parameters

**Parameter Processing**:
- **input/data field handling**:
  - If both "input" and "data" exist: Removes "input", keeps "data"
  - If only "input" exists: Renames "input" to "data"
- **chainId removal**: Removes "chainId" field as TRON API doesn't support it
- **Forwarding**: Modified request is then forwarded to destination

**Example**:
```json
// Original Request
{
  "jsonrpc": "2.0",
  "method": "eth_call",
  "params": [{
    "to": "0x123...",
    "input": "0xabcd...",
    "chainId": "0x1"
  }, "latest"],
  "id": 1
}

// Normalized Request (sent to TRON API)
{
  "jsonrpc": "2.0",
  "method": "eth_call",
  "params": [{
    "to": "0x123...",
    "data": "0xabcd..."
  }, "latest"],
  "id": 1
}
```

#### 3. eth_estimateGas Foundry Compatibility
**Purpose**: Enables Foundry's `forge create` command to work without `--gas-limit` by properly handling gas estimation requests

**Key Features**:
- **Ethereum to Tron Address Conversion**: Converts Ethereum addresses to Tron format by adding `0x41` prefix
- **Parameter Normalization**: Similar to eth_call (input→data, removes chainId/gas/gasPrice)
- **Contract Creation Support**: Handles `"to": null` for contract deployment transactions
- **Parameter Count Fix**: Removes extra parameters that Foundry sends (e.g., "pending")

**Address Conversion Process**:
- Ethereum address: `0x8f7dc3d0f5961df9c5ee2fcb59886b87262afad6a`
- Tron address: `0x418f7dc3d0f5961df9c5ee2fcb59886b87262afad6a` (adds `41` prefix)

**Parameter Processing**:
- **Address conversion**: Converts `from` and `to` addresses to Tron format
- **Null handling**: Preserves `"to": null` for contract creation
- **Field normalization**: `input` → `data`, removes `chainId`, `gas`, `gasPrice`
- **Parameter truncation**: Removes extra parameters beyond the transaction object

**Example**:
```json
// Foundry Request (2 parameters)
{
  "jsonrpc": "2.0",
  "method": "eth_estimateGas",
  "params": [{
    "from": "0xe0f150addcce307c1a58767437fe537620a2e34a",
    "to": null,
    "input": "0x608060405...",
    "chainId": "0x1",
    "nonce": "0x0"
  }, "pending"],
  "id": 2
}

// Normalized Request (sent to TRON API)
{
  "jsonrpc": "2.0",
  "method": "eth_estimateGas",
  "params": [{
    "from": "0x41e0f150addcce307c1a58767437fe537620a2e34a",
    "to": null,
    "data": "0x608060405...",
    "nonce": "0x0"
  }],
  "id": 2
}

// Successful Response
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": "0x17854"
}
```

**Foundry Integration**:
This fix enables the following Foundry command to work correctly:
```bash
forge create src/Contract.sol:Contract \
  --rpc-url http://localhost:8545/ \
  --private-key 0x... \
  --legacy
```

#### 4. Block Response Enhancement
**Purpose**: Fixes invalid or missing stateRoot fields in TRON block responses for Ethereum client compatibility

**Processing**:
- **Applies to**: `eth_getBlockByNumber` and `eth_getBlockByHash` responses
- **Detection**: Identifies missing, empty ("0x"), or invalid stateRoot values
- **Enhancement**: Replaces with valid 32-byte hex string
- **Forwarding**: Request is forwarded normally, only response is modified

**Conditions for stateRoot fix**:
- Missing stateRoot field
- Empty stateRoot ("0x")
- Invalid length stateRoot (not 66 characters including "0x")

**Example**:
```json
// TRON API Response (invalid stateRoot)
{
  "jsonrpc": "2.0",
  "result": {
    "number": "0x123",
    "hash": "0xabc...",
    "stateRoot": "0x"
  },
  "id": 1
}

// Enhanced Response (sent to client)
{
  "jsonrpc": "2.0",
  "result": {
    "number": "0x123",
    "hash": "0xabc...",
    "stateRoot": "0x0101010101010101010101010101010101010101010101010101010101010101"
  },
  "id": 1
}
```

### Response Processing Features

#### JSON-RPC 2.0 Compliance
- **Clean responses**: Omits null error fields in successful responses
- **Proper structure**: Maintains correct JSON-RPC 2.0 format
- **Header management**: Updates Content-Length when response body is modified

#### Header Handling
- **Request headers**: Forwards relevant headers while filtering problematic ones
- **Response headers**: Preserves original response headers from TRON API
- **Content-Length**: Automatically recalculated when responses are enhanced

#### Error Handling
- **Malformed requests**: Non-JSON-RPC requests are forwarded as-is
- **Network errors**: Proper HTTP status codes for upstream failures
- **Parsing errors**: Graceful handling of invalid JSON responses

## Logging

The proxy uses structured logging with different levels:
- **INFO**: Request/response flow, server startup
- **WARN**: Non-critical issues (e.g., failed JSON parsing)
- **ERROR**: Critical errors (network failures, etc.)

Set the `RUST_LOG` environment variable to control log levels:
```bash
RUST_LOG=debug ./target/release/tron-foundry-proxy --port 8545 --dest https://api.trongrid.io/jsonrpc
```

## Architecture

- **HTTP Server**: Built with [axum](https://github.com/tokio-rs/axum) for high performance
- **HTTP Client**: Uses [reqwest](https://github.com/seanmonstar/reqwest) for forwarding requests
- **JSON Processing**: [serde_json](https://github.com/serde-rs/json) for JSON-RPC parsing
- **CLI**: [clap](https://github.com/clap-rs/clap) for command-line argument parsing
- **Async Runtime**: [tokio](https://github.com/tokio-rs/tokio) for async operations

## Development

### Running in Development
```bash
cargo run -- --port 8545 --dest https://api.trongrid.io/jsonrpc
```

### Testing
```bash
# Check compilation
cargo check

# Run tests (if any)
cargo test

# Build optimized release
cargo build --release
```

## License

[Add your license information here]
