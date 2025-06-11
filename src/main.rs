use axum::{
    extract::{Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::Response,
    routing::{get, post},
    Router,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use tracing::{debug, error, info, warn};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port to listen on
    #[arg(short, long)]
    port: u16,

    /// Destination URL to forward requests to
    #[arg(short, long)]
    dest: String,
}

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    destination: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    params: Option<Value>,
    id: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let args = Args::parse();

    info!("Starting proxy server on port {} forwarding to {}", args.port, args.dest);

    let state = AppState {
        client: reqwest::Client::new(),
        destination: args.dest,
    };

    let app = Router::new()
        .route("/", post(handle_post_request))
        .route("/", get(handle_get_request))
        .fallback(handle_fallback)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", args.port)).await?;
    info!("Proxy server listening on {}", listener.local_addr()?);

    axum::serve(listener, app).await?;

    Ok(())
}

async fn handle_post_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response<String>, StatusCode> {
    info!("Received POST request, body length: {}", body.len());
    debug!("Request body: {}", body);
    debug!("Request headers:");
    for (name, value) in &headers {
        debug!("  {}: {:?}", name.as_str(), value);
    }

    // Try to parse as JSON-RPC request
    match serde_json::from_str::<JsonRpcRequest>(&body) {
        Ok(mut rpc_request) => {
            info!("Parsed JSON-RPC request: method={}", rpc_request.method);

            // Handle special cases
            match rpc_request.method.as_str() {
                "eth_getTransactionCount" => {
                    info!("Overriding eth_getTransactionCount with 0x0");
                    let response = JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        result: Some(json!("0x0")),
                        error: None,
                        id: rpc_request.id,
                    };
                    let response_body = serde_json::to_string(&response)
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

                    debug!("eth_getTransactionCount response body: {}", response_body);

                    return Ok(Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(response_body)
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
                }
                "eth_call" => {
                    info!("Normalizing eth_call parameters");
                    if let Some(params) = &mut rpc_request.params {
                        if let Some(params_array) = params.as_array_mut() {
                            if let Some(first_param) = params_array.get_mut(0) {
                                if let Some(obj) = first_param.as_object_mut() {
                                    // If both "input" and "data" exist, remove "input"
                                    if obj.contains_key("input") && obj.contains_key("data") {
                                        obj.remove("input");
                                        info!("Removed 'input' field (keeping 'data')");
                                    }
                                    // If only "input" exists, rename to "data"
                                    else if let Some(input_value) = obj.remove("input") {
                                        obj.insert("data".to_string(), input_value);
                                        info!("Renamed 'input' field to 'data'");
                                    }

                                    // Remove chainId field as TRON API doesn't support it
                                    if obj.remove("chainId").is_some() {
                                        info!("Removed 'chainId' field for TRON API compatibility");
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }

            // Forward the (possibly modified) request
            let modified_body = serde_json::to_string(&rpc_request)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

            debug!("Modified request body being sent to destination: {}", modified_body);

            forward_request(&state, Method::POST, &headers, &modified_body, &rpc_request.method).await
        }
        Err(_) => {
            // Not a valid JSON-RPC request, forward as-is
            info!("Not a JSON-RPC request, forwarding as-is");
            forward_request(&state, Method::POST, &headers, &body, "unknown").await
        }
    }
}

async fn handle_get_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    query: Query<HashMap<String, String>>,
) -> Result<Response<String>, StatusCode> {
    info!("Received GET request with {} query parameters", query.len());

    // Build query string
    let query_string = if query.is_empty() {
        String::new()
    } else {
        format!("?{}",
            query.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("&")
        )
    };

    forward_get_request(&state, &headers, &query_string).await
}

async fn handle_fallback(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response<String>, StatusCode> {
    info!("Received fallback request");
    forward_get_request(&state, &headers, "").await
}

async fn forward_request(
    state: &AppState,
    method: Method,
    headers: &HeaderMap,
    body: &str,
    rpc_method: &str,
) -> Result<Response<String>, StatusCode> {
    let url = &state.destination;

    info!("Forwarding {} request to {}", method, url);

    let mut request_builder = match method {
        Method::POST => state.client.post(url),
        Method::GET => state.client.get(url),
        _ => return Err(StatusCode::METHOD_NOT_ALLOWED),
    };

    // Copy relevant headers (excluding problematic ones)
    for (name, value) in headers {
        let header_name_str = name.as_str();

        // Skip headers that might cause issues with Tron API
        if header_name_str.eq_ignore_ascii_case("content-length") {
            debug!("Skipping problematic header: {}", header_name_str);
            continue;
        }

        if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()) {
            if let Ok(header_value) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
                debug!("Forwarding header: {} = {:?}", header_name_str, header_value);
                request_builder = request_builder.header(header_name, header_value);
            }
        }
    }

    if method == Method::POST {
        request_builder = request_builder.body(body.to_string());
    }

    match request_builder.send().await {
        Ok(response) => {
            let status = response.status();
            let response_headers = response.headers().clone();

            match response.text().await {
                Ok(mut response_body) => {
                    info!("Received response from destination, status: {}, body length: {}",
                          status, response_body.len());

                    // Log the actual response content for debugging
                    debug!("Raw response body: {}", response_body);

                    // Log response headers for debugging
                    debug!("Response headers from destination:");
                    for (name, value) in &response_headers {
                        debug!("  {}: {:?}", name.as_str(), value);
                    }

                    // Apply block response enhancement for specific methods
                    let original_length = response_body.len();
                    if matches!(rpc_method, "eth_getBlockByNumber" | "eth_getBlockByHash") {
                        response_body = enhance_block_response(&response_body, rpc_method);
                    }
                    let modified_length = response_body.len();

                    // Log the final response being sent to client
                    debug!("Final response body being sent to client: {}", response_body);

                    let mut response_builder = Response::builder().status(status.as_u16());

                    // Copy response headers, but update Content-Length if response was modified
                    debug!("Copying response headers to client:");
                    for (name, value) in response_headers {
                        if let Some(name) = name {
                            // Skip Content-Length if we modified the response body
                            if name.as_str().eq_ignore_ascii_case("content-length") && original_length != modified_length {
                                debug!("  Skipping original Content-Length header due to response modification");
                                continue;
                            }

                            if let Ok(header_value) = axum::http::HeaderValue::from_bytes(value.as_bytes()) {
                                debug!("  Copying header: {} = {:?}", name.as_str(), header_value);
                                response_builder = response_builder.header(name.as_str(), header_value);
                            } else {
                                warn!("  Failed to convert header value for {}: {:?}", name.as_str(), value);
                            }
                        }
                    }

                    // Set correct Content-Length if response was modified
                    if original_length != modified_length {
                        debug!("  Setting new Content-Length: {} (was {})", modified_length, original_length);
                        response_builder = response_builder.header("content-length", modified_length.to_string());
                    }

                    response_builder
                        .body(response_body)
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
                }
                Err(e) => {
                    error!("Failed to read response body: {}", e);
                    Err(StatusCode::BAD_GATEWAY)
                }
            }
        }
        Err(e) => {
            error!("Failed to forward request: {}", e);
            Err(StatusCode::BAD_GATEWAY)
        }
    }
}

async fn forward_get_request(
    state: &AppState,
    headers: &HeaderMap,
    query_string: &str,
) -> Result<Response<String>, StatusCode> {
    // For GET requests, we need to modify the destination URL to include query parameters
    let url = format!("{}{}", state.destination, query_string);

    info!("Forwarding GET request to {}", url);

    let mut request_builder = state.client.get(&url);

    // Copy relevant headers
    for (name, value) in headers {
        if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()) {
            if let Ok(header_value) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
                request_builder = request_builder.header(header_name, header_value);
            }
        }
    }

    match request_builder.send().await {
        Ok(response) => {
            let status = response.status();
            let response_headers = response.headers().clone();

            match response.text().await {
                Ok(response_body) => {
                    info!("Received GET response from destination, status: {}, body length: {}",
                          status, response_body.len());

                    let mut response_builder = Response::builder().status(status.as_u16());

                    // Copy response headers
                    for (name, value) in response_headers {
                        if let Some(name) = name {
                            if let Ok(header_value) = axum::http::HeaderValue::from_bytes(value.as_bytes()) {
                                response_builder = response_builder.header(name.as_str(), header_value);
                            }
                        }
                    }

                    response_builder
                        .body(response_body)
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
                }
                Err(e) => {
                    error!("Failed to read GET response body: {}", e);
                    Err(StatusCode::BAD_GATEWAY)
                }
            }
        }
        Err(e) => {
            error!("Failed to forward GET request: {}", e);
            Err(StatusCode::BAD_GATEWAY)
        }
    }
}

fn enhance_block_response(response_body: &str, method: &str) -> String {
    match serde_json::from_str::<JsonRpcResponse>(response_body) {
        Ok(mut rpc_response) => {
            if let Some(result) = &mut rpc_response.result {
                if let Some(block) = result.as_object_mut() {
                    let mut modified = false;

                    // Check if stateRoot is missing or invalid
                    let needs_state_root_fix = match block.get("stateRoot") {
                        None => {
                            info!("Adding missing stateRoot to {} response", method);
                            true
                        }
                        Some(state_root) => {
                            if let Some(state_root_str) = state_root.as_str() {
                                // Check if stateRoot is invalid (empty "0x" or not 66 characters)
                                if state_root_str == "0x" || state_root_str.len() != 66 {
                                    info!("Fixing invalid stateRoot '{}' in {} response", state_root_str, method);
                                    true
                                } else {
                                    false
                                }
                            } else {
                                info!("Fixing non-string stateRoot in {} response", method);
                                true
                            }
                        }
                    };

                    if needs_state_root_fix {
                        block.insert(
                            "stateRoot".to_string(),
                            json!("0x0101010101010101010101010101010101010101010101010101010101010101")
                        );
                        modified = true;
                    }

                    // Return the modified response if any changes were made
                    if modified {
                        if let Ok(modified_response) = serde_json::to_string(&rpc_response) {
                            return modified_response;
                        }
                    }
                }
            }
        }
        Err(e) => {
            warn!("Failed to parse response as JSON-RPC for block enhancement: {}", e);
        }
    }

    // Return original response if no modification was needed or possible
    response_body.to_string()
}
