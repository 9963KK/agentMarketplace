use std::net::SocketAddr;

use agent_marketplace::server::{PlatformApp, http};

#[tokio::main]
async fn main() {
    let addr = std::env::var("AGENT_MARKETPLACE_ADDR")
        .or_else(|_| std::env::var("PORT").map(|port| format!("0.0.0.0:{port}")))
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
        .parse::<SocketAddr>()
        .expect("AGENT_MARKETPLACE_ADDR or PORT must define a valid socket address");
    let app = PlatformApp::spawn().expect("platform app should start");

    if let Err(error) = http::serve(app, addr).await {
        eprintln!("platform server failed: {error}");
        std::process::exit(1);
    }
}
