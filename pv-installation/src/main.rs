use eyre::{eyre, Context};

mod pv_simulator_pebc;
mod pv_simulator_simple;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt().init();

    let connection = s2energy::websockets_json::connect_as_client(
        std::env::var("CEM_URL")
            .wrap_err("Could not read CEM URL from environment variable CEM_URL")?,
    )
    .await?;

    let control_type = std::env::var("CONTROL_TYPE")
        .wrap_err("Could not read control type from environment variable CONTROL_TYPE")?;
    
    match control_type.as_str() {
        "PEBC" => pv_simulator_pebc::start_mock(connection).await?,
        "NOT_CONTROLABLE" => pv_simulator_simple::start_mock(connection).await?,
        other => {
            return Err(eyre!(
                "Invalid value for CONTROL TYPE ({other}); should PEBC or NOT_CONTROLABLE"
            ));
        }
    }

    Ok(())
}
