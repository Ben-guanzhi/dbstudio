use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dbstudio_mcp=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    dbstudio_mcp::server::run_stdio().await
}
