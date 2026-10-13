use std::io::Result;

use axum::Router;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let app = Router::new();
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    let address = listener.local_addr()?;
    tracing::info!(%address, "Server listening");
    axum::serve(listener, app).await?;
    Ok(())
}
