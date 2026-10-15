use forgeflow::api;

#[tokio::main]
async fn main() {
    let address = std::env::var("FORGEFLOW_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&address)
        .await
        .expect("bind server");
    println!("forgeflow listening on {address}");
    axum::serve(listener, api::router())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("serve requests");
}
