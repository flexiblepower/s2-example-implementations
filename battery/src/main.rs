use eyre::{Context, eyre};
use tracing_subscriber::EnvFilter;
use tungstenite::client::IntoClientRequest;

mod battery_simulator;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // For KIFLIN: set an authorization header.
    let auth_token = std::env::var("AUTH_TOKEN")
        .wrap_err("Could not read authorization token from environment variable AUTH_TOKEN")?;
    let cem_url = std::env::var("CEM_URL")
        .wrap_err("Could not read CEM URL from environment variable CEM_URL")?;
    let mut request = cem_url.into_client_request()?;
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {}", auth_token).parse()?);

    let connection = s2energy::websockets_json::connect_as_client(request).await?;

    let control_type = std::env::var("CONTROL_TYPE")
        .wrap_err("Could not read control type from environment variable CONTROL_TYPE")?;

    match control_type.as_str() {
        "FRBC" => battery_simulator::start_mock(connection).await?,
        other => {
            return Err(eyre!(
                "Invalid value for CONTROL TYPE ({other}); should FRBC"
            ));
        }
    }

    Ok(())
}
